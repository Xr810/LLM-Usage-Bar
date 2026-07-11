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
}
