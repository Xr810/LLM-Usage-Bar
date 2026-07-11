use serde_json::Value;

#[test]
fn tauri_identity_and_protocol_do_not_collide_with_original_cc_switch() {
    let config: Value = serde_json::from_str(include_str!("../tauri.conf.json"))
        .expect("tauri.conf.json must remain valid JSON");

    assert_eq!(config["productName"], "LLM Usage Bar");
    assert_eq!(config["identifier"], "com.llmusagebar.desktop");
    assert!(
        config["plugins"].get("deep-link").is_none(),
        "the dashboard must not claim the original CC Switch URL scheme"
    );
    assert_eq!(config["bundle"]["createUpdaterArtifacts"], false);
    assert!(
        config["plugins"].get("updater").is_none(),
        "the original CC Switch release endpoint must not update this app"
    );

    let macos_plist = include_str!("../Info.plist");
    assert!(!macos_plist.contains("<string>ccswitch</string>"));
    assert!(!macos_plist.contains("CFBundleURLSchemes"));

    let windows_config: Value = serde_json::from_str(include_str!("../tauri.windows.conf.json"))
        .expect("tauri.windows.conf.json must remain valid JSON");
    assert_eq!(
        windows_config["app"]["windows"][0]["title"],
        "LLM Usage Bar"
    );

    let tray_source = include_str!("../src/lib.rs");
    assert!(tray_source.contains(".tooltip(\"LLM Usage Bar\")"));
    assert!(!tray_source.contains(".tooltip(\"CC Switch\")"));

    let directory_hook = include_str!("../../src/hooks/useDirectorySettings.ts");
    assert!(directory_hook.contains("join(home, \".llm-usage-bar\")"));
    assert!(!directory_hook.contains("join(home, \".cc-switch\")"));

    let main_source = include_str!("../../src/main.tsx");
    assert!(main_source.contains("~/.llm-usage-bar/config.json"));
    assert!(!main_source.contains("~/.cc-switch/config.json"));
}
