# Spec: Synchronisation zwischen mehreren Rechnern

Stand: 2026-09-24, Revision 4.1. Revisionen 1–3 wurden von GPT-6 Astra
geprüft; die Zuordnung der Befunde steht im Anhang. Seit Revision 3 ist
Zurückziehen ein endgültiger Grabstein plus neu identifizierte private Kopie;
Freigabe-Revisionen und Lifecycle-Operationen entfallen.

## Ziel

Der Benutzer nutzt die App auf mehreren Rechnern (privater Linux-PC,
Windows-Firmenrechner). Videos, Zusammenfassungen, Chats und Sammlungen sollen
auf allen Rechnern erscheinen. Jeder Rechner bleibt voll offline-fähig: Die
lokale SQLite-Datenbank ist weiterhin die einzige Datenquelle der App; ein
kleiner Sync-Server auf dem eigenen VPS tauscht nur Änderungen aus.

Nicht-Ziele: Synchronisation von Einstellungen (`ai.json`, `agent.json`,
`summary-presets.json`, Websuche, UI-Zustand in `localStorage`), API-Keys
(`auth.json`, **nie**), Ende-zu-Ende-Verschlüsselung, Weboberfläche,
Mehrbenutzerbetrieb, Echtzeit-Push, Ausnahme einzelner Chats vom Sync,
automatische Wiedervereinigung nach einem Server-Restore.

## Grundsätze

1. **Local-first.** Die App liest und schreibt nur lokal. Sync läuft im
   Hintergrund; ohne Netz arbeitet alles wie bisher.
2. **Eigener Sync statt Fremdlösung** (Remote-DB, Dateisync, Litestream,
   cr-sqlite, Turso/PowerSync/Electric verworfen, siehe Revision 1).
3. **Jede Existenz hat eine zufällige, nie wiederverwendete `uid`** — auch
   Videos. Grabsteine sind endgültig und werden auch für unbekannte uids
   gespeichert (Sperre gegen verspätete Anfragen). Die YouTube-ID ist nur
   natürlicher Schlüssel zum Verschmelzen unabhängig angelegter, geteilter
   Kopien. Keine Uhrzeit entscheidet über Existenz.
4. **Geteilt oder privat, ohne Zwischenzustand.** Ein privates Video
   (`local_only = 1`) ist vom Sync vollständig entkoppelt. Wird ein bereits
   veröffentlichtes Video privat geschaltet, bekommt der Server einen
   Grabstein der alten uid mit Grund `withdrawn`, und die lokale Zeile bekommt
   eine neue uid. Jedes andere Gerät, das die alte uid hält, macht daraus
   ebenfalls eine private Kopie mit neuer uid. Erneutes Freigeben einer privaten
   Kopie ist ein normales Neuanlegen, das der Server über die YouTube-ID mit
   einer bestehenden geteilten Existenz verschmilzt.
5. **Der Server ist Schiedsrichter und echot jeden verarbeiteten Schlüssel.**
   Jede Push-Operation gibt allen berührten Schlüsseln eine neue Server-`seq`,
   ob angenommen oder verworfen. Clients überspringen beim Anwenden nur
   Schlüssel mit ausstehender lokaler Änderung; das Echo liefert den Gewinner
   nach dem nächsten Push.
6. **Outbox = Menge schmutziger Schlüssel** mit Eigentümer. Der Push liest
   Inhalte in einem kurzen Snapshot; existiert die Zeile, ist es ein Upsert,
   sonst ein Delete.
7. **Chat-Runden sind die Übertragungseinheit** (unveränderlich, vollständig).
8. **Server im selben Repo**, eigenes Cargo-Projekt `sync-server/`, gemeinsame
   Typen in `sync-proto/`, kein Cargo-Workspace im Repo-Root.

## Begriffe

| Begriff | Bedeutung |
| --- | --- |
| Existenz | Datensatz mit eigener `uid` |
| Grabstein (`gone`) | endgültiges Ende einer uid; Grund `deleted`, `withdrawn` oder `merged` (mit `mergedInto`) |
| Alias | uid X wurde in die Wurzel Y verschmolzen; Operationen auf X wirken auf Y |
| veröffentlicht | `videos.published = 1`: die uid ist in einem Push-Snapshot enthalten gewesen (Anfrage evtl. angekommen) |
| privat | `videos.local_only = 1` |
| Echo | neue Server-`seq` für einen Schlüssel nach Verarbeitung einer Operation |

## Client-Datenmodell

### Neue Spalten und Änderungen

| Tabelle | Änderung |
| --- | --- |
| `videos` | neu `uid TEXT`, `local_only INTEGER NOT NULL DEFAULT 0`, `published INTEGER NOT NULL DEFAULT 0`; **`UNIQUE` auf `video_id` entfällt** (eine private und eine geteilte Kopie desselben YouTube-Videos dürfen nebeneinander bestehen; `add_video` verhindert weiterhin Dubletten über `video_exists`) |
| `summaries`, `chats`, `collections` | neu `uid TEXT` |
| `chat_messages` | neu `round_uid TEXT`, `position INTEGER` |

- uids: `lower(hex(randomblob(16)))`. Jeder INSERT-Pfad setzt sie (auch
  `backfill_legacy_summaries` und Apply). `UNIQUE`-Index je uid-Spalte;
  `BEFORE INSERT … WHEN NEW.uid IS NULL → RAISE(ABORT)`.
- `append_chat_turn`: eine `round_uid` je Aufruf, `position` = Index.
- Alle Zeitstempel lokal kanonisch (`YYYY-MM-DDTHH:MM:SS.mmmZ`), damit
  lokale und übertragene Sortierungen übereinstimmen; Altbestände
  kanonisiert die Migration (Runden vorher gruppiert).
- Global stabile Sortierungen:
  - Chat-Nachrichten `ORDER BY created_at, round_uid, position`.
  - Zusammenfassungen `ORDER BY created_at DESC, uid DESC` überall, wo
    „neueste“ bestimmt wird (`get_summaries`, `delete_summary`, Apply, Chat-
    und Übergabe-Kontext).
- `set_video_collections` schreibt nur echte Differenzen.
- `chats.context_options` bekommt das optionale Feld `pendingSummaryUids`
  (siehe Apply).

### Sync-Tabellen

```sql
CREATE TABLE sync_outbox (
    seq        INTEGER PRIMARY KEY AUTOINCREMENT,  -- monoton, nie wiederverwendet
    entity     TEXT NOT NULL,  -- video|summary|chat|round|collection|membership
    key        TEXT NOT NULL,  -- uid; membership: '<video_uid>/<collection_uid>'
    owner      TEXT,           -- Video-uid (NULL bei collection)
    parent     TEXT,           -- round: Chat-uid; membership: Sammlungs-uid
    tomb       TEXT,           -- nur video: 'deleted' | 'withdrawn'
    changed_at TEXT NOT NULL,  -- kanonischer UTC-Zeitpunkt der letzten lokalen Änderung
    unsendable TEXT,           -- Grund, falls über der Größengrenze (je Version)
    UNIQUE(entity, key)
);
CREATE TABLE sync_state (key TEXT PRIMARY KEY, value TEXT);
  -- cursor, dataset_id, applying, seeded, schema
CREATE TABLE sync_inbox (seq INTEGER PRIMARY KEY, entity TEXT, key TEXT, payload TEXT);
```

Schreiben eines Eintrags: `DELETE` des alten Eintrags gleichen Schlüssels und
`INSERT` (neue `seq` aus AUTOINCREMENT, `unsendable` zurückgesetzt).
Quittung: `DELETE … WHERE seq = <quittierte seq>` — neuere Versionen bleiben.

### Trigger

Alle Trigger schweigen, solange `sync_state.applying = '1'` (nur innerhalb der
Apply-Transaktion gesetzt, vor dem Commit entfernt).

| Auslöser | Bedingung | Outbox |
| --- | --- | --- |
| `videos` INSERT | `local_only = 0` | `video` |
| `videos` UPDATE OF `url, title, thumbnail_url, thumbnail_data, transcript, chapters, published_at, description, transcript_error` | `local_only = 0` | `video` |
| `videos` BEFORE DELETE | – | Einträge mit `owner = uid` löschen; wenn `published = 1`: `video` mit `tomb = 'deleted'` |
| `summaries` INSERT, DELETE | Video existiert, `local_only = 0` | `summary` |
| `chats` INSERT, UPDATE | Video `local_only = 0` | `chat` |
| `chats` BEFORE DELETE | Video existiert, `local_only = 0` | Einträge `round` mit `parent = uid` löschen, dann `chat` |
| `chat_messages` INSERT | Video des Chats `local_only = 0` | `round` (Schlüssel `round_uid`, `parent` = Chat-uid) |
| `collections` INSERT, UPDATE | – | `collection` |
| `collections` BEFORE DELETE | – | Einträge `membership` mit `parent = uid` löschen, dann `collection` |
| `video_collections` INSERT, DELETE | Video und Sammlung existieren, Video `local_only = 0` | `membership` |

- Kaskaden: Bei Video-Löschung existiert das Video in den Kind-Triggern nicht
  mehr, sie schweigen; der Server kaskadiert selbst. Chat-Löschung kaskadiert
  Nachrichten ohne Trigger; offene Runden-Einträge räumt der Chat-Trigger ab.
- `videos.summary*` und `updated_at` lösen nichts aus.

### Privat schalten und freigeben (`video_set_local_only`)

Eine `IMMEDIATE`-Transaktion in Rust (kein Trigger), damit die Schritte
zusammen passieren:

- **Geteilt → privat:** Einträge mit `owner = uid` löschen. Ist `published =
  1`: `video`-Eintrag mit alter uid und `tomb = 'withdrawn'`, dann
  **Neu-Identifizieren des Teilbaums**: neue uid für das Video, alle seine
  Summaries und Chats, eine neue `round_uid` je alter Runde (neue uids
  innerhalb gleicher Zeitstempel ordnungserhaltend vergeben, damit
  Reihenfolge und „neueste Summary“ gleich bleiben); `pendingSummaryUids`
  der Chats über die Zuordnung alt → neu umschreiben; `published = 0`.
  `local_only = 1`. So teilt die private Kopie keinen Schlüssel mehr mit
  geteilten Existenzen (auch nicht mit einer späteren Neuanlage derselben
  Inhalte auf einem anderen Gerät).
- **Privat → geteilt:** `local_only = 0`, Einträge für Video und alle Kinder
  (Summaries, Chats, Runden, Zuordnungen) anlegen. Die Kopie wird beim Push neu
  angelegt bzw. vom Server verschmolzen.

Umschaltzeitpunkt ist der Commit. Ein vorher erstellter Snapshot darf noch
gesendet werden; der folgende Grabstein entfernt die Inhalte vom Server und
sperrt die uid auch gegen verspätete Anfragen.

### Migration

1. Vor der Transaktion `PRAGMA journal_mode = WAL`.
2. Eine `IMMEDIATE`-Transaktion, idempotent: `videos` nach SQLite-Verfahren
   für Tabellenumbau neu anlegen (ohne `UNIQUE(video_id)`, mit neuen Spalten;
   `foreign_keys` dafür vorübergehend aus, `foreign_key_check` vor dem Commit),
   übrige Spalten ergänzen, uids setzen, Runden bilden (eine je `(chat_id,
   created_at)` mit den **ursprünglichen** Zeitstempeln, `position` nach `id`),
   Indizes, Sync-Tabellen, Trigger (versioniert über `sync_state.schema`; bei
   neuer Version `DROP` + `CREATE`), einmalig (`seeded`) Outbox für alle Zeilen
   nicht privater Videos und alle Sammlungen.
   Nach dem Gruppieren der Runden werden **alle** Zeitstempelspalten lokal
   kanonisiert (`strftime('%Y-%m-%dT%H:%M:%fZ', …)`, Verhalten bei
   `+00:00`-Offsets und Nanosekunden per Test belegen); neue Schreibpfade
   schreiben nur noch kanonisch.
3. Abbruch → Rollback; nächster Start wiederholt.
4. Tabellenumbau exakt nach dem SQLite-Verfahren: `PRAGMA foreign_keys = OFF`
   auf der Migrationsverbindung **vor** `BEGIN`, neue Tabelle anlegen, Daten
   mit Integer-ids kopieren, alte Tabelle löschen, neue umbenennen,
   `foreign_key_check`, Commit, danach `foreign_keys = ON`. Nie die alte
   Tabelle zuerst umbenennen.

### Verbindungen und Transaktionen

- `open_db`: `foreign_keys = ON`, `busy_timeout = 5000` je Verbindung.
- Lesen-dann-Schreiben-Transaktionen mit `TransactionBehavior::Immediate`
  (`append_chat_turn`, `delete_summary`, `update_summary`,
  `set_video_collections`, `video_set_local_only`, Snapshot, Quittung, Apply).
  Keine Netzwerk- oder KI-Aufrufe innerhalb.

## Protokoll (`sync-proto`)

`PROTOCOL_VERSION = 1`. Jede Anfrage trägt `X-Sync-Protocol`,
`Authorization: Bearer`; mutierende und lesende Sync-Anfragen zusätzlich
`X-Sync-Dataset` (erwartete Datensatz-ID). JSON `camelCase`. Zeitstempel
kanonisch; `sync-proto` bietet `canonical_time()` und vergleicht nur kanonisch.

### Endpunkte

```text
GET  /v1/health            -> {status, protocol, datasetId}      (ohne Auth, ohne Dataset)
POST /v1/push  {ops:[Op]}  -> {results:[OpResult]}                (gleiche Länge/Reihenfolge)
GET  /v1/pull?since=<seq>  -> {states:[ServerState], next, more}
```

- Der Server prüft `X-Sync-Dataset` **vor** jeder Verarbeitung; Abweichung →
  `409 {"error":"datasetMismatch","datasetId":…}` ohne Wirkung.
- Die Datensatz-ID holt der Client vor dem ersten Push/Pull per `health`
  und behandelt sie nach „Datensatz-Bindung“ (siehe Einstellungen).
  Der Server prüft den Header in derselben Transaktion wie die Operation.

| Antwort | Client |
| --- | --- |
| 400 (Strukturfehler, Index im Text) | nichts quittieren, betroffene Operation `unsendable` markieren, Rest weiter senden |
| 401 | anhalten bis Einstellungsänderung |
| 409 `datasetMismatch`, 409 `cursorAhead` | anhalten, „Neu abgleichen“ anbieten |
| 413 | Block halbieren; Einzeloperation → `unsendable` |
| 426 | „App bzw. Server aktualisieren“, anhalten |
| 5xx, Netz | Backoff bis 10 min, Outbox unverändert |

### Operationen (Client → Server)

Getaggtes Enum; Schlüssel ergeben sich aus dem Inhalt.

| `type` | Felder |
| --- | --- |
| `video` | `uid, changedAt, youtubeId, url, title, thumbnailUrl, thumbnailData (Base64/null), transcript, chapters, publishedAt, description, transcriptError, createdAt` |
| `videoGone` | `uid, reason: deleted \| withdrawn` |
| `summary` | `uid, videoUid, createdAt, summary, provider, model, options` |
| `summaryDelete` | `uid, videoUid` |
| `chat` | `uid, videoUid, changedAt, title, createdAt, updatedAt, contextOptions {transcript, summaryUids: null \| [uid]}` |
| `chatDelete` | `uid, videoUid` |
| `round` | `uid, chatUid, videoUid, createdAt, messages: [{role, content, toolCalls, toolCallId, provider, model}]` |
| `collection` | `uid, changedAt, name, createdAt` |
| `collectionDelete` | `uid` |
| `membership` | `videoUid, collectionUid, present, changedAt` |

Löschungen (`videoGone`, `summaryDelete`, `chatDelete`, `collectionDelete`)
unbekannter uids speichern einen Grabstein und antworten `ok`; sie antworten
nie mit `retry`.

`OpResult = {status: ok | rejected | retry, reason?}`:
- `ok`, `rejected` → Client quittiert (das Echo bringt den kanonischen Zustand).
- `retry` → nicht quittieren; nur wenn ein direktes Elternobjekt dem Server
  **unbekannt** ist (weder lebend noch Grabstein), mit `missing: <entity>/<uid>`.

### Zustände (Server → Client)

`ServerState` mit `seq`: `video {uid, …}`, `videoGone {uid, reason,
mergedInto?}`, `summary`, `summaryGone {uid}`, `chat`, `chatGone {uid}`,
`round`, `collection`, `collectionGone {uid, reason, mergedInto?}`,
`membership {videoUid, collectionUid, present}`.

### Grenzen und Validierung (Konstanten in `sync-proto`)

| Grenze | Wert |
| --- | --- |
| Operationen je Push | 500 |
| Bytes je Push-Body | 32 MiB |
| Bytes je Operation (serialisiert) | 16 MiB (nur Packgrenze des Clients; der Server prüft nur den Body) |
| `thumbnailData` dekodiert | 2 MiB |
| Pull-Seite | ~8 MiB oder 500 Zustände, mindestens einer |

Server prüft: uid `^[0-9a-f]{32}$`, YouTube-ID `^[A-Za-z0-9_-]{11}$`, Rollen,
`toolCalls` gültiges JSON, Base64, Sammlungsname 1–80 Zeichen nach Trim,
kanonische Zeitstempel. Beziehungen: Eltern einer bestehenden Summary, eines
Chats, einer Runde sind unveränderlich (nach Alias-Auflösung); `chatUid` einer
Runde gehört zu `videoUid`; `summaryUids` eines Chats gehören zu dessen Video
(fremde entfernt). Strukturfehler → 400 für den Batch ohne Wirkung;
Beziehungsfehler → `rejected`.

`name_key(name)` = Trim + ASCII-Kleinschreibung (entspricht lokal `NOCASE`).

## Server-Regeln

Ein Push-Batch = eine Transaktion. Alias-uids werden vorab auf ihre Wurzel
abgebildet (Ketten flach gehalten). Tie-Break für LWW: höhere Geräte-ID.

**Echo:** Jede Operation erhöht die `seq` jedes berührten Schlüssels. Bei
Ablehnung wegen eines Elternobjekts ist der berührte Schlüssel dessen
Grabstein (Video, Chat oder Sammlung) — nie ein erfundener Kind-Zustand.

### Videos

| Fall | Ergebnis |
| --- | --- |
| `video`, uid unbekannt, keine lebende Existenz mit dieser YouTube-ID | neue Existenz |
| `video`, uid unbekannt, lebende Existenz A mit dieser YouTube-ID | Alias uid → A, Grabstein `merged` mit `mergedInto A`, Inhalt in A zusammenführen, Teilbaum von A echoen |
| `video`, uid lebt | Inhalt zusammenführen |
| `video` oder Kind, uid mit Grabstein | `rejected`, Echo des Grabsteins |
| `videoGone`, uid lebt | Grabstein mit Grund, Kinder serverseitig löschen (ohne Kind-Grabsteine; Kind-uids dürfen später unter einer neuen Existenz wiederkommen) |
| `videoGone`, uid unbekannt | Grabstein speichern (Sperre gegen verspätetes Anlegen) |
| `videoGone`, uid hat schon Grabstein | Grund bleibt unverändert |

Invariante: höchstens eine lebende Existenz je YouTube-ID auf dem Server.

**Inhalt zusammenführen:** `title, url, thumbnailUrl, publishedAt` nach LWW.
`transcript, chapters, description, thumbnailData`: vorhandener Wert wird nie
durch `null` ersetzt; leeres Feld wird aus jedem Datensatz gefüllt, auch einem
älteren; zwei verschiedene Werte → LWW. Je Feld wird der Zeitstempel des
Feldgewinners gespeichert. `transcriptError = null`, sobald ein Transkript da
ist, sonst LWW.

### Kinder

- `summary`, `round`: unveränderlich; bekannte uid → `ok` ohne Änderung.
  Eltern mit Grabstein → `rejected` + Echo des Eltern-Grabsteins; Eltern
  unbekannt → `retry`.
- `chat`: LWW auf Titel, `contextOptions`, `updatedAt`.
- `summaryDelete`, `chatDelete`: Grabstein (endgültig), `chatDelete` löscht
  Runden serverseitig.
- `membership`: LWW auf `present`; Sammlung oder Video mit Grabstein →
  `rejected` + Echo; unbekannt → `retry`.

### Sammlungen

- Unbekannte uid, Namensschlüssel einer lebenden Sammlung Y → Alias zu Y,
  Grabstein `merged`, Zuordnungen umhängen (Paar-Konflikt: LWW, Gleichstand →
  `present`).
- Bekannte uid: zuerst Namens-LWW gegen den gespeicherten Namen. Nur wenn der
  **gewinnende** Name den Schlüssel einer anderen lebenden Sammlung Y trägt,
  wird verschmolzen (Y bleibt Wurzel). Ein verlierender Rename bewirkt nichts.
- Bei jedem Verschmelzen werden die Wurzel **und alle ihre Zuordnungen**
  geechot (ein Gerät, das die Wurzel wegen einer lokalen Namenskollision
  übersprungen hatte, bekommt so auch deren bestehende Zuordnungen).
- `collectionDelete`: Grabstein, Zuordnungen serverseitig löschen. Operationen
  auf Alias-uids wirken auf die Wurzel.

### Pull

Zustände mit `seq > since` nach `seq`, seitenweise. `next` = höchste `seq` der
Seite, bei leerer Seite `since`. Jeder Schlüssel hat genau einen Zustand;
ändert er sich beim Blättern, erscheint er später erneut.

## Client-Sync-Engine (`src-tauri/src/sync/`)

Ein Lauf ist durch einen Mutex serialisiert; Einstellungsänderung und
„Neu abgleichen“ nehmen denselben Mutex. Reihenfolge: (Datensatz-ID sichern),
Push, Pull, Apply. Die Engine ist in `snapshot`, `send`, `acknowledge`,
`pull_page`, `apply` zerlegt (Tests können Antworten verwerfen).

### Push

1. **Snapshot** (eine kurze `IMMEDIATE`-Transaktion; derselbe unveränderliche
   Snapshot speist beide Phasen, neue Outbox-Einträge warten auf den nächsten
   Lauf): Outbox lesen (ohne
   `unsendable` der aktuellen Version); je Eintrag aus dem aktuellen Zustand
   eine Operation bauen — Zeile existiert → Upsert mit `changedAt` des
   Eintrags; Zeile fehlt → Delete (`video` → `videoGone` mit `tomb`,
   `membership` → `present = false`). Zweite Sicherung: Upserts für private
   Videos und deren Kinder verwerfen. **In derselben Transaktion** `published =
   1` für alle Videos setzen, deren uid in einer Operation vorkommt.
2. **Zwei Phasen:** Zuerst alle Löschungen (`videoGone`, `collectionDelete`,
   `chatDelete`, `summaryDelete`). Erst wenn **alle** davon quittiert sind,
   folgen die Upserts in der Reihenfolge collection, video, summary, chat,
   round, membership. Bricht Phase 1 ab, endet der Lauf ohne Phase 2. Grund:
   Eine Neuanlage derselben YouTube-ID bzw. desselben Sammlungsnamens würde
   sonst in die noch lebende alte Existenz verschmolzen und mit ihr gelöscht.
   Löschungen antworten nie mit `retry`.
3. **Packen** je Phase nach Anzahl und Bytes. Operationen über der
   Einzelgrenze → `unsendable` mit Grund am Eintrag, im Status gemeldet.
4. **Senden** blockweise; nach jeder Antwort kurze Schreibtransaktion:
   `ok`/`rejected` quittieren (nur wenn `seq` unverändert), `retry` behalten.

### Pull (Staging)

Seitenweise ab `cursor`; jede Seite in einer Schreibtransaktion nach
`sync_inbox`, `cursor = next`. Nach `more = false` Apply; liegt nach einem
Absturz schon etwas in der Inbox, wird es auch ohne neue Seiten angewandt.

### Apply

Eine `IMMEDIATE`-Transaktion, `applying = '1'`, Inbox je Schlüssel auf höchste
`seq` verdichtet. Reihenfolge: (1) `videoGone`, `collectionGone`, `chatGone`,
`summaryGone`; (2) `collection`, `video`, `summary`, `chat`, `round`,
`membership`.

**Grundregel:** Ein lebender Zustand wird übersprungen, wenn für seinen
Schlüssel ein Outbox-Eintrag aussteht — auch wenn lokal keine Zeile existiert
(eine ausstehende Löschung wird nie rückgängig gemacht). Grabsteine gelten
immer.

| Zustand | Lokale Lage | Wirkung |
| --- | --- | --- |
| `videoGone deleted` | Zeile mit uid | löschen (kaskadiert), Owner-Outbox löschen |
| `videoGone withdrawn` | Zeile mit uid | privat machen wie beim lokalen Umschalten: Owner-Outbox löschen, Teilbaum neu identifizieren, `local_only = 1`, `published = 0`, Inhalte bleiben. Ein erneutes Echo desselben Grabsteins findet die alte uid nicht mehr und wirkt nicht |
| `videoGone`/`collectionGone merged` → M | M hat einen ausstehenden Löschwunsch (Outbox-Eintrag für M, Zeile M fehlt) | Video: bei `tomb = deleted` X lokal löschen, bei `tomb = withdrawn` X privat machen; Sammlung: X lokal löschen. Der Löschwunsch für M bleibt unverändert |
| dito | Zeile X, keine Zeile M, kein Löschwunsch für M | uid X → M umbenennen; Outbox-Einträge mit Schlüssel/Owner/Parent X (inkl. Zuordnungsschlüssel) auf M umschreiben, `changed_at` erhalten, Kollisionen: jüngerer Eintrag bleibt |
| dito | Zeilen X und M | Kinder bzw. Zuordnungen von X an M hängen (doppelte Paare zusammenfassen), X löschen (Outbox von X explizit umschreiben, Trigger schweigen) |
| Alias und Grabstein der Wurzel im selben Batch | – | erst Alias auflösen, dann Grabstein auf alle betroffenen lokalen Zeilen anwenden; kein lebender Rest |
| `summaryGone`, `chatGone`, `collectionGone deleted` | Zeile | löschen, zugehörige Outbox-Einträge löschen |
| `video` | Zeile mit uid, privat | nichts (tritt nur durch Wettlauf auf) |
| `video` | Zeile mit uid, geteilt | Inhalt übernehmen |
| `video` | keine Zeile mit uid | einfügen (`local_only = 0`, `published = 1`), auch wenn eine andere Zeile dieselbe YouTube-ID hat |
| Kind-Zustände | Video privat oder fehlt | ignorieren |
| `summary`, `round` | uid bekannt | nichts |
| `chat` | – | Kopf übernehmen; `summaryUids` → lokale ids, unbekannte in `pendingSummaryUids` merken (`null`/`[]` bleiben) |
| `collection` | Namenskonflikt mit lokaler Sammlung anderer uid, die in der Outbox aussteht | **zurückstellen** (siehe unten) |
| `collection` | sonst | zweiphasig anwenden: erst geänderte Namen vorübergehend auf `\u0001<uid>` setzen, dann Endnamen — so stören Umbenennungen innerhalb eines Batches den `NOCASE`-Index nicht |
| `membership` | Sammlung ist zurückgestellt | **zurückstellen** |
| `membership` | Video privat oder fehlt, Sammlung fehlt ohne Zurückstellung | verwerfen |

**Zurückstellen:** Zurückgestellte Zustände bleiben in `sync_inbox` (auf
der Platte, überstehen Neustarts) und werden bei **jedem** Apply erneut
beurteilt, auch ohne neue Seiten. Ein höherer Zustand desselben Schlüssels
ersetzt den älteren; ein Grabstein der Sammlung erledigt sie und ihre
abhängigen Zuordnungen. Beispiel: B legt „KI“ lokal an, der Server bringt
eine andere „KI“ (Y) mit Zuordnungen → Y und Zuordnungen warten; benennt B
seine Sammlung um oder löscht sie, wird Y beim nächsten Apply eingefügt; wird
sie stattdessen serverseitig in Y verschmolzen, löst `merged` das auf.
Alle übrigen Inbox-Zustände werden nach dem Apply entfernt.

Danach: `pendingSummaryUids` aller Chats der betroffenen Videos gegen die
jetzt vorhandenen Summaries **desselben Videos** auflösen; abgeleitete `videos.summary*` neu setzen
(neueste nach `created_at DESC, uid DESC`); erledigte und verworfene Inbox-Zustände entfernen, zurückgestellte behalten; `applying`
entfernen; Commit; Event `sync://applied {videoIds, collections}`.

Beim Push werden lokale Summary-ids und `pendingSummaryUids` zusammen als
`summaryUids` gesendet.

### Auslöser und Status

- Start (wenn aktiviert); Schleife: alle 15 s Push, falls Outbox nicht leer;
  alle 120 s voller Lauf; Befehl `sync_now`.
- Backoff bei Netz/5xx bis 10 min; 401, 409, 426 halten an.
- Befehle: `sync_config_get`, `sync_config_set`, `sync_test`, `sync_now`,
  `sync_status` → `{enabled, running, lastSuccessAt, lastError, pending,
  unsendable: [{entity, reason}], stopped: null | "auth" | "dataset" |
  "version"}`, `sync_rebaseline`, `video_set_local_only(videoId, localOnly)`.
  Event `sync://status`.
- HTTP ohne Weiterleitungen, 60 s Timeout; Antworten werden gegen die
  Konfiguration des gestarteten Laufs geprüft (URL-Wechsel während eines Laufs
  quittiert nichts). Token nie in Logs, Fehlern oder Frontend.

### Einstellungen

`sync.json` neben `config.json`, atomar und privat (Muster `ai/auth.rs`):
`{enabled, serverUrl, token, newVideosLocal}`. URL mit dem `url`-Crate prüfen
(`https`, `http` nur für `localhost`/`127.0.0.1`). Frontend sieht `hasToken`
statt Token; leeres Tokenfeld = unverändert. `newVideosLocal`: `insert_video`
setzt `local_only`.

**Datensatz-Bindung:** Die gespeicherte Datensatz-ID wird bei URL-Wechsel
**nicht** gelöscht. Vor dem ersten Push/Pull nach Start oder URL-Wechsel holt
der Client die ID per `health`:
- keine gespeicherte ID (nie synchronisiert: `cursor = 0`, kein Video
  `published`) → speichern, weiter;
- gleiche ID (z. B. neue URL desselben Servers) → weiter mit Cursor;
- andere ID → anhalten (`stopped = "dataset"`), nur „Neu abgleichen“ hilft.

`add_video` prüft Dubletten zusätzlich atomar beim Einfügen
(`INSERT … WHERE NOT EXISTS`), nicht nur vor dem Netzwerkabruf.

### Neu abgleichen (`sync_rebaseline`)

Nur auf Knopfdruck nach Datensatzwechsel, unter dem Lauf-Mutex, eine
Transaktion: Inbox leeren, `cursor = 0`, Outbox-Einträge für alle Zeilen
nicht privater Videos und alle Sammlungen anlegen (bestehende Löschwünsche
bleiben), neue Datensatz-ID speichern. `published` bleibt unverändert (eine
mögliche frühere Veröffentlichung verpflichtet weiterhin zu einem Grabstein
beim Löschen).
Grenze: Nach dem Backup Gelöschtes kann von Geräten mit Altbestand
zurückkommen.

## Betrieb des Servers

```text
sync-server serve                  # SYNC_PORT (8080), SYNC_DB (/data/sync.db)
sync-server device add <name>      # druckt Token einmal; Name eindeutig
sync-server device list
sync-server device revoke <name>
sync-server dataset rotate         # neue datasetId (nach Restore Pflicht)
```

- Geräte-ID = interne Integer-ID (Tie-Break). Token: 32 Bytes aus
  `getrandom`, hex; gespeichert nur SHA-256, Suche über den Hash-Index.
- Backup täglich: `VACUUM INTO '<datum>.db.tmp'`, dann umbenennen; alte
  `.tmp` vorher entfernen; heutige Datei vorhanden → nichts tun; ältestes von
  mehr als 7 erst nach Erfolg löschen; Fehler ins Log, Dienst läuft weiter.
- Restore (`sync-server/README.md`): Container stoppen, Backup nach `sync.db`
  kopieren, vorhandene `-wal`/`-shm` entfernen, `dataset rotate`, starten.

## UI

- **Einstellungen, Tab „Sync“** (Muster `websearch-settings.ts`): Schalter
  „Synchronisation aktiv“, Server-URL, Token (Passwortfeld), „Neue Videos auf
  diesem Gerät nur lokal speichern“, „Verbindung testen“, „Jetzt
  synchronisieren“, Statuszeile (letzter Erfolg, Fehler, ausstehend, nicht
  sendbar mit Grund), bei `stopped = "dataset"` Hinweis + „Neu abgleichen“
  mit Erklärung.
- **Detailansicht:** Schalter „Nur lokal (nicht synchronisieren)“ bei den
  Sammlungen. Beim Einschalten eines veröffentlichten Videos Bestätigung: „Das
  Video wird vom Server entfernt. Andere Geräte behalten ihre Kopie als
  lokales Video.“
- **Liste:** Symbol an privaten Videos (`title`/`aria-label`), Filter „Nur
  lokale“.
- **Kopfleiste:** Sync-Symbol (aus/ok/läuft/Fehler), Klick = `sync_now`,
  Tooltip mit Stand.
- **Nach `sync://applied`:** Liste und Sammlungen neu laden; Detailansicht nur,
  wenn das aktive Video betroffen ist und keine Zusammenfassung oder
  Chat-Anfrage läuft (sonst danach). Wurde das aktive Video umbenannt
  (neue uid, lokale id bleibt), bleibt die Auswahl erhalten.

## Deployment

- `sync-server/Dockerfile` (Multi-Stage, Build-Kontext Repo-Root, nur
  `sync-proto/` und `sync-server/`, Laufzeit als Nicht-Root),
  `sync-server/deploy/docker-compose.yml`, `sync-server/deploy/deploy.sh`.
- VPS `/docker/youtube-sync/` (`docker-compose.yml`, `src/`, `data/`),
  Container `youtube-sync`, Netz `n8n_default` (extern), Traefik-Labels wie
  `junge-talente`: Router `youtube-sync`,
  ``Host(`yt-sync.srv1280390.hstgr.cloud`)``, `entrypoints=websecure`,
  `tls.certresolver=mytlschallenge`, Port 8080, `restart: always`.
- `deploy.sh`: rsync nach `hostinger:/docker/youtube-sync/src/`, Compose-Datei
  nach `/docker/youtube-sync/`, `docker compose up -d --build`, Health per
  HTTPS. Traefik und andere Stacks bleiben unberührt.
- Doku `apps/youtube-sync.md` im WebServer-Repo.

## Referenzfälle

A, B = Geräte, V = Video, „sync X“ = voller Lauf von X.

### Server (in `sync-server`)

| Nr. | Ablauf | Erwartung |
| --- | --- | --- |
| S1 | A pusht neues V | V lebt; Pull ab 0 liefert V |
| S2 | V Titel „neu“ (t2); B pusht „alt“ (t1) | Titel „neu“, V geechot |
| S3 | V mit Transkript; neueres V ohne | Transkript bleibt |
| S4 | V ohne Transkript (t2); älteres V (t1) mit | Transkript übernommen, Titel bleibt t2 |
| S5 | `summaryDelete U`, danach `summary U` | U gelöscht, `summaryGone` geechot |
| S6 | V `deleted`; Push von V und Kind (alte uid) | `rejected`, Echo `videoGone` |
| S7 | V `deleted`; neue uid, gleiche YouTube-ID | neue Existenz lebt |
| S8 | A und B legen dieselbe YouTube-ID an | eine Wurzel, `videoGone merged`, Kinder beider unter der Wurzel |
| S9 | `videoGone withdrawn` für V mit Kindern | Grabstein, Kinder weg; spätere Kinder-Pushes `rejected` |
| S10 | `videoGone` für unbekannte uid, danach `video` mit dieser uid | Grabstein bleibt, `video` `rejected` |
| S11 | V `withdrawn`; neue uid mit derselben YouTube-ID und neuen Kind-uids | neue Existenz mit diesen Kindern; Grenzfall: alte Kind-uids unter der neuen Existenz werden ebenfalls angenommen |
| S12 | Sammlung Y „KI“; X „ki“ + Zuordnung (V, X) | X `merged` → Y, (V, Y) |
| S13 | X „Forschung“ (t3); veralteter Rename von X auf „KI“ (t2), Y „KI“ existiert | nichts verschmolzen, X bleibt „Forschung“ |
| S14 | Ketten X→Y, Y→Z; Operation auf X | wirkt auf Z |
| S15 | Zuordnung present (t1), A false (t2), B true (t1' < t2) | `present = false`, geechot |
| S16 | Runde zu Chat mit Grabstein | `rejected`, Echo `chatGone` |
| S17 | Summary zu unbekanntem Video, Runde zu unbekanntem Chat | `retry` mit `missing` |
| S18 | Strukturfehler in Op 3 von 5 | 400 mit Index, keine Wirkung |
| S19 | Chat mit fremden `summaryUids` | fremde entfernt |
| S20 | falsches Token, falsche Version, falsches Dataset, `since` zu groß | 401, 426, 409 ohne Wirkung, 409 |
| S21 | Pull mit Seitenlimit, Schlüssel ändert sich zwischen Seiten | erscheint später erneut, `next` monoton; leere Seite `next = since` |
| S22 | Backup zweimal am Tag, Verzeichnis fehlt, Rest-`.tmp` | eine Datei, Fehler geloggt, Dienst läuft |
| S23 | Löschung einer unbekannten Summary-/Chat-/Sammlungs-uid vor dem Erst-Upload ihres Elternobjekts | Grabstein, `ok`, nie `retry`; eine spätere Neuanlage dieser uid bleibt gesperrt |

### Client-Datenmodell (in `src-tauri`, temporäre DB)

| Nr. | Fall | Erwartung |
| --- | --- | --- |
| C1 | Migration einer Alt-DB (Videos, Summaries, Chats mit mehreren Runden, Sammlungen, Legacy-Summary) | Tabellenumbau ohne Datenverlust, uids, Runden richtig gruppiert, Outbox je Zeile; zweiter Start ändert nichts |
| C2 | jeder INSERT-Pfad | uid gesetzt; ohne uid Abbruch |
| C3 | privates Video: Summary, Runde, Zuordnung | keine Outbox-Einträge |
| C4 | Summary löschen, dann Video privat schalten | Summary-Eintrag weg; bei `published = 1` genau ein `video`-Eintrag `withdrawn` mit alter uid; Video, Summaries, Chats, Runden mit neuen uids, Inhalte und Reihenfolgen unverändert |
| C5 | privates Video freigeben | Einträge für Video und alle Kinder |
| C6 | Video löschen: veröffentlicht / nie veröffentlicht | ein `deleted`-Eintrag / keiner; keine Kind-Einträge |
| C7 | Chat mit offener Runde löschen | Runden-Eintrag weg, `chat`-Eintrag da |
| C8 | Sammlung mit offenen Zuordnungen löschen | nur `collection`-Eintrag |
| C9 | Snapshot, danach UI-Änderung, dann Quittung | neuer Eintrag bleibt; Snapshot setzt `published` |
| C10 | Video löschen nach Snapshot, vor Senden | `deleted`-Eintrag entsteht (weil `published = 1`) |
| C11 | Apply lebender Zustand bei ausstehendem Eintrag, lokal keine Zeile (Löschwunsch) | nichts eingefügt, Eintrag bleibt |
| C12 | Apply `videoGone withdrawn`, danach dasselbe Echo erneut | privat, Teilbaum neu identifiziert, Inhalte bleiben, Owner-Outbox leer; zweites Echo wirkungslos |
| C13 | Apply `merged` (mit und ohne Zielzeile), Outbox-Einträge der alten uid | umgeschrieben, keine Dubletten, lokale Zuordnungen bleiben lokal |
| C13a | Apply `merged` X → M, M lokal mit ausstehendem Löschwunsch (Video `deleted`, Video `withdrawn`, Sammlung); der X-Upsert war vorher quittiert | X gelöscht bzw. privat; Löschwunsch M unverändert; nächster Snapshot sendet ihn als Delete |
| C14 | Apply fremder Runde mit gleichem `created_at` | gleiche Reihenfolge auf beiden DBs |
| C15 | zwei Summaries gleicher Zeit, entgegengesetzte Importreihenfolge | gleiche „neueste“ |
| C16 | `summaryUids` `null`, `[]`, bekannt, erst später ankommend | `null`/`[]` bleiben, späte uid wird aufgelöst |
| C17 | Apply zweier Sammlungen, die im Batch Namen tauschen; lokal ausstehende gleichnamige Sammlung | kein Constraint-Fehler; ausstehende Kollision übersprungen |
| C18 | Apply zwischen Lese- und Schreibphase von `append_chat_turn` | kein `BUSY_SNAPSHOT`, Runde gespeichert |
| C19 | Abbruch von Apply und Migration an jeder Stelle | Rollback, Wiederholung idempotent |
| C20 | Apply `video` mit YouTube-ID einer privaten Kopie | zweite Zeile, private unverändert |
| C21 | Migration: Kinderzahlen, Inhalte, Integer-ids, Referenzen, Autoincrement-Stand vor/nach Umbau; Abbruch mitten im Umbau | identisch bzw. Rollback |
| C22 | Kanonisierung alter Zeitstempel (`+00:00`, Nanosekunden, schon kanonisch) | einheitlich `…mmmZ`, Runden vorher korrekt gruppiert |
| C23 | zwei gleichzeitige `add_video` derselben YouTube-ID | genau eine Zeile |

### Ende-zu-Ende (Rust-Integrationstest, `sync-server` als dev-dependency, Server in-process)

| Nr. | Ablauf | Erwartung |
| --- | --- | --- |
| E1 | A: Video, 2 Summaries, Chat mit 2 Runden, Sammlung; sync A, B | B identisch |
| E2 | beide offline „KI“ anlegen und je ein Video zuordnen; sync A, B, A | eine „KI“ mit beiden Videos auf beiden |
| E3 | A löscht V, B hängt offline Summary an; sync A, B, A | V auf beiden weg |
| E4 | A schaltet V privat; sync A, B | Server ohne V; A und B je private Kopie mit eigener uid |
| E5 | B mit `newVideosLocal`; neues Video; sync B, A | Server und A kennen es nicht |
| E6 | Titel offline auf A (später) und B; sync B, A, B | beide = A |
| E7 | Server nicht erreichbar | Fehlerstatus, Outbox und Fachdaten unverändert (`published` darf gesetzt sein) |
| E8 | A: neues Video, Antwort verworfen; A schaltet privat und löscht es; sync A | Server hat Grabstein, kein Video |
| E9 | A: Grabstein vor verspätetem Anlegen derselben uid | Video bleibt weg |
| E10 | E4 mit Summary, Chat, 2 Runden; danach A gibt frei; sync A, B; danach löscht und ändert A Summary und Chat der geteilten Existenz; sync A, B | B hat die geteilte Existenz zusätzlich zu seiner privaten Kopie; die private Kopie bleibt vollständig unverändert |
| E11 | E10, danach B gibt seine private Kopie frei; sync B, A | eine Existenz; ursprünglich gemeinsame Summaries/Chats erscheinen doppelt (bekannte Grenze), keine geht verloren |
| E12 | A löscht V und legt es neu an; B schickt Altbestand | neue Existenz unberührt |
| E13 | zwei Push-Blöcke, zweiter bricht ab, Neustart | alles genau einmal, Rest sendbar |
| E14 | Pull bricht nach Seite 1 ab, Neustart | Inbox fortgesetzt, Endstand korrekt |
| E15 | Operation über Einzelgrenze und kleine Operation | kleine geht durch, große `unsendable` |
| E16 | `dataset rotate` mit ausstehendem Löschwunsch | keine Wirkung auf neuem Datensatz vor Bestätigung; nach „Neu abgleichen“ Bestand wieder oben |
| E17 | Runden offline auf A und B gleicher Zeit | identischer Verlauf |
| E18 | beide importieren dieselbe YouTube-ID mit je einer Summary offline | eine Existenz mit beiden Summaries auf beiden |
| E19 | A löscht Chat mit neuer, ungesendeter Runde; sync A | kein `unsendable`, Chat-Grabstein |
| E20 | A löscht V und legt dieselbe YouTube-ID neu an (bzw. privat + sofort wieder freigeben); jeweils in einem Block, auf zwei Blöcke verteilt, mit verworfener Grabstein-Antwort | neue Existenz lebt mit neuem Inhalt |
| E21 | Server-Sammlung Y „KI“ mit Zuordnung (V, Y); B legt nach seinem Snapshot „KI“ an; zwei volle Läufe — Varianten: B lässt „KI“ stehen / benennt sie vor dem zweiten Lauf frei um / löscht sie; jeweils mit App-Neustart nach dem ersten Apply | B hat Y und (V, Y) ohne weitere Änderung an Y |
| E22 | URL-Wechsel auf Server mit anderer Datensatz-ID und höherem Zähler, alte Inbox gefüllt | Anhalten vor jeder Wirkung; nach „Neu abgleichen“ vollständig |
| E23 | Neu abgleichen, danach löscht der Benutzer ein früher veröffentlichtes Video vor dem ersten Snapshot | Grabstein wird gesendet |
| E24 | `deleted` und `withdrawn` für dieselbe uid in beiden Ankunftsreihenfolgen | erster Grund bleibt; Geräte löschen bzw. privatisieren entsprechend |

## Etappen

| Etappe | Inhalt | Implementierung | Abnahme |
| --- | --- | --- | --- |
| 0 | `sync-proto`: Operationen, Zustände, Grenzen, `canonical_time`, `name_key`, Validierung, Serde-Tests | Orchestrator | vor 1/2 |
| 1 | Client-Datenmodell: Migration, Trigger, Outbox, `video_set_local_only`, IMMEDIATE, Sortierungen, `set_video_collections`-Diff, Snapshot, Quittung, Staging, Zurückstellen, Apply | Opus 5.5 | C1–C23 inkl. C13a, alle bisherigen Tests |
| 2 | Server: Merge, HTTP, Auth, CLI, Backup, Dataset, Dockerfile, Compose, `deploy.sh`, Library-Einstieg (`app(db_path) -> Router`) für Tests | Opus 5.5 (parallel zu 1) | S1–S23, `docker build` |
| 3 | Client-Engine: HTTP, Packen, Schleife, Einstellungen, Befehle, Neu abgleichen; Ende-zu-Ende-Tests. **E1, E4, E8, E10, E18, E20, E21 zuerst**, bevor der Rest folgt | Opus 5.5 | E1–E24 |
| 4 | UI | DeepSeek | UI-Tests, Screenshot-Prüfung durch Orchestrator |
| 5 | Deployment, Geräte, E1 gegen den VPS, Release-Build | Orchestrator | Health per HTTPS, E1 live |

## Bekannte Grenzen

- Uhrzeiten wirken nur auf LWW-Felder (Titel, Chat-Kopf, Sammlungsname,
  Zuordnung).
- Private Kopien sind entkoppelt: Löschen einer privaten Kopie wirkt nur
  lokal; nach Zurückziehen und erneutem Freigeben auf einem anderen Gerät kann
  ein Gerät eine private und eine geteilte Kopie desselben YouTube-Videos
  zeigen, bis es seine private freigibt (dann verschmelzen beide) oder löscht.
  Beim Verschmelzen erscheinen ursprünglich gemeinsame Summaries und Chats
  doppelt, weil die private Kopie neue uids bekommen hat.
- Treffen `deleted` und `withdrawn` für dieselbe uid zusammen, gilt der zuerst
  beim Server angekommene Grund.
- Bereits versendete Bytes sind nicht rückholbar; der Grabstein entfernt sie
  beim nächsten Push.
- Nach Server-Restore kann Gelöschtes von Geräten mit Altbestand zurückkommen.
- Die erste Synchronisation überträgt alle Vorschaubilder und Transkripte.
- Windows zunächst nur über die plattformneutrale Implementierung abgedeckt.

## Anhang: Plan-Review

### Revision 4 → 4.1 (Astra, Runde 4)

G1 (übersprungene Sammlung und Zuordnungen gingen verloren, wenn die lokale
Konfliktsammlung später umbenannt oder gelöscht statt verschmolzen wird):
Zurückstellen in `sync_inbox` statt Verwerfen (E21 mit Varianten und
Neustart). Dazu redaktionelle Angleichungen (Zeitstempel, Datensatz-ID),
ordnungserhaltende Neu-Identifizierung, gemeinsamer Snapshot beider Phasen,
Löschungen nie `retry` (S23), S11 präzisiert, Etappen-Abnahme vollständig.

### Revision 3 → 4 (Astra, Runde 3)

| Befund | Lösung |
| --- | --- |
| F1 Kinder privater Kopien | ganzer Teilbaum wird beim Privatisieren neu identifiziert (lokal und auf fremden Geräten); Duplikate beim späteren Verschmelzen als Grenze akzeptiert (C4, C12, E10, E11) |
| F2 Neuanlage vor Grabstein | Push in zwei Phasen, Löschungen zuerst und vollständig quittiert (E20) |
| F3 `merged` gegen ausstehende Löschung | eigene Apply-Zeile: Ziel mit Löschwunsch → X löschen bzw. privatisieren (C13a) |
| F4 übersprungene Zuordnungen | Server echot beim Verschmelzen Wurzel und alle Zuordnungen (E21) |
| F5 Datensatz bei URL-Wechsel, `published` bei Neuabgleich | ID wird bei URL-Wechsel geprüft statt vergessen; Neuabgleich lässt `published` stehen (E22, E23) |
| Zeitformat (MITTEL) | Migration kanonisiert alle lokalen Zeitstempel (C22) |
| `deleted` vs. `withdrawn` (MITTEL) | erster Grund gilt, beide Reihenfolgen getestet (E24) |
| Hinweise | übernommen: FK-Aus vor `BEGIN`, Umbaureihenfolge, C21; atomare Dublettenprüfung (C23); Alias vor Grabstein im selben Batch; Outbox-Umschreiben inkl. Zuordnungsschlüssel |

Die Privatkopie-Semantik (entkoppelt, Löschen einer privaten Kopie nur lokal)
hat Astra als fachlich vertretbar bewertet; der Benutzer hat ihr am
2026-09-24 zugestimmt.

### Revision 2 → 3 (Astra, Runde 2)

| Befund | Lösung |
| --- | --- |
| N1 Revision zu spät bestätigt | Revisionen entfallen; Kinder brauchen nur ein lebendes Elternobjekt |
| N2 Lifecycle ohne kausale Vorbedingung | Lifecycle entfällt; Zurückziehen = endgültiger Grabstein einer uid, Freigeben = Neuanlage/Verschmelzung; nichts ist an eine Frist gebunden |
| N3 NULL ≠ „nie veröffentlicht“ | `published` wird im Snapshot gesetzt, bevor HTTP beginnt; Grabsteine auch für unbekannte uids (E8, E9, C10) |
| N4 Apply macht ausstehende Löschung rückgängig | Grundregel: Überspringen auch ohne lokale Zeile (C11) |
| N5 Identitätslagen bei Videos | lokales `UNIQUE(video_id)` entfällt; private und geteilte Kopie bestehen nebeneinander; Server: höchstens eine lebende Existenz je YouTube-ID, auch beim erneuten Freigeben (Neuanlage wird verschmolzen) (C20, E10, E11) |
| N6 Sammlungen | Namens-LWW vor Kollisionsprüfung (S13); lokale Kollision mit ausstehender Sammlung übersprungen, zweiphasige Umbenennung (C17) |
| N7 Eltern/Runden/Kontext | Chat-Trigger räumt offene Runden ab; Eltern-Grabstein → `rejected` + Echo, unbekannt → `retry` mit `missing`; `pendingSummaryUids` (C7, C16, S16, S17, E19) |
| N8 Dataset | `X-Sync-Dataset` vor jeder Wirkung geprüft; Neu abgleichen atomar inkl. Inbox (E16) |
| Hinweise ohne Vertragsbefund | übernommen: monotone Outbox-`seq`, Umschreiben bei Umbenennung, Quittung je `seq`, `next` bei leerer Seite, kanonische Zeitstempel, Feldgewinner-Zeitstempel, `unsendable` je Version, Restore mit WAL/SHM und `.tmp` |

Die Alias-Entscheidung aus Revision 2 (Operationen auf Alias-uids wirken auf
die Wurzel) hat Astra bestätigt.

### Revision 1 → 2 (Astra, Runde 1)

19 Befunde; Kernänderungen: uid je Video-Existenz, endgültige Grabsteine,
Outbox als schmutzige Schlüssel mit Owner, Echo, Chat-Runden als Einheit,
Pull-Staging, `IMMEDIATE`, Byte-Grenzen, Validierung über getaggte
Operationen, Datensatz-ID. Details im Review-Verlauf (`.herd/`, nicht
versioniert).
