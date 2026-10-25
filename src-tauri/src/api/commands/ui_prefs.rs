//! BE-23 UI preferences (C18, and the C8 language event).
//!
//! Preferences live on device-local `settings.json`. `failureModes` must not
//! be written into the sqlite `settings` table, because that table is part of
//! the WebDAV and S3 snapshot. This stage does not add cloud sync.
//!
//! `settings-changed` is emitted once per successful change, with every key
//! that actually changed. `language` uses the same event. There is no
//! `language-changed` event.

use tauri::{AppHandle, Emitter};

use crate::config::settings::{
    AppSettings, FailureMode, FailureModes, IdentitySwitchPolicy, UiPrefs,
};
use crate::error::AppError;

pub(crate) const EVENT_SETTINGS_CHANGED: &str = "settings-changed";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SettingsChanged {
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UiPrefsChange {
    pub changed_keys: Vec<String>,
    pub opened_failure_modes: Vec<&'static str>,
}

#[tauri::command]
pub fn get_ui_prefs() -> UiPrefs {
    crate::config::settings::get_settings().ui_prefs()
}

/// Patch semantics: omitted fields stay as they are, including a nested
/// `failureModes` field. The same value written again changes nothing and
/// emits no event. Out of range or unknown values return `INVALID_ARGUMENT`
/// and write nothing.
#[tauri::command]
pub fn set_ui_prefs(app: AppHandle, patch: serde_json::Value) -> Result<UiPrefs, AppError> {
    let mut settings = crate::config::settings::get_settings();
    let change = apply_ui_prefs_patch(&mut settings, &patch)?;
    if !change.changed_keys.is_empty() {
        crate::config::settings::update_settings(settings)?;
        log_opened_failure_modes(&change.opened_failure_modes);
        emit_settings_changed(&app, &change.changed_keys);
    }
    Ok(crate::config::settings::get_settings().ui_prefs())
}

pub(crate) fn language_changed_keys(before: Option<&str>, after: Option<&str>) -> Vec<String> {
    if before == after {
        Vec::new()
    } else {
        vec!["language".to_string()]
    }
}

pub(crate) fn emit_settings_changed(app: &AppHandle, keys: &[String]) {
    emit_settings_changed_with(keys, |payload| {
        if let Err(err) = app.emit(EVENT_SETTINGS_CHANGED, payload) {
            log::warn!("settings-changed emit failed: {err}");
        }
    });
}

pub(crate) fn emit_settings_changed_with<F>(keys: &[String], emit: F)
where
    F: FnOnce(&SettingsChanged),
{
    if keys.is_empty() {
        return;
    }
    emit(&SettingsChanged {
        keys: keys.to_vec(),
    });
}

fn log_opened_failure_modes(opened: &[&str]) {
    for mode in opened {
        log::warn!("UI failure mode {mode} changed to open; this lowers the local safety default");
    }
}

pub(crate) fn apply_ui_prefs_patch(
    settings: &mut AppSettings,
    patch: &serde_json::Value,
) -> Result<UiPrefsChange, AppError> {
    let Some(object) = patch.as_object() else {
        return Err(AppError::invalid_argument(
            "UI preference patch must be an object.",
            "patch",
        ));
    };
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "alertThresholdPct" | "subscriptionWarnPct" | "onIdentitySwitch" | "failureModes"
        ) {
            return Err(AppError::invalid_argument("Unknown UI preference.", key));
        }
    }

    let next_alert = match object.get("alertThresholdPct") {
        Some(value) => require_percent(value, "alertThresholdPct")?,
        None => settings.alert_threshold_pct,
    };
    let next_subscription = match object.get("subscriptionWarnPct") {
        Some(value) => require_percent(value, "subscriptionWarnPct")?,
        None => settings.subscription_warn_pct,
    };
    let next_switch = match object.get("onIdentitySwitch") {
        Some(value) => require_identity_switch(value)?,
        None => settings.on_identity_switch,
    };
    let next_modes = match object.get("failureModes") {
        Some(value) => require_failure_modes(settings.failure_modes, value)?,
        None => settings.failure_modes,
    };

    let mut changed_keys = Vec::new();
    let mut opened_failure_modes = Vec::new();
    if next_alert != settings.alert_threshold_pct {
        settings.alert_threshold_pct = next_alert;
        changed_keys.push("alertThresholdPct".to_string());
    }
    if next_subscription != settings.subscription_warn_pct {
        settings.subscription_warn_pct = next_subscription;
        changed_keys.push("subscriptionWarnPct".to_string());
    }
    if next_switch != settings.on_identity_switch {
        settings.on_identity_switch = next_switch;
        changed_keys.push("onIdentitySwitch".to_string());
    }
    if next_modes != settings.failure_modes {
        if next_modes.proxy == FailureMode::Open
            && settings.failure_modes.proxy != FailureMode::Open
        {
            opened_failure_modes.push("proxy");
        }
        if next_modes.license == FailureMode::Open
            && settings.failure_modes.license != FailureMode::Open
        {
            opened_failure_modes.push("license");
        }
        settings.failure_modes = next_modes;
        changed_keys.push("failureModes".to_string());
    }

    Ok(UiPrefsChange {
        changed_keys,
        opened_failure_modes,
    })
}

fn require_percent(value: &serde_json::Value, field: &str) -> Result<u8, AppError> {
    let Some(number) = value.as_i64() else {
        return Err(AppError::invalid_argument(
            "Percent must be an integer from 1 to 100.",
            field,
        ));
    };
    if (1..=100).contains(&number) {
        Ok(number as u8)
    } else {
        Err(AppError::invalid_argument(
            "Percent must be an integer from 1 to 100.",
            field,
        ))
    }
}

fn require_identity_switch(value: &serde_json::Value) -> Result<IdentitySwitchPolicy, AppError> {
    match value.as_str() {
        Some("ask") => Ok(IdentitySwitchPolicy::Ask),
        Some("auto_allow") => Ok(IdentitySwitchPolicy::AutoAllow),
        _ => Err(AppError::invalid_argument(
            "onIdentitySwitch must be ask or auto_allow.",
            "onIdentitySwitch",
        )),
    }
}

fn require_failure_modes(
    current: FailureModes,
    value: &serde_json::Value,
) -> Result<FailureModes, AppError> {
    let Some(object) = value.as_object() else {
        return Err(AppError::invalid_argument(
            "failureModes must be an object.",
            "failureModes",
        ));
    };
    for key in object.keys() {
        if key != "proxy" && key != "license" {
            return Err(AppError::invalid_argument(
                "Unknown failureModes field.",
                &format!("failureModes.{key}"),
            ));
        }
    }
    let mut next = current;
    if let Some(proxy) = object.get("proxy") {
        next.proxy = require_failure_mode(proxy, "failureModes.proxy")?;
    }
    if let Some(license) = object.get("license") {
        next.license = require_failure_mode(license, "failureModes.license")?;
    }
    Ok(next)
}

fn require_failure_mode(value: &serde_json::Value, field: &str) -> Result<FailureMode, AppError> {
    match value.as_str() {
        Some("closed") => Ok(FailureMode::Closed),
        Some("open") => Ok(FailureMode::Open),
        _ => Err(AppError::invalid_argument(
            "Failure mode must be closed or open.",
            field,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CODE_INVALID_ARGUMENT;
    use serde_json::json;

    fn prefs_of(settings: &AppSettings) -> UiPrefs {
        settings.ui_prefs()
    }

    #[test]
    fn omitted_fields_stay_put_and_one_change_is_one_event() {
        let mut settings = AppSettings {
            alert_threshold_pct: 91,
            subscription_warn_pct: 64,
            on_identity_switch: IdentitySwitchPolicy::Ask,
            failure_modes: FailureModes {
                proxy: FailureMode::Closed,
                license: FailureMode::Open,
            },
            ..AppSettings::default()
        };
        let before = prefs_of(&settings);

        let change = apply_ui_prefs_patch(
            &mut settings,
            &json!({
                "subscriptionWarnPct": 64,
                "failureModes": { "proxy": "open" }
            }),
        )
        .expect("patch");

        assert_eq!(change.changed_keys, vec!["failureModes".to_string()]);
        assert_eq!(change.opened_failure_modes, vec!["proxy"]);
        assert_eq!(settings.alert_threshold_pct, before.alert_threshold_pct);
        assert_eq!(settings.subscription_warn_pct, 64);
        assert_eq!(settings.on_identity_switch, IdentitySwitchPolicy::Ask);
        assert_eq!(settings.failure_modes.proxy, FailureMode::Open);
        assert_eq!(settings.failure_modes.license, FailureMode::Open);

        let mut emissions = 0;
        emit_settings_changed_with(&change.changed_keys, |payload| {
            emissions += 1;
            assert_eq!(payload.keys, vec!["failureModes".to_string()]);
            assert_eq!(
                serde_json::to_value(payload).expect("payload"),
                json!({ "keys": ["failureModes"] })
            );
        });
        emit_settings_changed_with(&[], |_| emissions += 1);
        assert_eq!(emissions, 1);
    }

    #[test]
    fn a_patch_can_change_several_keys_in_one_event() {
        let mut settings = AppSettings::default();
        let change = apply_ui_prefs_patch(
            &mut settings,
            &json!({
                "alertThresholdPct": 90,
                "onIdentitySwitch": "auto_allow",
                "failureModes": { "license": "open" }
            }),
        )
        .expect("patch");

        assert_eq!(
            change.changed_keys,
            vec![
                "alertThresholdPct".to_string(),
                "onIdentitySwitch".to_string(),
                "failureModes".to_string(),
            ]
        );
        assert_eq!(change.opened_failure_modes, vec!["license"]);
        assert_eq!(settings.subscription_warn_pct, 70);
        assert_eq!(settings.failure_modes.proxy, FailureMode::Closed);

        let mut emissions = 0;
        emit_settings_changed_with(&change.changed_keys, |payload| {
            emissions += 1;
            assert_eq!(payload.keys.len(), 3);
        });
        assert_eq!(emissions, 1);
    }

    #[test]
    fn repeating_the_same_patch_changes_nothing() {
        let mut settings = AppSettings::default();
        let first = apply_ui_prefs_patch(
            &mut settings,
            &json!({ "alertThresholdPct": 88, "onIdentitySwitch": "ask" }),
        )
        .expect("first");
        assert_eq!(first.changed_keys, vec!["alertThresholdPct".to_string()]);

        let second = apply_ui_prefs_patch(
            &mut settings,
            &json!({ "alertThresholdPct": 88, "onIdentitySwitch": "ask" }),
        )
        .expect("second");
        assert!(second.changed_keys.is_empty());
        assert!(second.opened_failure_modes.is_empty());
    }

    #[test]
    fn out_of_range_and_non_integers_are_invalid_argument_and_write_nothing() {
        let mut settings = AppSettings {
            alert_threshold_pct: 91,
            ..AppSettings::default()
        };
        for value in [
            json!(0),
            json!(101),
            json!(-1),
            json!(80.5),
            json!("80"),
            json!(true),
        ] {
            let err = apply_ui_prefs_patch(
                &mut settings,
                &json!({ "alertThresholdPct": value, "subscriptionWarnPct": 65 }),
            )
            .expect_err("invalid percent");
            assert_eq!(err.code(), Some(CODE_INVALID_ARGUMENT));
            let payload = serde_json::to_value(&err).expect("json");
            assert_eq!(payload["details"]["field"], "alertThresholdPct");
            assert_eq!(settings.alert_threshold_pct, 91);
            assert_eq!(settings.subscription_warn_pct, 70);
        }
    }

    #[test]
    fn unknown_keys_and_enums_are_rejected_without_a_partial_write() {
        let mut settings = AppSettings {
            alert_threshold_pct: 91,
            failure_modes: FailureModes {
                proxy: FailureMode::Closed,
                license: FailureMode::Closed,
            },
            ..AppSettings::default()
        };

        let unknown = apply_ui_prefs_patch(
            &mut settings,
            &json!({ "showTextLabels": false, "alertThresholdPct": 90 }),
        )
        .expect_err("unknown");
        assert_eq!(
            serde_json::to_value(&unknown).expect("json")["details"]["field"],
            "showTextLabels"
        );

        let mode = apply_ui_prefs_patch(
            &mut settings,
            &json!({ "onIdentitySwitch": "auto", "alertThresholdPct": 90 }),
        )
        .expect_err("enum");
        assert_eq!(
            serde_json::to_value(&mode).expect("json")["details"]["field"],
            "onIdentitySwitch"
        );

        let nested = apply_ui_prefs_patch(
            &mut settings,
            &json!({ "failureModes": { "proxy": "open", "dns": "open" } }),
        )
        .expect_err("nested");
        assert_eq!(
            serde_json::to_value(&nested).expect("json")["details"]["field"],
            "failureModes.dns"
        );
        assert_eq!(settings.alert_threshold_pct, 91);
        assert_eq!(settings.failure_modes.proxy, FailureMode::Closed);
        assert_eq!(settings.on_identity_switch, IdentitySwitchPolicy::Ask);
    }

    #[test]
    fn ui_prefs_are_not_written_into_the_synced_sqlite_settings_table() {
        let db = crate::store::Database::memory().expect("memory db");
        let mut settings = AppSettings::default();
        let change = apply_ui_prefs_patch(
            &mut settings,
            &json!({
                "alertThresholdPct": 91,
                "subscriptionWarnPct": 71,
                "onIdentitySwitch": "auto_allow",
                "failureModes": { "proxy": "open", "license": "closed" }
            }),
        )
        .expect("patch");
        assert_eq!(change.changed_keys.len(), 4);
        assert_eq!(change.opened_failure_modes, vec!["proxy"]);

        let sql = db.export_sql_string_for_sync().expect("sync sql");
        let lowered = sql.to_ascii_lowercase();
        for key in [
            "alertthresholdpct",
            "alert_threshold_pct",
            "subscriptionwarnpct",
            "subscription_warn_pct",
            "onidentityswitch",
            "on_identity_switch",
            "failuremodes",
            "failure_modes",
        ] {
            assert!(
                db.get_setting(key).expect("read").is_none(),
                "{key} was stored in sqlite settings"
            );
            assert!(
                !lowered.contains(key),
                "{key} leaked into the WebDAV/S3 snapshot"
            );
        }
        assert!(crate::sync::webdav_auto_sync::should_trigger_for_table(
            "settings"
        ));
        assert!(!crate::sync::webdav_auto_sync::should_trigger_for_table(
            "failureModes"
        ));
    }
}
