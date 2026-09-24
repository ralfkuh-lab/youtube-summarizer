//! HTTP-Schicht: `/v1/health`, `/v1/push`, `/v1/pull` (Spec, Abschnitt
//! „Protokoll“). Alle Datenbankzugriffe laufen über eine Verbindung hinter
//! einem Mutex in `spawn_blocking`.

use crate::{db, merge, Error, Result};
use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, QueryRejection};
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex};
use sync_proto::{
    ErrorBody, HealthResponse, Op, PushResponse, HEADER_DATASET, HEADER_PROTOCOL,
    MAX_PUSH_BODY_BYTES, PROTOCOL_VERSION, PULL_PAGE_BYTES, PULL_PAGE_STATES,
};

type Db = Arc<Mutex<Connection>>;

/// Router des Sync-Servers über der Datenbank `db_path` (wird bei Bedarf angelegt).
pub fn app(db_path: &Path) -> Result<Router> {
    let db: Db = Arc::new(Mutex::new(db::open(db_path)?));
    Ok(Router::new()
        .route("/v1/health", get(health))
        .route("/v1/push", post(push))
        .route("/v1/pull", get(pull))
        .layer(DefaultBodyLimit::max(MAX_PUSH_BODY_BYTES))
        .with_state(db))
}

/// Bedient `app` auf `listener`, bis `shutdown` fertig ist.
pub async fn serve(
    listener: tokio::net::TcpListener,
    app: Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
}

async fn with_db<T: Send + 'static>(
    db: Db,
    work: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(move || {
        // Eine Panik in einer Transaktion hat sie bereits zurückgerollt.
        let mut conn = db.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        work(&mut conn)
    })
    .await
    .map_err(|err| Error::Io(std::io::Error::other(err)))?
}

struct Caller {
    token: String,
    dataset: String,
}

/// Protokollversion und Token aus den Kopfzeilen; das Token wird erst in der
/// Datenbank geprüft.
fn caller(headers: &HeaderMap) -> Result<Caller> {
    let text = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
    };
    if text(HEADER_PROTOCOL) != PROTOCOL_VERSION.to_string() {
        return Err(Error::Protocol);
    }
    let token = text(AUTHORIZATION.as_str())
        .strip_prefix("Bearer ")
        .ok_or(Error::Unauthorized)?;
    Ok(Caller {
        token: token.to_owned(),
        dataset: text(HEADER_DATASET).to_owned(),
    })
}

async fn health(State(db): State<Db>) -> Response {
    let result = with_db(db, |conn| db::dataset_id(conn)).await;
    respond(result.map(|dataset_id| HealthResponse {
        status: "ok".into(),
        protocol: PROTOCOL_VERSION,
        dataset_id,
    }))
}

#[derive(Deserialize)]
struct RawPush {
    ops: Vec<Value>,
}

/// Parst den Body so, dass ein Fehler in einer Operation deren Index trägt.
fn parse_ops(body: &[u8]) -> Result<Vec<Op>> {
    let raw: RawPush =
        serde_json::from_slice(body).map_err(|err| Error::Malformed(err.to_string()))?;
    raw.ops
        .into_iter()
        .enumerate()
        .map(|(index, op)| {
            serde_json::from_value(op).map_err(|err| Error::Invalid(index, err.to_string()))
        })
        .collect()
}

async fn push(
    State(db): State<Db>,
    headers: HeaderMap,
    body: std::result::Result<Bytes, BytesRejection>,
) -> Response {
    let result = async {
        let caller = caller(&headers)?;
        let body = body.map_err(|rejection| {
            if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                Error::TooLarge
            } else {
                Error::Malformed(rejection.body_text())
            }
        })?;
        with_db(db, move |conn| {
            let device = db::authenticate(conn, &caller.token)?;
            merge::push_parsed(conn, device, &caller.dataset, || parse_ops(&body))
        })
        .await
    }
    .await;
    respond(result.map(|results| PushResponse { results }))
}

#[derive(Deserialize)]
struct PullQuery {
    since: i64,
}

async fn pull(
    State(db): State<Db>,
    headers: HeaderMap,
    query: std::result::Result<Query<PullQuery>, QueryRejection>,
) -> Response {
    let result = async {
        let caller = caller(&headers)?;
        let since = query
            .map_err(|rejection| Error::Malformed(rejection.body_text()))
            .and_then(|Query(query)| match query.since {
                since if since >= 0 => Ok(since),
                _ => Err(Error::Malformed("since ist negativ".into())),
            });
        with_db(db, move |conn| {
            db::authenticate(conn, &caller.token)?;
            db::pull_page(
                conn,
                &caller.dataset,
                since?,
                PULL_PAGE_STATES,
                PULL_PAGE_BYTES,
            )
        })
        .await
    }
    .await;
    respond(result)
}

fn respond<T: Serialize>(result: Result<T>) -> Response {
    match result {
        Ok(body) => Json(body).into_response(),
        Err(err) => error_response(err),
    }
}

fn error_response(err: Error) -> Response {
    let mut body = ErrorBody {
        error: String::new(),
        message: None,
        dataset_id: None,
        index: None,
        protocol: None,
    };
    let (status, error) = match err {
        Error::Invalid(index, message) => {
            body.index = Some(index);
            body.message = Some(message);
            (StatusCode::BAD_REQUEST, "invalid")
        }
        Error::Malformed(message) => {
            body.message = Some(message);
            (StatusCode::BAD_REQUEST, "invalid")
        }
        Error::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
        Error::Protocol => {
            body.protocol = Some(PROTOCOL_VERSION);
            (StatusCode::UPGRADE_REQUIRED, "protocol")
        }
        Error::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "tooLarge"),
        Error::DatasetMismatch(id) => {
            body.dataset_id = Some(id);
            (StatusCode::CONFLICT, "datasetMismatch")
        }
        Error::CursorAhead => (StatusCode::CONFLICT, "cursorAhead"),
        other => {
            eprintln!("Interner Fehler: {other}");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
    };
    body.error = error.into();
    (status, Json(body)).into_response()
}
