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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let app_dir = app.path().app_data_dir().map_err(setup_error)?;
            fs::create_dir_all(&app_dir).map_err(setup_error)?;

            let paths = AppPaths {
                db_path: app_dir.join("videos.db"),
                config_path: app_dir.join("config.json"),
            };

            storage::init_db(&paths).map_err(setup_error)?;
            storage::load_config(&paths).map_err(setup_error)?;
            app.manage(paths);

            #[cfg(debug_assertions)]
            {
                let managed_paths = app.state::<AppPaths>().inner().clone();
                automation::start(managed_paths);
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::get_ai_providers,
            commands::save_config,
            commands::save_provider_config,
            commands::add_custom_provider,
            commands::delete_custom_provider,
            commands::refresh_provider_models,
            commands::get_videos,
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
