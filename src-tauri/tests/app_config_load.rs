use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use llm_usage_bar_lib::product_identity::{
    DATABASE_FILE, DATABASE_IDENTITY_ARCHIVE_FILE, LEGACY_DATABASE_FILE, LOG_BASENAME,
};
use llm_usage_bar_lib::{
    prepare_database_runtime_test_hook, runtime_log_paths_test_hook, AppError, AppType, Database,
    MultiAppConfig, Provider,
};

fn ensure_test_home() -> &'static Path {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let base = std::env::temp_dir().join(format!(
            "llm-usage-bar-app-config-load-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("create isolated test home");
        std::env::set_var("CC_SWITCH_TEST_HOME", &base);
        std::env::set_var("HOME", &base);
        base
    })
    .as_path()
}

fn reset_test_fs() {
    let home = ensure_test_home();
    for subdir in [".llm-usage-bar", ".cc-switch"] {
        let path = home.join(subdir);
        if path.exists() {
            fs::remove_dir_all(&path).expect("reset isolated test directory");
        }
    }
}

fn test_mutex() -> &'static Mutex<()> {
    static MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
    MUTEX.get_or_init(|| Mutex::new(()))
}

fn cfg_path() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME should be set by ensure_test_home");
    PathBuf::from(home)
        .join(".llm-usage-bar")
        .join("config.json")
}

#[test]
fn database_identity_migrates_v13_and_threads_authoritative_runtime_paths() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let app_dir = home.join(".llm-usage-bar");
    fs::create_dir_all(&app_dir).expect("create app config dir");
    let canonical_app_dir = fs::canonicalize(&app_dir).expect("canonicalize app config dir");
    let old_path = app_dir.join(LEGACY_DATABASE_FILE);

    let old_db = Database::init_at(&old_path).expect("create real v13 prior-name database");
    assert_eq!(old_db.database_path(), Some(old_path.as_path()));
    old_db
        .save_provider(
            "claude",
            &Provider::with_id(
                "identity-fixture".to_string(),
                "Identity Fixture".to_string(),
                serde_json::json!({"env": {"ANTHROPIC_AUTH_TOKEN": "fixture-only"}}),
                None,
            ),
        )
        .expect("seed prior-name database");
    drop(old_db);

    let prepared =
        prepare_database_runtime_test_hook(&app_dir).expect("prepare and open runtime database");
    let identity = &prepared.identity;
    let expected_new_path = canonical_app_dir.join(DATABASE_FILE);
    let expected_archive_path = canonical_app_dir.join(DATABASE_IDENTITY_ARCHIVE_FILE);

    assert!(identity.migrated);
    assert_eq!(identity.database_path, expected_new_path);
    assert_eq!(
        identity.archived_prior_path,
        Some(expected_archive_path.clone())
    );
    assert_eq!(identity.retained_prior_path, None);
    assert_eq!(identity.durability_warning, None);
    assert!(identity.database_path.exists());
    assert!(expected_archive_path.exists());
    assert!(!canonical_app_dir.join(LEGACY_DATABASE_FILE).exists());

    assert_eq!(
        prepared.database.database_path(),
        Some(identity.database_path.as_path())
    );

    let exported = prepared
        .database
        .export_sql_string()
        .expect("export authoritative database");
    let backup_id = prepared
        .database
        .import_sql_string(&exported)
        .expect("import should back up the authoritative database");
    assert!(
        !backup_id.is_empty(),
        "disk database backup must be created"
    );
    assert!(canonical_app_dir
        .join("backups")
        .join(format!("{backup_id}.db"))
        .exists());

    let (file_log, crash_log) = runtime_log_paths_test_hook(&canonical_app_dir);
    assert_eq!(
        file_log,
        canonical_app_dir
            .join("logs")
            .join(format!("{LOG_BASENAME}.log"))
    );
    assert_eq!(crash_log, canonical_app_dir.join("crash.log"));
}

#[test]
fn database_runtime_fresh_install_opens_the_prepared_authoritative_path() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let app_dir = home.join(".llm-usage-bar");
    fs::create_dir_all(&app_dir).expect("create fresh app config dir");
    let canonical_app_dir = fs::canonicalize(&app_dir).expect("canonicalize app config dir");

    let prepared =
        prepare_database_runtime_test_hook(&app_dir).expect("prepare and open fresh database");
    let expected = canonical_app_dir.join(DATABASE_FILE);

    assert!(!prepared.identity.migrated);
    assert_eq!(prepared.identity.database_path, expected);
    assert_eq!(
        prepared.database.database_path(),
        Some(prepared.identity.database_path.as_path())
    );
    assert!(prepared.identity.database_path.exists());
}

#[test]
fn load_v1_config_returns_error_and_does_not_write() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let path = cfg_path();
    fs::create_dir_all(path.parent().unwrap()).expect("create cfg dir");

    // 最小 v1 形状：providers + current，且不含 version/apps/mcp
    let v1_json = r#"{"providers":{},"current":""}"#;
    fs::write(&path, v1_json).expect("seed v1 json");
    let before = fs::read_to_string(&path).expect("read before");

    let err = MultiAppConfig::load().expect_err("v1 should not be auto-migrated");
    match err {
        AppError::Localized { key, .. } => assert_eq!(key, "config.unsupported_v1"),
        other => panic!("expected Localized v1 error, got {other:?}"),
    }

    // 文件不应有任何变化，且不应生成 .bak
    let after = fs::read_to_string(&path).expect("read after");
    assert_eq!(before, after, "config.json should not be modified");
    let bak = home.join(".llm-usage-bar").join("config.json.bak");
    assert!(!bak.exists(), ".bak should not be created on load error");
}

#[test]
fn load_v1_with_extra_version_still_treated_as_v1() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let path = cfg_path();
    std::fs::create_dir_all(path.parent().unwrap()).expect("create cfg dir");

    // 畸形：包含 providers + current + version，但没有 apps，应按 v1 处理
    let v1_like = r#"{"providers":{},"current":"","version":2}"#;
    std::fs::write(&path, v1_like).expect("seed v1-like json");
    let before = std::fs::read_to_string(&path).expect("read before");

    let err = MultiAppConfig::load().expect_err("v1-like should not be parsed as v2");
    match err {
        AppError::Localized { key, .. } => assert_eq!(key, "config.unsupported_v1"),
        other => panic!("expected Localized v1 error, got {other:?}"),
    }

    let after = std::fs::read_to_string(&path).expect("read after");
    assert_eq!(before, after, "config.json should not be modified");
    let bak = home.join(".llm-usage-bar").join("config.json.bak");
    assert!(!bak.exists(), ".bak should not be created on v1-like error");
}

#[test]
fn load_invalid_json_returns_parse_error_and_does_not_write() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let path = cfg_path();
    fs::create_dir_all(path.parent().unwrap()).expect("create cfg dir");

    fs::write(&path, "{not json").expect("seed invalid json");
    let before = fs::read_to_string(&path).expect("read before");

    let err = MultiAppConfig::load().expect_err("invalid json should error");
    match err {
        AppError::Json { .. } => {}
        other => panic!("expected Json error, got {other:?}"),
    }

    let after = fs::read_to_string(&path).expect("read after");
    assert_eq!(before, after, "config.json should remain unchanged");
    let bak = home.join(".llm-usage-bar").join("config.json.bak");
    assert!(!bak.exists(), ".bak should not be created on parse error");
}

#[test]
fn load_valid_v2_config_succeeds() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();
    let path = cfg_path();
    fs::create_dir_all(path.parent().unwrap()).expect("create cfg dir");

    // 使用默认结构序列化为 v2
    let default_cfg = MultiAppConfig::default();
    let json = serde_json::to_string_pretty(&default_cfg).expect("serialize default cfg");
    fs::write(&path, json).expect("write v2 json");

    let loaded = MultiAppConfig::load().expect("v2 should load successfully");
    assert_eq!(loaded.version, 2);
    assert!(loaded.get_manager(&AppType::Claude).is_some());
    assert!(loaded.get_manager(&AppType::Codex).is_some());
}
