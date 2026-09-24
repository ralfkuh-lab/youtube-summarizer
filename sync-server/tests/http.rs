//! S18, S20 und Durchstich per HTTP gegen den in-process gestarteten Server
//! (derselbe Weg, den `src-tauri` in seinen Ende-zu-Ende-Tests nutzt).

use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use sync_proto::{MAX_PUSH_BODY_BYTES, PROTOCOL_VERSION};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const T1: &str = "2026-09-24T10:00:00.000Z";

struct Server {
    addr: SocketAddr,
    db_path: PathBuf,
    token: String,
    dataset: String,
}

async fn start() -> Server {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "sync-server-http-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("sync.db");
    let conn = sync_server::db::open(&db_path).unwrap();
    let token = sync_server::db::add_device(&conn, "A").unwrap();
    let dataset = sync_server::db::dataset_id(&conn).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = sync_server::app(&db_path).unwrap();
    tokio::spawn(sync_server::serve(listener, app, std::future::pending()));
    Server {
        addr,
        db_path,
        token,
        dataset,
    }
}

struct Request<'a> {
    method: &'a str,
    path: String,
    token: &'a str,
    protocol: String,
    dataset: &'a str,
    body: Vec<u8>,
}

impl Server {
    fn get(&self, path: &str) -> Request<'_> {
        Request {
            method: "GET",
            path: path.into(),
            token: &self.token,
            protocol: PROTOCOL_VERSION.to_string(),
            dataset: &self.dataset,
            body: Vec::new(),
        }
    }

    fn push(&self, ops: Value) -> Request<'_> {
        Request {
            method: "POST",
            body: serde_json::to_vec(&json!({ "ops": ops })).unwrap(),
            ..self.get("/v1/push")
        }
    }

    /// Minimaler HTTP/1.1-Client: eine Anfrage je Verbindung.
    async fn send(&self, request: Request<'_>) -> (u16, Value) {
        let mut stream = TcpStream::connect(self.addr).await.unwrap();
        let head = format!(
            "{} {} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\
             Authorization: Bearer {}\r\nX-Sync-Protocol: {}\r\nX-Sync-Dataset: {}\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            request.method,
            request.path,
            request.token,
            request.protocol,
            request.dataset,
            request.body.len()
        );
        stream.write_all(head.as_bytes()).await.unwrap();
        // Bei 413 schließt der Server eventuell vor dem Ende des Bodys.
        let _ = stream.write_all(&request.body).await;
        let mut response = Vec::new();
        let _ = stream.read_to_end(&mut response).await;
        let text = String::from_utf8_lossy(&response);
        let status = text[9..12].parse().unwrap();
        let body = text.split_once("\r\n\r\n").unwrap().1;
        (status, serde_json::from_str(body).unwrap_or(Value::Null))
    }

    async fn states(&self) -> Vec<Value> {
        let (status, body) = self.send(self.get("/v1/pull?since=0")).await;
        assert_eq!(status, 200, "{body}");
        body["states"].as_array().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.db_path.parent().unwrap());
    }
}

fn uid(n: u32) -> String {
    format!("{n:032x}")
}

fn video(uid: &str) -> Value {
    json!({
        "type": "video", "uid": uid, "changedAt": T1, "youtubeId": "dQw4w9WgXcQ",
        "url": "https://www.youtube.com/watch?v=dQw4w9WgXcQ", "title": "Titel",
        "thumbnailUrl": "https://i.ytimg.com/x.jpg", "thumbnailData": null,
        "transcript": null, "chapters": null, "publishedAt": null,
        "description": null, "transcriptError": null, "createdAt": T1
    })
}

fn collection(uid: &str, name: &str) -> Value {
    json!({ "type": "collection", "uid": uid, "changedAt": T1, "name": name, "createdAt": T1 })
}

#[tokio::test]
async fn health_needs_no_auth_and_push_pull_round_trip() {
    let server = start().await;
    let (status, body) = server
        .send(Request {
            token: "falsch",
            protocol: "0".into(),
            dataset: "",
            ..server.get("/v1/health")
        })
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        body,
        json!({ "status": "ok", "protocol": 1, "datasetId": server.dataset })
    );

    let (status, body) = server
        .send(server.push(json!([video(&uid(1)), collection(&uid(2), "KI")])))
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "results": [{ "status": "ok" }, { "status": "ok" }] })
    );
    let (status, body) = server.send(server.get("/v1/pull?since=0")).await;
    assert_eq!(status, 200);
    assert_eq!(body["next"], 2);
    assert_eq!(body["more"], false);
    assert_eq!(body["states"][0]["type"], "video");
    assert_eq!(body["states"][0]["seq"], 1);
    assert_eq!(body["states"][1]["name"], "KI");
}

#[tokio::test]
async fn s18_structural_error_in_op_3_of_5_is_400_with_index_and_no_effect() {
    let server = start().await;
    let ops = json!([
        collection(&uid(1), "a"),
        collection(&uid(2), "b"),
        collection("KEINE-UID", "c"),
        collection(&uid(4), "d"),
        collection(&uid(5), "e"),
    ]);
    let (status, body) = server.send(server.push(ops)).await;
    assert_eq!(status, 400);
    assert_eq!(body["error"], "invalid");
    assert_eq!(body["index"], 2);
    // Auch ein unbekannter Operationstyp trägt seinen Index.
    let ops = json!([collection(&uid(1), "a"), { "type": "gibtsNicht" }]);
    let (status, body) = server.send(server.push(ops)).await;
    assert_eq!(status, 400);
    assert_eq!(body["index"], 1);
    assert!(server.states().await.is_empty());
}

#[tokio::test]
async fn s20_token_version_dataset_and_cursor_are_checked_without_effect() {
    let server = start().await;
    let ops = json!([collection(&uid(1), "a")]);

    let (status, body) = server
        .send(Request {
            token: "falsch",
            ..server.push(ops.clone())
        })
        .await;
    assert_eq!(
        (status, body["error"].clone()),
        (401, json!("unauthorized"))
    );

    let (status, body) = server
        .send(Request {
            protocol: "2".into(),
            ..server.push(ops.clone())
        })
        .await;
    assert_eq!((status, body["error"].clone()), (426, json!("protocol")));
    assert_eq!(body["protocol"], 1);

    let (status, body) = server
        .send(Request {
            dataset: "anderer",
            ..server.push(ops.clone())
        })
        .await;
    assert_eq!(status, 409);
    assert_eq!(body["error"], "datasetMismatch");
    assert_eq!(body["datasetId"], server.dataset);

    // Der Datensatz wird vor dem Parsen geprüft: auch mit kaputter Operation 409.
    let (status, body) = server
        .send(Request {
            dataset: "anderer",
            ..server.push(json!([collection("KEINE-UID", "x"), { "type": "gibtsNicht" }]))
        })
        .await;
    assert_eq!(
        (status, body["error"].clone()),
        (409, json!("datasetMismatch"))
    );

    let (status, _) = server
        .send(Request {
            dataset: "anderer",
            ..server.get("/v1/pull?since=0")
        })
        .await;
    assert_eq!(status, 409);
    assert!(
        server.states().await.is_empty(),
        "abgelehnte Pushes ohne Wirkung"
    );

    let (status, body) = server.send(server.get("/v1/pull?since=1")).await;
    assert_eq!((status, body["error"].clone()), (409, json!("cursorAhead")));
}

#[tokio::test]
async fn revoked_device_is_rejected_and_oversized_push_is_413() {
    let server = start().await;
    let conn = sync_server::db::open(&server.db_path).unwrap();
    assert!(sync_server::db::revoke_device(&conn, "A").unwrap());
    let (status, _) = server.send(server.get("/v1/pull?since=0")).await;
    assert_eq!(status, 401);

    let token = sync_server::db::add_device(&conn, "B").unwrap();
    let (status, body) = server
        .send(Request {
            token: &token,
            body: vec![b' '; MAX_PUSH_BODY_BYTES + 1],
            ..server.push(json!([]))
        })
        .await;
    assert_eq!((status, body["error"].clone()), (413, json!("tooLarge")));
}
