mod ai;
#[cfg(debug_assertions)]
mod automation;
mod commands;
mod models;
mod storage;
mod youtube;

use std::fs;
use std::io;

use storage::AppPaths;
use tauri::Manager;

const LOCALHOST_PORT: u16 = 14220;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_localhost::Builder::new(LOCALHOST_PORT).build())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_dir = app.path().app_data_dir().map_err(setup_error)?;
            fs::create_dir_all(&app_dir).map_err(setup_error)?;

            let paths = AppPaths {
                db_path: app_dir.join("videos.db"),
                config_path: app_dir.join("config.json"),
            };

            storage::init_db(&paths).map_err(setup_error)?;
            crate::commands::ensure_migrated(&paths);

            // AI state: config service + auth store (0600) + shared HTTP client for catalog/chat
            let ai_http: reqwest::Client = reqwest::Client::builder()
                .user_agent("Mozilla/5.0 YouTubeSummarizer/0.1")
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .map_err(setup_error)?;
            let ai_config = std::sync::Mutex::new(crate::ai::config::AiConfigService::load(&paths));
            let ai_auth = std::sync::Mutex::new(crate::ai::auth::AuthStore::load(&paths));
            app.manage(ai_http);
            app.manage(ai_config);
            app.manage(ai_auth);

            app.manage(paths);

            #[cfg(debug_assertions)]
            {
                let managed_paths = app.state::<AppPaths>().inner().clone();
                automation::start(managed_paths);
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // New AI commands (folio port)
            commands::ai_catalog_get,
            commands::ai_catalog_refresh,
            commands::ai_config_get,
            commands::ai_provider_enable,
            commands::ai_model_toggle,
            commands::ai_custom_upsert,
            commands::ai_custom_delete,
            commands::ai_custom_models_fetch,
            commands::ai_default_model_set,
            commands::ai_auth_set,
            commands::ai_auth_remove,
            commands::ai_auth_status,
            commands::ai_model_chat_test,
            commands::get_videos,
            commands::get_collections,
            commands::create_collection,
            commands::update_collection,
            commands::delete_collection,
            commands::set_video_collections,
            commands::get_video_detail,
            commands::add_video,
            commands::refresh_transcript,
            commands::summarize_video,
            commands::delete_video
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn setup_error(message: impl ToString) -> Box<dyn std::error::Error> {
    Box::new(io::Error::new(io::ErrorKind::Other, message.to_string()))
}
