//! BE-12 native catalog.
//!
//! The four locale files are embedded at compile time. Lookup reads only the
//! `native.*` subtree and interpolates `{{var}}`. Fallback is
//! `zh-TW → zh → en`, `ja → en`, and anything else → `en`. A missing key is
//! returned as itself, with a debug-build warning.

use std::collections::HashMap;
use std::sync::OnceLock;

const EN: &str = include_str!("../../../src/i18n/locales/en.json");
const ZH: &str = include_str!("../../../src/i18n/locales/zh.json");
const ZH_TW: &str = include_str!("../../../src/i18n/locales/zh-TW.json");
const JA: &str = include_str!("../../../src/i18n/locales/ja.json");

/// Safe fallback before settings initialization or for unknown stored codes.
pub const UNSET_LANGUAGE: &str = "en";

/// First-launch policy; the caller supplies the OS reader for deterministic tests.
pub(crate) fn initial_language(read_locale: impl FnOnce() -> Option<String>) -> &'static str {
    let locale = read_locale()
        .unwrap_or_default()
        .replace('_', "-")
        .to_ascii_lowercase();
    if locale.starts_with("zh") {
        if locale == "zhtw"
            || locale
                .split('-')
                .any(|part| matches!(part, "tw" | "hk" | "hant"))
        {
            "zh-TW"
        } else {
            "zh"
        }
    } else {
        "en"
    }
}

pub fn effective_language(stored: Option<&str>) -> &str {
    match stored {
        Some(language @ ("en" | "zh" | "zh-TW" | "ja")) => language,
        _ => UNSET_LANGUAGE,
    }
}

pub fn t(lang: &str, key: &str, args: &[(&str, &str)]) -> String {
    let catalogs = embedded_native_catalogs();
    translate_with(lang, key, args, |code, key| {
        catalogs
            .get(code)
            .and_then(|catalog| catalog.get(key))
            .map(String::as_str)
    })
}

pub(crate) fn translate_with<'a, F>(
    lang: &str,
    key: &str,
    args: &[(&str, &str)],
    mut lookup: F,
) -> String
where
    F: FnMut(&str, &str) -> Option<&'a str>,
{
    let template = fallback_chain(lang)
        .iter()
        .find_map(|code| lookup(code, key));
    match template {
        Some(template) => interpolate(template, args),
        None => {
            #[cfg(debug_assertions)]
            log::warn!("missing native i18n key: {key}");
            key.to_string()
        }
    }
}

pub(crate) fn interpolate(template: &str, args: &[(&str, &str)]) -> String {
    let mut output = template.to_string();
    for (name, value) in args {
        let token = format!("{{{{{name}}}}}");
        output = output.replace(&token, value);
    }
    output
}

fn fallback_chain(lang: &str) -> &'static [&'static str] {
    match lang {
        "zh-TW" => &["zh-TW", "zh", "en"],
        "zh" => &["zh", "en"],
        "ja" => &["ja", "en"],
        _ => &["en"],
    }
}

fn embedded_native_catalogs() -> &'static HashMap<&'static str, HashMap<String, String>> {
    static CATALOGS: OnceLock<HashMap<&'static str, HashMap<String, String>>> = OnceLock::new();
    CATALOGS.get_or_init(|| {
        HashMap::from([
            ("en", native_messages(EN)),
            ("zh", native_messages(ZH)),
            ("zh-TW", native_messages(ZH_TW)),
            ("ja", native_messages(JA)),
        ])
    })
}

fn native_messages(json: &str) -> HashMap<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).expect("embedded locale JSON must parse");
    let mut messages = HashMap::new();
    if let Some(native) = value.get("native") {
        collect("native", native, &mut messages);
    }
    messages
}

fn collect(prefix: &str, value: &serde_json::Value, messages: &mut HashMap<String, String>) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                let next = format!("{prefix}.{key}");
                collect(&next, child, messages);
            }
        }
        serde_json::Value::String(text) => {
            messages.insert(prefix.to_string(), text.clone());
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic<'a>() -> HashMap<&'a str, HashMap<&'a str, &'a str>> {
        HashMap::from([
            (
                "en",
                HashMap::from([
                    ("native.tray.greeting", "Hello {{name}}"),
                    ("native.tray.onlyEn", "English only"),
                ]),
            ),
            (
                "zh",
                HashMap::from([("native.tray.greeting", "{{name}} 你好")]),
            ),
            ("zh-TW", HashMap::from([])),
            ("ja", HashMap::from([])),
        ])
    }

    fn lookup<'a>(
        catalogs: &'a HashMap<&str, HashMap<&str, &str>>,
        lang: &str,
        key: &str,
        args: &[(&str, &str)],
    ) -> String {
        translate_with(lang, key, args, |code, key| {
            catalogs
                .get(code)
                .and_then(|catalog| catalog.get(key))
                .copied()
        })
    }

    #[test]
    fn interpolation_replaces_every_named_placeholder() {
        let catalogs = synthetic();
        assert_eq!(
            lookup(&catalogs, "en", "native.tray.greeting", &[("name", "Ada")]),
            "Hello Ada"
        );
        assert_eq!(
            lookup(
                &catalogs,
                "zh",
                "native.tray.greeting",
                &[("name", "Ada"), ("unused", "x")]
            ),
            "Ada 你好"
        );
        assert_eq!(
            interpolate("{{count}} / {{count}}", &[("count", "2")]),
            "2 / 2"
        );
        assert_eq!(interpolate("keep {{missing}}", &[]), "keep {{missing}}");
    }

    #[test]
    fn fallback_chain_uses_zh_then_en_for_zh_tw_and_en_for_ja() {
        let catalogs = synthetic();
        assert_eq!(
            lookup(
                &catalogs,
                "zh-TW",
                "native.tray.greeting",
                &[("name", "Lin")]
            ),
            "Lin 你好"
        );
        assert_eq!(
            lookup(&catalogs, "zh-TW", "native.tray.onlyEn", &[]),
            "English only"
        );
        assert_eq!(
            lookup(&catalogs, "ja", "native.tray.onlyEn", &[]),
            "English only"
        );
        assert_eq!(
            lookup(&catalogs, "fr", "native.tray.onlyEn", &[]),
            "English only"
        );
        assert_eq!(
            lookup(&catalogs, "ja", "native.tray.missing", &[]),
            "native.tray.missing"
        );
    }

    #[test]
    fn embedded_tray_copy_matches_the_locale_files() {
        assert_eq!(t("en", "native.tray.showMain", &[]), "Open main window");
        assert_eq!(t("en", "native.tray.quit", &[]), "Quit");
        assert_eq!(t("zh", "native.tray.showMain", &[]), "打开主界面");
        assert_eq!(t("zh-TW", "native.tray.openWebsite", &[]), "開啟官方網站");
        assert_eq!(
            t("ja", "native.tray.noProviders", &[]),
            "(プロバイダーなし)"
        );
        assert_eq!(t("fr", "native.tray.quit", &[]), "Quit");
        assert_eq!(t("en", "native.tray.missing", &[]), "native.tray.missing");
        assert_eq!(t("en", "app.title", &[]), "app.title");
        assert_eq!(effective_language(None), "en");
        assert_eq!(effective_language(Some("ja")), "ja");
        assert_eq!(effective_language(Some("fr")), "en");
    }
}
