# sync-server

Kleiner Sync-Server für die YouTube-Summarizer-App. Er tauscht nur Änderungen
zwischen den Rechnern aus; die lokale Datenbank jeder App bleibt die einzige
Datenquelle. Verbindlich ist [`docs/spec-sync.md`](../docs/spec-sync.md); die
gemeinsamen Protokolltypen liegen in [`sync-proto/`](../sync-proto/).

Eigenständiges Cargo-Projekt, kein Workspace im Repo-Root.

## Entwicklung

```bash
cd sync-server
cargo test          # Merge-Regeln (src/tests.rs) und HTTP (tests/http.rs)
cargo fmt --check
cargo clippy --all-targets -- -D warnings
SYNC_DB=/tmp/sync.db SYNC_PORT=8080 cargo run -- serve
```

Docker-Image (Build-Kontext ist das Repo-Root, `Dockerfile.dockerignore`
beschränkt ihn auf `sync-proto/` und `sync-server/`):

```bash
docker build -f sync-server/Dockerfile .
```

### Server in Tests starten

`src-tauri` bindet den Server als dev-dependency ein und startet ihn in-process
auf einem Zufallsport:

```rust
let db_path = dir.path().join("sync.db");
let conn = sync_server::db::open(&db_path)?;
let token = sync_server::db::add_device(&conn, "A")?;
let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
let url = format!("http://{}", listener.local_addr()?);
let app = sync_server::app(&db_path)?;
tokio::spawn(sync_server::serve(listener, app, std::future::pending()));
```

## Betrieb

```text
sync-server serve                  # SYNC_PORT (8080), SYNC_DB (/data/sync.db)
sync-server device add <name>      # druckt das Token einmal; Name eindeutig
sync-server device list            # id, Name, angelegt
sync-server device revoke <name>
sync-server dataset rotate         # neue datasetId (nach Restore Pflicht)
```

- Endpunkte: `GET /v1/health` (ohne Auth), `POST /v1/push`, `GET /v1/pull?since=<seq>`.
- Die Geräte-ID ist die interne Integer-ID; sie entscheidet LWW-Gleichstände und
  wird nie wiederverwendet. Gespeichert wird nur der SHA-256-Hash des Tokens.
- Ein Push-Body darf höchstens 32 MiB groß sein (sonst 413).
- `SIGTERM` (z. B. `docker stop`) beendet den Dienst geordnet.

## Deployment (VPS)

Auf dem VPS liegt der Stack unter `/docker/youtube-sync/`:
`docker-compose.yml`, `src/` (Quellen) und `data/` (Datenbank und Backups,
gehört UID 10001). Container `youtube-sync`, externes Netz `n8n_default`,
Traefik-Router `youtube-sync` für `yt-sync.srv1280390.hstgr.cloud`.

```bash
sync-server/deploy/deploy.sh
```

Das Skript kopiert Quellen und Compose-Datei per `rsync`, baut und startet mit
`docker compose up -d --build` und prüft danach `/v1/health` per HTTPS. Traefik
und andere Stacks bleiben unberührt.

## Geräte

```bash
ssh hostinger 'docker exec youtube-sync sync-server device add laptop'
ssh hostinger 'docker exec youtube-sync sync-server device list'
ssh hostinger 'docker exec youtube-sync sync-server device revoke laptop'
```

Das Token erscheint nur einmal; es gehört in die Sync-Einstellungen der App.
Nach einem Widerruf antwortet der Server diesem Gerät mit 401.

## Backup

`serve` legt `data/backups/` beim Start an und prüft stündlich, ob das heutige
Backup fehlt:

- `VACUUM INTO 'backups/<YYYY-MM-DD>.db.tmp'`, danach umbenennen;
- übrig gebliebene `.tmp` werden vorher entfernt, eine vorhandene heutige Datei
  lässt den Tag aus;
- nach Erfolg werden die ältesten Backups über 7 hinaus gelöscht;
- Fehler landen im Log (`docker logs youtube-sync`), der Dienst läuft weiter.

Andere Dateien im Backup-Verzeichnis werden nicht angefasst.

## Restore

```bash
ssh hostinger
cd /docker/youtube-sync
docker compose stop youtube-sync
cp data/backups/<YYYY-MM-DD>.db data/sync.db
rm -f data/sync.db-wal data/sync.db-shm
chown 10001:10001 data/sync.db
docker compose run --rm youtube-sync dataset rotate
docker compose up -d
```

`dataset rotate` ist Pflicht: Die Geräte erkennen die neue Datensatz-ID,
halten an und bieten „Neu abgleichen“ an. Grenze: Was nach dem Backup gelöscht
wurde, kann von Geräten mit Altbestand zurückkommen.
