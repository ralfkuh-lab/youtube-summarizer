# Release

Checkliste für einen Release-Build und Hinweise zum Paketieren je Plattform.
Befehle laufen im Repository-Root, sofern nicht anders angegeben.

## Checkliste

1. Stand sauber: `git status` ohne offene Änderungen, `main` entspricht `origin/main`.
2. Version anheben, wenn der Build verteilt wird: `version` in
   `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` und `package.json`
   gleich halten.
3. Gates:
   ```bash
   npm run build
   npm run test:ui
   (cd src-tauri && cargo fmt --check && cargo test)
   ```
   Bei Änderungen am Sync zusätzlich `cargo test` in `sync-proto/` und
   `sync-server/`; ein geänderter Server wird per
   `sync-server/deploy/deploy.sh` ausgerollt (siehe `sync-server/README.md`),
   bevor Clients mit neuem Protokoll verteilt werden.
4. Release-Build: `npm run tauri -- build`. Unter Linux zeigen danach die
   Symlinks `youtube-summarizer-release` und `youtube-summarizer.deb` im
   Projekt-Root auf die neuen Artefakte.
5. Installieren und den Build selbst prüfen (nicht nur die Tests):
   - Linux: `sudo dpkg -i youtube-summarizer.deb` (nur der Maintainer, sudo).
   - Windows: Installer aus `src-tauri/target/release/bundle/nsis/`.
   - Kurztest in der installierten App: Video hinzufügen (Transkript lädt),
     Zusammenfassung streamt, Chat antwortet, Sync-Knopf gleicht ab,
     „An Agent übergeben“ kopiert ein Kommando.
6. `TODO.md` („Last Verified State“) und `docs/verification-log.md` mit
   Datum, Gates und getesteten Plattformen nachziehen; committen und pushen.

## Linux

Ziel ist das deb-Paket (dpkg-Name `you-tube-summarizer`, Binary
`/usr/bin/youtube-summarizer`); `bundle.targets: "all"` erzeugt zusätzlich
rpm und AppImage unter `src-tauri/target/release/bundle/`. Für Videos und
Codecs empfiehlt das Paket die GStreamer-Plugins (`tauri.conf.json`).

## Windows

Siehe Abschnitt „Windows“ in [`AGENTS.md`](../AGENTS.md).

## macOS (ungetestet)

Bisher wurde die App auf macOS weder gebaut noch gestartet. Erwartet nach
Tauri-2-Standard, beim ersten Lauf zu bestätigen und hier nachzutragen:

- Voraussetzungen: Xcode Command Line Tools (`xcode-select --install`),
  Rust, Node >= 20, `npm install`.
- `npm run tauri -- build` erzeugt `YouTube Summarizer.app` unter
  `src-tauri/target/release/bundle/macos/` und ein `.dmg` unter
  `bundle/dmg/`. Das Icon `icons/icon.icns` ist vorhanden.
- Der Build ist weder signiert noch notarisiert. Gatekeeper blockiert ihn
  daher beim ersten Start; lokal hilft Rechtsklick → „Öffnen“ oder
  `xattr -dr com.apple.quarantine "/Applications/YouTube Summarizer.app"`.
  Für eine Weitergabe wären ein Apple-Developer-Zertifikat und Notarisierung
  nötig (Tauri: `APPLE_SIGNING_IDENTITY`, `APPLE_ID` u. a.).
- Zu prüfen: Das Fenster lädt über `tauri-plugin-localhost` von
  `http://localhost:14220` (eingehende-Verbindungen-Dialog der Firewall?),
  Video-Wiedergabe im WKWebView, Zwischenablage bei der Agent-Übergabe, die
  POSIX-Agentenvorlage mit der Standard-Shell zsh, Lage des
  App-Datenverzeichnisses (`~/Library/Application Support/dev.ralf.youtube-summarizer`).
