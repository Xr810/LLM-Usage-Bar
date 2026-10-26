//! BE-12 language commands (C8).
//!
//! `settings.language` is the only stored language. `get_language` returns
//! the effective language after settings initialize an absent value: `zh` for
//! existing installs, OS locale only for fresh installs. `set_language`
//! persists a supported language and emits one `settings-changed` event with
//! `keys: ["language"]` when the stored value changes. There is no
//! `language-changed` event.
//!
//! Rebuilding the live tray menu needs a Tauri app handle, so that step is
//! not covered by the unit tests here. The catalog lookup and the change
//! decision are.
//!
//! Budget alerts read C10 `TrayApiBudgetView.level`. This module does not
//! add C12 `get_guard_state`; that command is not on main. Notification
//! copy uses the embedded native catalog through `NotificationTexts`.

use tauri::{AppHandle, State};

use crate::app_state::AppState;
use crate::config::settings::AppSettings;
use crate::error::AppError;
use crate::i18n::effective_language;

#[tauri::command]
pub fn get_language() -> String {
    effective_language(crate::config::settings::get_settings().language.as_deref()).to_string()
}

#[tauri::command]
pub fn set_language(
    app: AppHandle,
    state: State<'_, AppState>,
    lang: String,
) -> Result<String, AppError> {
    let mut settings = crate::config::settings::get_settings();
    if assign_language(&mut settings, &lang)? {
        crate::config::settings::update_settings(settings)?;
        crate::api::commands::ui_prefs::emit_settings_changed(&app, &["language".to_string()]);
        rebuild_tray_menu(&app, state.inner());
    }
    Ok(effective_language(crate::config::settings::get_settings().language.as_deref()).to_string())
}

pub(crate) fn assign_language(settings: &mut AppSettings, raw: &str) -> Result<bool, AppError> {
    let language = canonical_language(raw)?;
    if settings.language.as_deref() == Some(language) {
        return Ok(false);
    }
    settings.language = Some(language.to_string());
    Ok(true)
}

fn canonical_language(raw: &str) -> Result<&'static str, AppError> {
    match raw.trim() {
        "en" => Ok("en"),
        "zh" => Ok("zh"),
        "zh-TW" => Ok("zh-TW"),
        "ja" => Ok("ja"),
        _ => Err(AppError::invalid_argument(
            "Language must be en, zh, zh-TW, or ja.",
            "language",
        )),
    }
}

fn rebuild_tray_menu(app: &AppHandle, state: &AppState) {
    match crate::tray::create_tray_menu(app, state) {
        Ok(menu) => {
            if let Some(tray) = app.tray_by_id(crate::tray::TRAY_ID) {
                if let Err(err) = tray.set_menu(Some(menu)) {
                    log::warn!("language change could not replace the tray menu: {err}");
                }
            }
        }
        Err(err) => {
            log::warn!("language change could not rebuild the tray menu: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::commands::ui_prefs::{emit_settings_changed_with, EVENT_SETTINGS_CHANGED};
    use crate::error::CODE_INVALID_ARGUMENT;
    use crate::tray::TrayTexts;

    #[test]
    fn assign_language_is_idempotent_and_rejects_unknown_codes() {
        let mut settings = AppSettings {
            language: Some("zh".to_string()),
            ..AppSettings::default()
        };

        assert!(!assign_language(&mut settings, "zh").expect("same"));
        assert!(!assign_language(&mut settings, " zh ").expect("trimmed same"));
        assert!(assign_language(&mut settings, "en").expect("switch"));
        assert_eq!(settings.language.as_deref(), Some("en"));

        let err = assign_language(&mut settings, "zh-CN").expect_err("unsupported");
        assert_eq!(err.code(), Some(CODE_INVALID_ARGUMENT));
        assert_eq!(
            serde_json::to_value(&err).expect("json")["details"]["field"],
            "language"
        );
        assert_eq!(settings.language.as_deref(), Some("en"));
        assert!(assign_language(&mut settings, "").is_err());
        assert!(assign_language(&mut settings, "fr").is_err());
    }

    #[test]
    fn a_language_change_emits_one_settings_changed_event() {
        assert_eq!(EVENT_SETTINGS_CHANGED, "settings-changed");
        let mut emissions = 0;
        emit_settings_changed_with(&["language".to_string()], |payload| {
            emissions += 1;
            assert_eq!(payload.keys, vec!["language".to_string()]);
        });
        emit_settings_changed_with(&[], |_| emissions += 1);
        assert_eq!(emissions, 1);
    }

    #[test]
    fn tray_copy_follows_the_catalog_for_zh_and_en() {
        let english = TrayTexts::from_language("en");
        let chinese = TrayTexts::from_language("zh");
        assert_eq!(english.show_main, "Open main window");
        assert_eq!(english.quit, "Quit");
        assert_eq!(chinese.show_main, "打开主界面");
        assert_eq!(chinese.open_website, "打开官方网站");
        assert_eq!(chinese.lightweight_mode, "轻量模式");
        assert_ne!(english.show_main, chinese.show_main);
    }
}
