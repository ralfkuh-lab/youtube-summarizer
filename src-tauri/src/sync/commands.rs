//! Tauri-Befehle des Syncs (docs/spec-sync.md, „Auslöser und Status“).

use std::sync::Arc;

use tauri::State;

use super::config::{self, SyncConfigInput, SyncConfigView};
use super::engine::{SyncEngine, SyncStatus, SyncTestResult};
use crate::models::Video;
use crate::storage::{self, AppPaths, AppResult};

type Engine = Arc<SyncEngine>;

#[tauri::command]
pub fn sync_config_get(paths: State<'_, AppPaths>) -> SyncConfigView {
    SyncConfigView::from(&config::load(&paths))
}

#[tauri::command]
pub async fn sync_config_set(
    engine: State<'_, Engine>,
    config: SyncConfigInput,
) -> AppResult<SyncConfigView> {
    engine.set_config(&config).await
}

#[tauri::command]
pub async fn sync_test(
    engine: State<'_, Engine>,
    config: SyncConfigInput,
) -> AppResult<SyncTestResult> {
    engine.test_connection(&config).await
}

#[tauri::command]
pub async fn sync_now(engine: State<'_, Engine>) -> AppResult<SyncStatus> {
    Ok(engine.sync_now().await)
}

#[tauri::command]
pub async fn sync_status(engine: State<'_, Engine>) -> AppResult<SyncStatus> {
    Ok(engine.current_status().await)
}

#[tauri::command]
pub async fn sync_rebaseline(engine: State<'_, Engine>) -> AppResult<SyncStatus> {
    engine.rebaseline().await
}

#[tauri::command]
pub async fn video_set_local_only(
    paths: State<'_, AppPaths>,
    engine: State<'_, Engine>,
    video_id: i64,
    local_only: bool,
) -> AppResult<Video> {
    let paths = paths.inner().clone();
    let video = tokio::task::spawn_blocking(move || {
        storage::video_set_local_only(&paths, video_id, local_only)?;
        storage::get_video(&paths, video_id)?.ok_or_else(|| "Video nicht gefunden".to_string())
    })
    .await
    .map_err(|err| format!("Video konnte nicht umgeschaltet werden: {err}"))??;
    engine.publish_status().await;
    Ok(video)
}
