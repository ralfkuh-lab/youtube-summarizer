# Code-Review vom 2026-09-06

Die Trennung zwischen Tauri, YouTube-Anbindung, KI und Speicherung ist
grundsätzlich sinnvoll. Die folgenden Punkte sollten gezielt verbessert werden.
Der Review umfasst die zentralen Abläufe und Modulgrenzen; die App wurde dabei
nicht interaktiv getestet. Bis auf die separat reproduzierte Race Condition
beruhen die Befunde auf Code-Inspektion.

## 1. Hohe Priorität: Race Conditions beim Videowechsel

`selectVideo()` in [src/main.ts](../src/main.ts) (Zeile 721 zum Review-Zeitpunkt)
zeigt eine Antwort an, ohne zu prüfen, ob das Video noch ausgewählt ist.
Bei schnellem Wechsel kann rechts Video A erscheinen, während Aktionen Video B
betreffen. Auch der Erfolgsfall von `refreshActiveTranscript()` hat diese Lücke.

Die Race Condition wurde mit der aus dem Quelltext übernommenen Funktion und
kontrolliert verzögerten Antworten reproduziert: erst Video 1, dann Video 2
auswählen; Antwort 2 vor Antwort 1 auflösen. Ergebnis: `activeVideoId = 2`,
angezeigtes Video = 1.

**Abhilfe:** Auswahl beziehungsweise Anfrage-ID vor jedem UI-Update prüfen.
Regressionstests sollten vertauschte Antwortreihenfolgen und einen Videowechsel
während des Transkript-Neuladens abdecken.

## 2. Konfiguration hat zwei konkurrierende Zustände

Einstellungen arbeiten mit gehaltenem Speicherzustand; Zusammenfassungen laden
Konfiguration und Schlüssel erneut von Platte in `summarize_video_impl()` in
[commands.rs](../src-tauri/src/commands.rs) (Zeile 810).
Gleichzeitig ändern Setter den Speicherzustand vor dem Speichern, etwa
`provider_enable()` in [ai/config.rs](../src-tauri/src/ai/config.rs) (Zeile 79)
und `set()`/`remove()` in [ai/auth.rs](../src-tauri/src/ai/auth.rs).
Scheitert das Schreiben, können Einstellungen und Zusammenfassung unterschiedliche
Werte verwenden. Ein erneuter identischer Setter-Aufruf kann zudem wegen des
bereits geänderten Speicherzustands ohne weiteren Schreibversuch zurückkehren.

**Abhilfe:** einen gemeinsamen Konfigurationsdienst verwenden und Änderungen
erst nach erfolgreichem Speichern übernehmen. Schreibfehler und anschließende
Wiederholungen gezielt testen.

## 3. Unvollständige KI-Antworten können als erfolgreich gelten

Beim Stream-Ende genügt bereits irgendein Text, auch ohne Abschlussereignis:
`chat_stream_cancellable()` und `finish_stream()` in
[ai/client.rs](../src-tauri/src/ai/client.rs) (Zeilen 219 und 253).
Schließt ein Provider den Stream vorzeitig regulär, wird die Teilantwort als
erfolgreiche Zusammenfassung gespeichert. Transportfehler werden dagegen
bereits als Fehler behandelt.

**Abhilfe:** vollständigen Abschluss verfolgen und unvollständige Ergebnisse
entsprechend behandeln. Einen Stream testen, der Text liefert und anschließend
ohne Abschlussereignis endet.

## 4. Die Bibliothek lädt unnötig viele Daten

`get_videos()` in [storage.rs](../src-tauri/src/storage.rs) (Zeile 187) lädt
alle Transkripte, Zusammenfassungen und Bilder. Zusätzlich führt
`hydrate_video_collections()` eine Sammlungsabfrage pro Video aus.
Bei wachsender Bibliothek steigen dadurch Datenvolumen und Speicherbedarf;
eine konkrete Performance-Grenze wurde im Review nicht gemessen.

**Abhilfe:** schlanke Listenobjekte verwenden, Details nachladen und
Sammlungszuordnungen gesammelt abfragen. Das Verhalten mit einer größeren
Testbibliothek prüfen.

## 5. Standardmodell umgeht die Freischaltungsprüfung

`resolve_summary_model()` in [commands.rs](../src-tauri/src/commands.rs)
(Zeile 877) prüft Anbieter und Whitelist nur bei expliziter Auswahl.
Ein inzwischen deaktiviertes Standardmodell bleibt bei einem Aufruf ohne
explizite Modellauswahl darüber verwendbar.

**Abhilfe:** beide Auswahlwege durch dieselbe Validierung führen. Tests für
ein Standardmodell mit deaktiviertem Anbieter beziehungsweise entferntem
Whitelist-Eintrag ergänzen.

## Refactoring der Modulgrenzen

Zuerst [src/main.ts](../src/main.ts) mit rund 2.100 Zeilen in Bibliothek,
Video-Details und Zusammenfassungsdialog aufteilen. Aus
[commands.rs](../src-tauri/src/commands.rs) gehören Migration und
Zusammenfassungslogik in eigene Module; die Commands sollten hauptsächlich
Aufrufe weiterreichen. Dazu gezielte Tests für asynchrone UI-Zustände und
Speicherfehler ergänzen.

## Verifikation

- `npm run build`: erfolgreich; bestehende Warnung zu großen Bundle-Chunks.
- `cargo test`: 80 bestanden, ein Netzwerktest ignoriert.
- Race Condition beim Videowechsel mit kontrollierter Antwortreihenfolge
  reproduziert.
- Im Review keine Codeänderungen vorgenommen und keinen Dev-Server oder
  Tauri-Prozess gestartet. Die Befunde sind noch offen.
