mod ai_config;
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

/// Pick a free localhost port at runtime.
///
/// A fixed port is fragile: on some systems (e.g. Windows with Hyper-V) the
/// desired port can fall into a reserved exclusion range and binding fails
/// with a permission error. Binding to port 0 lets the OS hand us a free port.
///
/// We bind `"localhost"` (not `127.0.0.1`) so the probe uses the same address
/// family that `tauri-plugin-localhost` later binds via `Server::http`.
fn pick_localhost_port() -> u16 {
    std::net::TcpListener::bind(("localhost", 0))
        .and_then(|listener| listener.local_addr())
        .map(|addr| addr.port())
        .unwrap_or(LOCALHOST_PORT)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let port = pick_localhost_port();

    tauri::Builder::default()
        .plugin(tauri_plugin_localhost::Builder::new(port).build())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let url = format!("http://localhost:{port}/index.html")
                .parse()
                .map_err(setup_error)?;
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External(url))
                .title("YouTube Summarizer")
                .inner_size(1200.0, 760.0)
                .min_inner_size(900.0, 560.0)
                .resizable(true)
                .build()?;

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
            commands::test_provider_model_chat,
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
