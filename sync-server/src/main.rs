//! CLI des Sync-Servers (Spec, Abschnitt „Betrieb des Servers“).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;
use sync_server::db;

const USAGE: &str = "Aufruf:
  sync-server serve                  # SYNC_PORT (8080), SYNC_DB (/data/sync.db)
  sync-server device add <name>      # druckt das Token einmal
  sync-server device list
  sync-server device revoke <name>
  sync-server dataset rotate         # neue datasetId (nach Restore Pflicht)";

fn main() -> ExitCode {
    let db_path =
        PathBuf::from(std::env::var("SYNC_DB").unwrap_or_else(|_| "/data/sync.db".into()));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["serve"] => serve(&db_path),
        ["device", "add", name] => db::open(&db_path)
            .and_then(|conn| db::add_device(&conn, name))
            .map(|token| println!("{token}")),
        ["device", "list"] => db::open(&db_path)
            .and_then(|conn| db::list_devices(&conn))
            .map(|devices| {
                for (id, name, created_at) in devices {
                    println!("{id}\t{name}\t{created_at}");
                }
            }),
        ["device", "revoke", name] => {
            db::open(&db_path).and_then(|conn| match db::revoke_device(&conn, name)? {
                true => Ok(()),
                false => Err(sync_server::Error::Malformed(format!(
                    "kein Gerät '{name}'"
                ))),
            })
        }
        ["dataset", "rotate"] => db::open(&db_path)
            .and_then(|conn| db::rotate_dataset(&conn))
            .map(|id| println!("{id}")),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Fehler: {err}");
            ExitCode::FAILURE
        }
    }
}

fn serve(db_path: &Path) -> sync_server::Result<()> {
    let port: u16 = match std::env::var("SYNC_PORT") {
        Ok(text) => text
            .parse()
            .map_err(|_| sync_server::Error::Malformed(format!("SYNC_PORT ungültig: {text}")))?,
        Err(_) => 8080,
    };
    let backup_dir = db_path.parent().unwrap_or(Path::new(".")).join("backups");
    std::fs::create_dir_all(&backup_dir)?;
    let app = sync_server::app(db_path)?;
    spawn_backups(db_path.to_owned(), backup_dir);
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
        eprintln!(
            "sync-server hört auf Port {port}, Datenbank {}",
            db_path.display()
        );
        sync_server::serve(listener, app, shutdown()).await
    })?;
    Ok(())
}

/// Stündliche Prüfung, ob das heutige Backup fehlt; eigene Verbindung, damit
/// Fehler den Dienst nicht berühren.
fn spawn_backups(db_path: PathBuf, dir: PathBuf) {
    std::thread::spawn(move || loop {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        match db::open(&db_path) {
            Ok(conn) => db::backup_logged(&conn, &dir, &today),
            Err(err) => eprintln!("Backup {today} fehlgeschlagen: {err}"),
        }
        std::thread::sleep(Duration::from_secs(3600));
    });
}

/// SIGTERM (`docker stop`) oder Strg+C.
async fn shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate()).expect("SIGTERM-Handler");
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
