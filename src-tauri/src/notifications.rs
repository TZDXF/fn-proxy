use crate::{
    error::{error, Result},
    text::Text,
};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

#[derive(Default)]
pub struct NotificationLocale(AtomicBool);
pub fn set_locale(app: &AppHandle, locale: &str) {
    if let Some(state) = app.try_state::<NotificationLocale>() {
        match locale {
            "en-US" => state.0.store(true, Ordering::Relaxed),
            "zh-CN" => state.0.store(false, Ordering::Relaxed),
            _ => {}
        }
    }
}
fn localized_notification(message: &Text, english: bool) -> (String, String) {
    let source = if english {
        include_str!("../../src/locales/en-US.json")
    } else {
        include_str!("../../src/locales/zh-CN.json")
    };
    let messages: serde_json::Value = serde_json::from_str(source).expect("bundled translations");
    let title = messages["notifications"]["connectionDisconnected"]
        .as_str()
        .unwrap_or("FN Proxy")
        .to_owned();
    let value = message
        .code
        .split('.')
        .fold(&messages, |value, key| &value[key]);
    let mut body = value
        .as_str()
        .unwrap_or("FN Proxy connection disconnected.")
        .to_owned();
    for (key, value) in &message.params {
        body = body.replace(&format!("{{{key}}}"), value);
    }
    (title, body)
}
pub fn show_connection_failure(app: &AppHandle, message: &Text) -> Result<()> {
    let english = app
        .try_state::<NotificationLocale>()
        .is_some_and(|state| state.0.load(Ordering::Relaxed));
    let (title, body) = localized_notification(message, english);
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|_| error("logs.systemNotificationFailed"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_notifications_share_ui_translations_and_interpolate_connection_and_limit() {
        let text = Text::with(
            "notice.recoveryExhausted",
            [("name", "fixture-nas".into()), ("max", "3".into())],
        );
        for english in [false, true] {
            let (title, body) = localized_notification(&text, english);
            assert!(!title.is_empty());
            assert!(body.contains("fixture-nas"));
            assert!(body.contains('3'));
            assert!(!body.contains("{name}"));
            assert!(!body.contains("{max}"));
            assert!(!body.contains("notice."));
        }
    }
}
