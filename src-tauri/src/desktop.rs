use crate::text::Text;
use serde::Deserialize;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    App, AppHandle, Manager, State, Window, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt;

const TRAY_ID: &str = "main-tray";
const MAIN_WINDOW: &str = "main";

/// Tray labels arrive from the frontend so menu text follows the UI locale
/// and every translation stays in `src/locales/*.json`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayLabels {
    show: String,
    quit: String,
    tooltip: String,
    #[serde(default)]
    locale: String,
}

pub struct TrayHandles {
    show: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
    tray: TrayIcon<tauri::Wry>,
}

#[tauri::command]
pub fn get_launch_at_login(app: AppHandle) -> Result<bool, Text> {
    app.autolaunch()
        .is_enabled()
        .map_err(|_| Text::new("settings.launchAtLoginReadFailed"))
}

#[tauri::command]
pub fn set_launch_at_login(app: AppHandle, enabled: bool) -> Result<bool, Text> {
    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    result.map_err(|_| Text::new("settings.launchAtLoginSaveFailed"))?;
    get_launch_at_login(app)
}

#[tauri::command]
pub fn set_tray_labels(app: AppHandle, handles: State<'_, TrayHandles>, labels: TrayLabels) {
    crate::notifications::set_locale(&app, &labels.locale);
    // A rejected label update must never break the app; the tray just keeps
    // its previous language until the next successful sync.
    let _ = handles.show.set_text(labels.show);
    let _ = handles.quit.set_text(labels.quit);
    let _ = handles.tray.set_tooltip(Some(labels.tooltip));
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let result = window
            .show()
            .and_then(|()| window.unminimize())
            .and_then(|()| window.set_focus());
        if let Err(error) = result {
            eprintln!("Failed to restore FN Proxy window: {error}");
        }
    }
}

pub fn setup(app: &App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show-main", "FN Proxy", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &separator, &quit])?;
    let icon = app
        .default_window_icon()
        .expect("FN Proxy must have an application icon")
        .clone();

    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("FN Proxy")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show-main" => show_main_window(app),
            // AppHandle::exit triggers RunEvent::Exit, preserving proxy cleanup.
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    app.manage(TrayHandles { show, quit, tray });
    Ok(())
}

pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != MAIN_WINDOW || window.app_handle().tray_by_id(TRAY_ID).is_none() {
        return;
    }
    if let WindowEvent::CloseRequested { api, .. } = event {
        // Keep the webview and Rust tasks alive; only an explicit tray quit exits.
        // If hiding fails, allow normal close rather than leaving an inaccessible app.
        match window.hide() {
            Ok(()) => api.prevent_close(),
            Err(error) => eprintln!("Failed to hide FN Proxy window: {error}"),
        }
    }
}
