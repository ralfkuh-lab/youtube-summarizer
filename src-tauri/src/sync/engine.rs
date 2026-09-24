//! Sync-Engine (docs/spec-sync.md, „Client-Sync-Engine“): Datensatz-Bindung,
//! Push in zwei Phasen, Pull-Staging, Apply, Status, Backoff und die
//! Hintergrundschleife. Ein Lauf, eine Einstellungsänderung und „Neu
//! abgleichen“ nehmen denselben Mutex. DB-Arbeit läuft in `spawn_blocking`.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::{params, Connection};
use serde::Serialize;
use sync_proto::{
    now_canonical, Op, OpStatus, MAX_OPS_PER_PUSH, MAX_OP_BYTES, MAX_PUSH_BODY_BYTES,
};

use super::apply::{self, Applied};
use super::client::{self, Failure, StopReason};
use super::config::{self, SyncConfig, SyncConfigInput, SyncConfigView};
use super::snapshot::{self, Pending};
use super::{db_error, pull, state_get, state_set};
use crate::storage::{self, AppPaths, AppResult};

const PUSH_INTERVAL: Duration = Duration::from_secs(15);
const FULL_INTERVAL: Duration = Duration::from_secs(120);
const BACKOFF_START: Duration = Duration::from_secs(15);
const BACKOFF_MAX: Duration = Duration::from_secs(600);
/// `{"ops":[]}`
const PUSH_BODY_OVERHEAD: usize = 10;
pub(crate) const BLOCKED_DELETE: &str =
    "Löschung nicht sendbar – Synchronisation angehalten – ‚Jetzt synchronisieren‘ versucht es erneut";

/// Ergebnis einer Push-Phase.
#[derive(Default)]
struct PhaseOutcome {
    /// Mindestens eine Operation bekam `retry`.
    retried: bool,
    /// Mindestens eine Operation wurde `unsendable`.
    unsendable: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub enabled: bool,
    pub running: bool,
    pub last_success_at: Option<String>,
    pub last_error: Option<String>,
    pub pending: i64,
    pub unsendable: Vec<Unsendable>,
    pub stopped: Option<StopReason>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unsendable {
    pub entity: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncTestResult {
    pub dataset_id: String,
    /// `None`: noch nie gebunden; sonst ob die ID zur gespeicherten passt.
    pub same_dataset: Option<bool>,
}

pub enum SyncEvent {
    Status(SyncStatus),
    Applied(Applied),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Full,
    PushOnly,
}

#[derive(Default)]
struct RunState {
    /// URL, für die die Datensatz-ID seit Start bzw. Einstellungsänderung
    /// geprüft ist.
    bound_url: Option<String>,
}

#[derive(Default)]
struct Backoff {
    delay: Option<Duration>,
    until: Option<Instant>,
}

pub struct SyncEngine {
    paths: AppPaths,
    http: reqwest::Client,
    run: tokio::sync::Mutex<RunState>,
    status: Mutex<SyncStatus>,
    backoff: Mutex<Backoff>,
    events: Box<dyn Fn(SyncEvent) + Send + Sync>,
}

impl SyncEngine {
    pub fn new(paths: AppPaths, events: Box<dyn Fn(SyncEvent) + Send + Sync>) -> AppResult<Self> {
        let enabled = config::load(&paths).is_active();
        Ok(Self {
            paths,
            http: client::http_client()?,
            run: tokio::sync::Mutex::new(RunState::default()),
            status: Mutex::new(SyncStatus {
                enabled,
                ..SyncStatus::default()
            }),
            backoff: Mutex::new(Backoff::default()),
            events,
        })
    }

    pub fn status(&self) -> SyncStatus {
        lock(&self.status).clone()
    }

    fn update(&self, change: impl FnOnce(&mut SyncStatus)) {
        let snapshot = {
            let mut status = lock(&self.status);
            let before = status.clone();
            change(&mut status);
            (*status != before).then(|| status.clone())
        };
        if let Some(status) = snapshot {
            (self.events)(SyncEvent::Status(status));
        }
    }

    async fn db<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Connection) -> AppResult<T> + Send + 'static,
    ) -> Result<T, Failure> {
        let paths = self.paths.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = storage::open_db(&paths)?;
            work(&mut conn)
        })
        .await
        .map_err(|err| format!("Sync-Aufgabe abgebrochen: {err}"))
        .and_then(|result| result)
        .map_err(Failure::Transient)
    }

    /// Ein Lauf: Datensatz-Bindung, Push, bei `Full` Pull und Apply. Ohne
    /// aktive Einstellungen oder bei angehaltenem Sync kein Netzwerkzugriff.
    pub async fn run(&self, mode: Mode) -> SyncStatus {
        self.run_locked(mode, false).await
    }

    /// „Jetzt synchronisieren“: wie ein voller Lauf, hebt aber vorher alle
    /// `unsendable`-Markierungen auf (bewusster Klick). Was weiterhin nicht
    /// sendbar ist, wird im Lauf neu markiert. Die Schleife tut das nie.
    pub async fn sync_now(&self) -> SyncStatus {
        self.run_locked(Mode::Full, true).await
    }

    async fn run_locked(&self, mode: Mode, retry_unsendable: bool) -> SyncStatus {
        let mut state = self.run.lock().await;
        let config = config::load(&self.paths);
        if !config.is_active() {
            self.update(|status| {
                status.enabled = false;
                status.running = false;
            });
            return self.status();
        }
        if self.status().stopped.is_some() {
            self.update(|status| status.enabled = true);
            return self.status();
        }
        self.update(|status| {
            status.enabled = true;
            status.running = true;
        });
        let cleared = if retry_unsendable {
            self.db(snapshot::clear_unsendable).await.map(|_| ())
        } else {
            Ok(())
        };
        // Der Lauf-Mutex bleibt bis nach Status und Apply-Event gehalten,
        // damit `running` nie von einem überlappenden Lauf überschrieben wird.
        let outcome = match cleared {
            Ok(()) => self.run_inner(&mut state, &config, mode).await,
            Err(failure) => Err(failure),
        };
        let applied = match outcome {
            Ok(applied) => {
                *lock(&self.backoff) = Backoff::default();
                self.update(|status| {
                    status.last_success_at = Some(now_canonical());
                    status.last_error = None;
                });
                applied
            }
            Err(Failure::Stopped(reason, message)) => {
                self.update(|status| {
                    status.stopped = Some(reason);
                    status.last_error = Some(message);
                });
                None
            }
            Err(Failure::Blocked(message)) => {
                self.update(|status| status.last_error = Some(message));
                None
            }
            Err(failure) => {
                let mut backoff = lock(&self.backoff);
                let delay = backoff
                    .delay
                    .map_or(BACKOFF_START, |delay| (delay * 2).min(BACKOFF_MAX));
                backoff.delay = Some(delay);
                backoff.until = Some(Instant::now() + delay);
                drop(backoff);
                self.update(|status| status.last_error = Some(failure.message()));
                None
            }
        };
        self.refresh_counts().await;
        self.update(|status| status.running = false);
        if let Some(applied) = applied {
            if applied.collections || !applied.video_ids.is_empty() {
                (self.events)(SyncEvent::Applied(applied));
            }
        }
        drop(state);
        self.status()
    }

    async fn run_inner(
        &self,
        state: &mut RunState,
        config: &SyncConfig,
        mode: Mode,
    ) -> Result<Option<Applied>, Failure> {
        let dataset = self.bind_dataset(state, config).await?;
        let complete = self.push_outbox(config, &dataset).await?;
        if !complete || mode == Mode::PushOnly {
            return Ok(None);
        }
        self.pull_all(config, &dataset).await?;
        Ok(Some(self.db(apply::apply).await?))
    }

    /// Datensatz-Bindung vor dem ersten Push/Pull nach Start oder
    /// Einstellungsänderung. Gibt die zu sendende Datensatz-ID zurück.
    async fn bind_dataset(
        &self,
        state: &mut RunState,
        config: &SyncConfig,
    ) -> Result<String, Failure> {
        if state.bound_url.as_deref() != Some(config.server_url.as_str()) {
            let server = client::health(&self.http, config).await?.dataset_id;
            let bound = self
                .db(move |conn| {
                    let stored = state_get(conn, "dataset_id")?;
                    match stored {
                        Some(stored) => Ok(stored == server),
                        None if never_synced(conn)? => {
                            state_set(conn, "dataset_id", &server)?;
                            Ok(true)
                        }
                        None => Ok(false),
                    }
                })
                .await?;
            if !bound {
                return Err(Failure::Stopped(
                    StopReason::Dataset,
                    "Der Server hat einen anderen Datenbestand – „Neu abgleichen“ nötig"
                        .to_string(),
                ));
            }
            state.bound_url = Some(config.server_url.clone());
        }
        self.db(|conn| {
            state_get(conn, "dataset_id")?.ok_or_else(|| "Datensatz-ID fehlt".to_string())
        })
        .await
    }

    /// Beide Phasen aus einem Snapshot. Phase 2 läuft nur, wenn alle
    /// Löschungen quittiert sind: Eine nicht sendbare Löschung (jetzt oder aus
    /// einem früheren Lauf) beendet den Lauf mit [`BLOCKED_DELETE`], ein
    /// `retry` mit `false` – jeweils ohne Upserts und ohne Pull.
    async fn push_outbox(&self, config: &SyncConfig, dataset: &str) -> Result<bool, Failure> {
        let snapshot = self.db(snapshot::snapshot).await?;
        let deletes = self.send_phase(config, dataset, snapshot.deletes).await?;
        if deletes.unsendable || snapshot.blocked_deletes > 0 {
            return Err(Failure::Blocked(BLOCKED_DELETE.to_string()));
        }
        if deletes.retried {
            return Ok(false);
        }
        self.send_phase(config, dataset, snapshot.upserts).await?;
        Ok(true)
    }

    /// Packt, sendet und quittiert blockweise. 413 halbiert den Block, eine
    /// einzelne Operation wird `unsendable`; 400 mit Index markiert diese
    /// Operation und sendet den Rest weiter.
    async fn send_phase(
        &self,
        config: &SyncConfig,
        dataset: &str,
        ops: Vec<Pending>,
    ) -> Result<PhaseOutcome, Failure> {
        let (blocks, oversized) = pack(ops, MAX_OPS_PER_PUSH, MAX_PUSH_BODY_BYTES, MAX_OP_BYTES);
        let mut outcome = PhaseOutcome {
            unsendable: !oversized.is_empty(),
            ..PhaseOutcome::default()
        };
        for (seq, reason) in oversized {
            self.mark_unsendable(seq, reason).await?;
        }
        let mut queue: VecDeque<Vec<Pending>> = blocks.into();
        while let Some(mut block) = queue.pop_front() {
            let ops: Vec<Op> = block.iter().map(|pending| pending.op.clone()).collect();
            match send(&self.http, config, dataset, ops).await {
                Ok(results) => {
                    self.ensure_config(config)?;
                    outcome.retried |= results
                        .iter()
                        .any(|result| result.status == OpStatus::Retry);
                    self.db(move |conn| snapshot::acknowledge(conn, &block, &results))
                        .await?;
                }
                Err(Failure::TooLarge) if block.len() > 1 => {
                    let rest = block.split_off(block.len() / 2);
                    queue.push_front(rest);
                    queue.push_front(block);
                }
                Err(Failure::TooLarge) => {
                    outcome.unsendable = true;
                    self.mark_unsendable(block[0].seq, "vom Server als zu groß abgelehnt".into())
                        .await?;
                }
                Err(Failure::Invalid(Some(index), message)) if index < block.len() => {
                    outcome.unsendable = true;
                    let invalid = block.remove(index);
                    self.mark_unsendable(invalid.seq, message).await?;
                    if !block.is_empty() {
                        queue.push_front(block);
                    }
                }
                Err(failure) => return Err(failure),
            }
        }
        Ok(outcome)
    }

    async fn mark_unsendable(&self, seq: i64, reason: String) -> Result<(), Failure> {
        self.db(move |conn| snapshot::mark_unsendable(conn, seq, &reason))
            .await
    }

    /// Seiten ab `cursor` bis `more = false` in die Inbox.
    async fn pull_all(&self, config: &SyncConfig, dataset: &str) -> Result<(), Failure> {
        loop {
            let since = self.db(|conn| pull::cursor(conn)).await?;
            let page = pull_page(&self.http, config, dataset, since).await?;
            self.ensure_config(config)?;
            let more = page.more;
            let stalled = more && page.next <= since;
            self.db(move |conn| pull::stage_page(conn, &page)).await?;
            if !more {
                return Ok(());
            }
            if stalled {
                return Err(Failure::Transient("Pull kommt nicht voran".to_string()));
            }
        }
    }

    /// Antworten zählen nur für die Konfiguration des gestarteten Laufs.
    fn ensure_config(&self, started: &SyncConfig) -> Result<(), Failure> {
        let current = config::load(&self.paths);
        if current.server_url == started.server_url && current.token == started.token {
            Ok(())
        } else {
            Err(Failure::Transient(
                "Einstellungen während des Laufs geändert".to_string(),
            ))
        }
    }

    /// Status mit `pending` und `unsendable` frisch aus der DB (`sync_status`).
    pub async fn current_status(&self) -> SyncStatus {
        self.refresh_counts().await;
        self.status()
    }

    /// Nach lokalen Änderungen an der Outbox außerhalb eines Laufs (z. B.
    /// Privatschalten): Zähler neu lesen und `sync://status` senden.
    pub async fn publish_status(&self) -> SyncStatus {
        self.refresh_counts().await;
        let status = self.status();
        (self.events)(SyncEvent::Status(status.clone()));
        status
    }

    async fn refresh_counts(&self) {
        let counts = self
            .db(|conn| {
                let pending: i64 = conn
                    .query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0))
                    .map_err(db_error)?;
                let mut stmt = conn
                    .prepare(
                        "SELECT entity, unsendable FROM sync_outbox \
                         WHERE unsendable IS NOT NULL ORDER BY seq",
                    )
                    .map_err(db_error)?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok(Unsendable {
                            entity: row.get(0)?,
                            reason: row.get(1)?,
                        })
                    })
                    .map_err(db_error)?;
                let unsendable = rows.collect::<Result<Vec<_>, _>>().map_err(db_error)?;
                Ok((pending, unsendable))
            })
            .await;
        if let Ok((pending, unsendable)) = counts {
            self.update(|status| {
                status.pending = pending;
                status.unsendable = unsendable;
            });
        }
    }

    /// Einstellungen speichern (unter dem Lauf-Mutex); hebt ein Anhalten und
    /// den Backoff auf, die Datensatz-ID wird beim nächsten Lauf neu geprüft.
    pub async fn set_config(&self, input: &SyncConfigInput) -> AppResult<SyncConfigView> {
        let mut state = self.run.lock().await;
        let saved = config::save(&self.paths, input)?;
        // Neue Einstellungen: nicht sendbare Einträge beim nächsten Lauf
        // erneut versuchen.
        self.db(snapshot::clear_unsendable)
            .await
            .map_err(|failure| failure.message())?;
        state.bound_url = None;
        *lock(&self.backoff) = Backoff::default();
        self.update(|status| {
            status.enabled = saved.is_active();
            status.stopped = None;
            status.last_error = None;
        });
        Ok(SyncConfigView::from(&saved))
    }

    /// „Verbindung testen“ mit den eingegebenen Werten (leeres Token = das
    /// gespeicherte): Health und ein leerer Push prüfen Erreichbarkeit,
    /// Protokoll und Token, ohne etwas zu ändern.
    pub async fn test_connection(&self, input: &SyncConfigInput) -> AppResult<SyncTestResult> {
        let stored = config::load(&self.paths);
        let token = match input.token.as_deref().map(str::trim) {
            Some(token) if !token.is_empty() => token.to_string(),
            _ => stored.token,
        };
        let config = SyncConfig {
            enabled: true,
            server_url: config::normalize_url(&input.server_url)?,
            token,
            new_videos_local: false,
        };
        if !config.is_active() {
            return Err("Server-URL und Token eingeben".to_string());
        }
        let health = client::health(&self.http, &config)
            .await
            .map_err(|failure| failure.message())?;
        client::push(&self.http, &config, &health.dataset_id, Vec::new())
            .await
            .map_err(|failure| failure.message())?;
        let stored = self
            .db(|conn| state_get(conn, "dataset_id"))
            .await
            .map_err(|failure| failure.message())?;
        Ok(SyncTestResult {
            same_dataset: stored.map(|stored| stored == health.dataset_id),
            dataset_id: health.dataset_id,
        })
    }

    /// „Neu abgleichen“ unter dem Lauf-Mutex: neue Datensatz-ID vom Server,
    /// dann `pull::rebaseline` in einer Transaktion.
    pub async fn rebaseline(&self) -> AppResult<SyncStatus> {
        let mut state = self.run.lock().await;
        let config = config::load(&self.paths);
        if !config.is_active() {
            return Err("Synchronisation ist nicht eingerichtet".to_string());
        }
        let dataset = client::health(&self.http, &config)
            .await
            .map_err(|failure| failure.message())?
            .dataset_id;
        self.db(move |conn| pull::rebaseline(conn, &dataset))
            .await
            .map_err(|failure| failure.message())?;
        state.bound_url = Some(config.server_url.clone());
        drop(state);
        *lock(&self.backoff) = Backoff::default();
        self.update(|status| {
            status.stopped = None;
            status.last_error = None;
        });
        self.refresh_counts().await;
        Ok(self.status())
    }

    /// Ein Takt der Hintergrundschleife: voller Lauf alle 120 s, sonst Push,
    /// falls die Outbox nicht leer ist. Kein Netz ohne aktive Einstellungen,
    /// bei angehaltenem Sync oder während des Backoffs.
    pub async fn tick(&self, last_full: &mut Option<Instant>) {
        if !config::load(&self.paths).is_active() {
            self.update(|status| status.enabled = false);
            return;
        }
        if self.status().stopped.is_some() {
            return;
        }
        if lock(&self.backoff)
            .until
            .is_some_and(|until| Instant::now() < until)
        {
            return;
        }
        if last_full.is_none_or(|last| last.elapsed() >= FULL_INTERVAL) {
            *last_full = Some(Instant::now());
            self.run(Mode::Full).await;
        } else if self
            .db(|conn| {
                conn.query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(db_error)
            })
            .await
            .is_ok_and(|pending| pending > 0)
        {
            self.run(Mode::PushOnly).await;
        }
    }

    pub async fn background_loop(engine: Arc<Self>) {
        let mut last_full = None;
        loop {
            engine.tick(&mut last_full).await;
            tokio::time::sleep(PUSH_INTERVAL).await;
        }
    }
}

/// Nie synchronisiert: `cursor = 0` und kein Video veröffentlicht.
fn never_synced(conn: &Connection) -> AppResult<bool> {
    let published: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM videos WHERE published = 1)",
            params![],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    Ok(pull::cursor(conn)? == 0 && !published)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Teilt eine Phase in Blöcke nach Anzahl und Body-Bytes. Operationen über
/// `max_op_bytes` kommen mit Grund zurück (werden `unsendable`).
pub(crate) fn pack(
    ops: Vec<Pending>,
    max_ops: usize,
    max_body_bytes: usize,
    max_op_bytes: usize,
) -> (Vec<Vec<Pending>>, Vec<(i64, String)>) {
    let mut blocks = Vec::new();
    let mut oversized = Vec::new();
    let mut current: Vec<Pending> = Vec::new();
    let mut bytes = PUSH_BODY_OVERHEAD;
    for pending in ops {
        let size = serde_json::to_vec(&pending.op).map_or(usize::MAX, |json| json.len());
        if size > max_op_bytes {
            oversized.push((
                pending.seq,
                format!("zu groß ({} MiB)", size.div_ceil(1024 * 1024)),
            ));
            continue;
        }
        let separator = usize::from(!current.is_empty());
        if !current.is_empty()
            && (current.len() == max_ops || bytes + separator + size > max_body_bytes)
        {
            blocks.push(std::mem::take(&mut current));
            bytes = PUSH_BODY_OVERHEAD;
        }
        bytes += usize::from(!current.is_empty()) + size;
        current.push(pending);
    }
    if !current.is_empty() {
        blocks.push(current);
    }
    (blocks, oversized)
}

/// Sendet einen Block (Zerlegung für Tests, die Antworten verwerfen).
pub(crate) async fn send(
    http: &reqwest::Client,
    config: &SyncConfig,
    dataset: &str,
    ops: Vec<Op>,
) -> Result<Vec<sync_proto::OpResult>, Failure> {
    Ok(client::push(http, config, dataset, ops).await?.results)
}

/// Holt eine Pull-Seite.
pub(crate) async fn pull_page(
    http: &reqwest::Client,
    config: &SyncConfig,
    dataset: &str,
    since: i64,
) -> Result<sync_proto::PullResponse, Failure> {
    client::pull(http, config, dataset, since).await
}
