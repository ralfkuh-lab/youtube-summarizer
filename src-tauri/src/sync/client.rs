//! HTTP zum Sync-Server (docs/spec-sync.md, „Protokoll“): ohne
//! Weiterleitungen, 60 s Timeout. Fehlertexte enthalten nie das Token.

use std::time::Duration;

use reqwest::{RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use sync_proto::{
    ErrorBody, HealthResponse, Op, PullResponse, PushRequest, PushResponse, HEADER_DATASET,
    HEADER_PROTOCOL, PROTOCOL_VERSION,
};

use super::config::SyncConfig;

/// Warum der Sync angehalten hat (`sync_status.stopped`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    Auth,
    Dataset,
    Version,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Failure {
    /// 401, 409, 426: bis zu einer Einstellungsänderung bzw. „Neu abgleichen“.
    Stopped(StopReason, String),
    /// 413 für den ganzen Block.
    TooLarge,
    /// 400; `index` ist die ungültige Operation, falls der Server sie nennt.
    Invalid(Option<usize>, String),
    /// Netz, 5xx, Unerwartetes: Backoff, Outbox unverändert.
    Transient(String),
    /// Eine Löschung ist nicht sendbar: Der Lauf endet ohne Upserts und Pull
    /// (kein Backoff; eine neue Version des Eintrags hebt es auf).
    Blocked(String),
}

impl Failure {
    pub fn message(&self) -> String {
        match self {
            Failure::Stopped(_, message)
            | Failure::Transient(message)
            | Failure::Blocked(message) => message.clone(),
            Failure::TooLarge => "Anfrage zu groß".to_string(),
            Failure::Invalid(_, message) => format!("Server lehnt Anfrage ab: {message}"),
        }
    }
}

pub fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(60))
        .user_agent("YouTubeSummarizer-Sync/0.1")
        .build()
        .map_err(|err| format!("HTTP-Client konnte nicht erstellt werden: {err}"))
}

pub async fn health(
    http: &reqwest::Client,
    config: &SyncConfig,
) -> Result<HealthResponse, Failure> {
    let response: HealthResponse =
        send(http.get(format!("{}/v1/health", config.server_url))).await?;
    if response.protocol != PROTOCOL_VERSION {
        return Err(Failure::Stopped(
            StopReason::Version,
            format!(
                "Server spricht Protokoll {}, die App {PROTOCOL_VERSION} – App bzw. Server aktualisieren",
                response.protocol
            ),
        ));
    }
    Ok(response)
}

pub async fn push(
    http: &reqwest::Client,
    config: &SyncConfig,
    dataset: &str,
    ops: Vec<Op>,
) -> Result<PushResponse, Failure> {
    let expected = ops.len();
    let response: PushResponse = send(
        authorized(
            http.post(format!("{}/v1/push", config.server_url)),
            config,
            dataset,
        )
        .json(&PushRequest { ops }),
    )
    .await?;
    if response.results.len() != expected {
        return Err(Failure::Transient(format!(
            "Push-Antwort mit {} statt {expected} Ergebnissen",
            response.results.len()
        )));
    }
    Ok(response)
}

pub async fn pull(
    http: &reqwest::Client,
    config: &SyncConfig,
    dataset: &str,
    since: i64,
) -> Result<PullResponse, Failure> {
    send(authorized(
        http.get(format!("{}/v1/pull", config.server_url))
            .query(&[("since", since)]),
        config,
        dataset,
    ))
    .await
}

fn authorized(builder: RequestBuilder, config: &SyncConfig, dataset: &str) -> RequestBuilder {
    builder
        .header(HEADER_PROTOCOL, PROTOCOL_VERSION.to_string())
        .header(HEADER_DATASET, dataset)
        .bearer_auth(&config.token)
}

async fn send<T: DeserializeOwned>(builder: RequestBuilder) -> Result<T, Failure> {
    let response = builder.send().await.map_err(|err| {
        Failure::Transient(format!("Server nicht erreichbar: {}", err.without_url()))
    })?;
    let status = response.status();
    if status.is_success() {
        return response
            .json()
            .await
            .map_err(|err| Failure::Transient(format!("Antwort unlesbar: {}", err.without_url())));
    }
    let body: Option<ErrorBody> = response.json().await.ok();
    let error = body.as_ref().map(|body| body.error.as_str()).unwrap_or("");
    Err(match status {
        StatusCode::UNAUTHORIZED => {
            Failure::Stopped(StopReason::Auth, "Token vom Server abgelehnt".to_string())
        }
        StatusCode::CONFLICT if error == "datasetMismatch" || error == "cursorAhead" => {
            Failure::Stopped(
                StopReason::Dataset,
                "Der Server hat einen anderen Datenbestand – „Neu abgleichen“ nötig".to_string(),
            )
        }
        StatusCode::UPGRADE_REQUIRED => Failure::Stopped(
            StopReason::Version,
            "Protokoll passt nicht – App bzw. Server aktualisieren".to_string(),
        ),
        StatusCode::PAYLOAD_TOO_LARGE => Failure::TooLarge,
        StatusCode::BAD_REQUEST => Failure::Invalid(
            body.as_ref().and_then(|body| body.index),
            body.and_then(|body| body.message)
                .unwrap_or_else(|| "ungültige Anfrage".to_string()),
        ),
        status => Failure::Transient(format!("Unerwartete Server-Antwort: HTTP {status}")),
    })
}
