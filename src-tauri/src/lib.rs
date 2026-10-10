mod auth;
mod commands;
#[cfg(desktop)]
mod desktop;
#[cfg(test)]
mod docker;
mod error;
mod fn_connect;
mod inventory;
mod logging;
mod proxy;
mod recovery;
mod resolver;
mod service_sync;
mod storage;
mod text;
mod types;
mod updates;
use std::sync::Arc;
use tauri::Manager;
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_opener::init());
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_autostart::Builder::new().build());
    let app = builder
        .setup(|app| {
            let path = app.path().app_config_dir()?.join("profile.json");
            let profile = storage::load_profiles(&path).unwrap_or_default();
            let state = Arc::new(commands::AppState::new(path, profile));
            let logging_result = app
                .path()
                .app_log_dir()
                .map_err(std::io::Error::other)
                .and_then(|directory| state.initialize_logs(directory));
            if let Err(error) = logging_result {
                eprintln!("FN Proxy could not initialize runtime log: {error}");
                state.log(
                    app.handle(),
                    "warn",
                    text::Text::new("logs.fileLoggingFailed"),
                );
            }
            state.log(app.handle(), "info", text::Text::new("logs.appStarted"));
            app.manage(state.clone());
            #[cfg(desktop)]
            desktop::setup(app)?;
            commands::auto_connect(app.handle().clone(), state);
            Ok(())
        })
        .on_window_event(|window, event| {
            #[cfg(desktop)]
            desktop::on_window_event(window, event);
        })
        .invoke_handler(tauri::generate_handler![
            updates::check_for_updates,
            commands::get_bootstrap,
            commands::set_allow_lan_access,
            commands::set_auto_start_proxy,
            commands::get_snapshot,
            commands::get_logs,
            commands::connect_nas,
            commands::disconnect_nas,
            commands::save_login,
            commands::forget_login,
            commands::discover_services,
            commands::get_service_inventory,
            commands::probe_service,
            commands::refresh_session,
            commands::start_proxy,
            commands::update_services,
            commands::stop_proxy,
            desktop::get_launch_at_login,
            desktop::set_launch_at_login,
            desktop::set_tray_labels,
            commands::remove_connection
        ])
        .build(tauri::generate_context!())
        .expect("FN Proxy startup failed");
    app.run(|handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            commands::shutdown(handle);
        }
    });
}
