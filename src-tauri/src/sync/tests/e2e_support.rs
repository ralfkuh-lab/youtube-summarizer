//! Ende-zu-Ende-Umgebung: `sync-server` in-process auf Zufallsport, Geräte
//! mit eigener temporärer DB und eigener Engine.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use rusqlite::Connection;
use sync_proto::{State, PULL_PAGE_STATES};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{open, temp_paths};
use crate::storage::AppPaths;
use crate::sync::client;
use crate::sync::config::{self, SyncConfig, SyncConfigInput};
use crate::sync::engine::{Mode, SyncEngine, SyncStatus};

pub(crate) struct Server {
    pub url: String,
    db_path: PathBuf,
    tokens: Vec<String>,
    _dir: TempDir,
}

/// Startet einen Server mit `devices` Geräten.
pub(crate) async fn server(devices: usize) -> Server {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("sync.db");
    let conn = sync_server::db::open(&db_path).unwrap();
    let tokens = (0..devices)
        .map(|index| sync_server::db::add_device(&conn, &format!("Gerät {index}")).unwrap())
        .collect();
    drop(conn);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = sync_server::app(&db_path).unwrap();
    tokio::spawn(sync_server::serve(listener, app, std::future::pending()));
    Server {
        url,
        db_path,
        tokens,
        _dir: dir,
    }
}

impl Server {
    /// Alle Zustände ab 0 (je Schlüssel der aktuelle), über HTTP gelesen.
    pub async fn states(&self) -> Vec<State> {
        let config = self.config(0);
        let http = client::http_client().unwrap();
        let dataset = client::health(&http, &config).await.unwrap().dataset_id;
        let mut since = 0;
        let mut states = Vec::new();
        loop {
            let page = client::pull(&http, &config, &dataset, since).await.unwrap();
            assert!(page.states.len() <= PULL_PAGE_STATES);
            states.extend(page.states.into_iter().map(|state| state.state));
            since = page.next;
            if !page.more {
                return states;
            }
        }
    }

    /// `sync-server dataset rotate` (nach einem Restore).
    pub fn rotate_dataset(&self) -> String {
        let conn = sync_server::db::open(&self.db_path).unwrap();
        sync_server::db::rotate_dataset(&conn).unwrap()
    }

    /// Einstellungen, mit denen ein Gerät auf diesen Server umzieht.
    pub fn input(&self, device: usize) -> SyncConfigInput {
        let config = self.config(device);
        SyncConfigInput {
            enabled: true,
            server_url: config.server_url,
            token: Some(config.token),
            new_videos_local: false,
        }
    }

    pub fn config(&self, device: usize) -> SyncConfig {
        SyncConfig {
            enabled: true,
            server_url: self.url.clone(),
            token: self.tokens[device].clone(),
            new_videos_local: false,
        }
    }
}

pub(crate) struct Device {
    pub paths: AppPaths,
    pub engine: SyncEngine,
    _dir: TempDir,
}

impl Device {
    pub fn new(server: &Server, index: usize) -> Device {
        let (dir, paths) = temp_paths();
        config::save(&paths, &server.input(index)).unwrap();
        Device {
            engine: engine(&paths),
            paths,
            _dir: dir,
        }
    }

    /// Voller Lauf; ein Fehler lässt den Test scheitern.
    pub async fn sync(&self) -> SyncStatus {
        let status = self.engine.run(Mode::Full).await;
        assert_eq!(status.last_error, None, "{status:?}");
        assert_eq!(status.stopped, None, "{status:?}");
        status
    }

    /// App-Neustart: neue Engine, gleiche DB.
    pub fn restart(&mut self) {
        self.engine = engine(&self.paths);
    }

    pub fn conn(&self) -> Connection {
        open(&self.paths)
    }

    pub fn config(&self) -> SyncConfig {
        config::load(&self.paths)
    }

    pub fn dataset(&self) -> String {
        crate::sync::state_get(&self.conn(), "dataset_id")
            .unwrap()
            .unwrap()
    }
}

fn engine(paths: &AppPaths) -> SyncEngine {
    SyncEngine::new(paths.clone(), Box::new(|_| {})).unwrap()
}

/// Geteilter Inhalt einer DB ohne lokale ids und Metaspalten: gleiche
/// Projektion = gleicher Datenbestand.
pub(crate) fn projection(paths: &AppPaths) -> Vec<String> {
    let conn = open(paths);
    let queries = [
        "SELECT uid, video_id, url, title, transcript, summary, local_only FROM videos ORDER BY uid",
        "SELECT s.uid, v.uid, s.created_at, s.summary, s.provider, s.model FROM summaries s \
         JOIN videos v ON v.id = s.video_id ORDER BY s.uid",
        "SELECT c.uid, v.uid, c.title, c.created_at, c.updated_at FROM chats c \
         JOIN videos v ON v.id = c.video_id ORDER BY c.uid",
        "SELECT c.uid, m.round_uid, m.position, m.role, m.content, m.created_at \
         FROM chat_messages m JOIN chats c ON c.id = m.chat_id \
         ORDER BY c.uid, m.created_at, m.round_uid, m.position",
        "SELECT uid, name FROM collections ORDER BY uid",
        "SELECT v.uid, c.uid FROM video_collections vc JOIN videos v ON v.id = vc.video_id \
         JOIN collections c ON c.id = vc.collection_id ORDER BY v.uid, c.uid",
    ];
    let mut lines = Vec::new();
    for sql in queries {
        let mut stmt = conn.prepare(sql).unwrap();
        let columns = stmt.column_count();
        let rows = stmt
            .query_map([], |row| {
                (0..columns)
                    .map(|index| row.get::<_, rusqlite::types::Value>(index))
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap();
        for row in rows {
            lines.push(format!("{:?}", row.unwrap()));
        }
    }
    lines
}

/// Minimaler HTTP/1.1-Server für Antworten, die der echte Server bei
/// korrektem Client nie liefert (413, 400 mit Index, 5xx). `respond` bekommt
/// Methode, Pfad und Body und liefert Status und JSON.
pub(crate) struct Mock {
    pub url: String,
    pub requests: Arc<AtomicUsize>,
}

type Respond = dyn Fn(&str, &str, &[u8]) -> (u16, String) + Send + Sync;

pub(crate) async fn mock(respond: Arc<Respond>) -> Mock {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let respond = respond.clone();
            let counter = counter.clone();
            tokio::spawn(async move {
                let mut data = Vec::new();
                let mut buffer = [0u8; 65536];
                let (head_end, length) = loop {
                    let read = stream.read(&mut buffer).await.unwrap();
                    if read == 0 {
                        return;
                    }
                    data.extend_from_slice(&buffer[..read]);
                    if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&data[..end]).to_lowercase();
                        let length = head
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .map_or(0, |value| value.trim().parse().unwrap());
                        break (end + 4, length);
                    }
                };
                while data.len() < head_end + length {
                    let read = stream.read(&mut buffer).await.unwrap();
                    data.extend_from_slice(&buffer[..read]);
                }
                counter.fetch_add(1, Ordering::SeqCst);
                let head = String::from_utf8_lossy(&data[..head_end]).to_string();
                let mut parts = head.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let path = parts.next().unwrap_or("").to_string();
                let (status, body) = respond(&method, &path, &data[head_end..head_end + length]);
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    Mock { url, requests }
}

impl Mock {
    pub fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

/// Gerät gegen eine beliebige URL (z. B. einen Mock).
pub(crate) fn device_at(url: &str) -> Device {
    let (dir, paths) = temp_paths();
    config::save(
        &paths,
        &SyncConfigInput {
            enabled: true,
            server_url: url.to_string(),
            token: Some("token".into()),
            new_videos_local: false,
        },
    )
    .unwrap();
    Device {
        engine: engine(&paths),
        paths,
        _dir: dir,
    }
}
