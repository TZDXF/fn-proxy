mod auth;
mod commands;
#[cfg(desktop)]
mod desktop;
mod docker;
mod error;
mod inventory;
mod proxy;
mod resolver;
mod service_sync;
mod storage;
mod types;
use std::sync::Arc;
use tauri::Manager;
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let path = app.path().app_config_dir()?.join("profile.json");
            let profile = storage::load_profiles(&path).unwrap_or_default();
            let state = Arc::new(commands::AppState::new(path, profile));
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
            commands::get_bootstrap,
            commands::set_allow_lan_access,
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
