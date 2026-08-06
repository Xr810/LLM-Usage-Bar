//! 数据库模块测试
//!
//! 包含 Schema 迁移和基本功能的测试。

use super::*;
use crate::app_config::MultiAppConfig;
use crate::provider::{Provider, ProviderManager};
use indexmap::IndexMap;
use rusqlite::{params, Connection};
use serde_json::json;
use std::collections::HashMap;
use tempfile::NamedTempFile;

mod usage_sync_cursor_tests {
    use super::*;

    fn cursor_fixture(source: &str, key: &str, byte_offset: i64) -> UsageSyncCursor {
        UsageSyncCursor {
            source: source.to_string(),
            cursor_key: key.to_string(),
            resource_path: Some(format!("/tmp/{source}/{key}")),
            resource_identity: Some(format!("unix:{source}:{key}")),
            modified_at_ns: 11,
            size_bytes: byte_offset + 100,
            byte_offset,
            line_offset: 3,
            parser_state_json: Some(r#"{"version":1,"eventIndex":2}"#.to_string()),
            last_success_at: 33,
        }
    }

    #[test]
    fn v14_cursor_round_trips_every_resume_field() {
        let db = Database::memory().unwrap();
        let cursor = UsageSyncCursor {
            source: "codex".into(),
            cursor_key: "session.jsonl".into(),
            resource_path: Some("/tmp/session.jsonl".into()),
            resource_identity: Some("unix:1:2".into()),
            modified_at_ns: 11,
            size_bytes: 22,
            byte_offset: 17,
            line_offset: 3,
            parser_state_json: Some(r#"{"version":1,"eventIndex":2}"#.into()),
            last_success_at: 33,
        };

        db.put_usage_sync_cursor(&cursor).unwrap();

        assert_eq!(
            db.get_usage_sync_cursor("codex", "session.jsonl").unwrap(),
            Some(cursor)
        );
    }

    #[test]
    fn cursor_update_is_atomic_and_source_scoped() {
        let db = Database::memory().unwrap();
        let mut claude = cursor_fixture("claude", "same.jsonl", 10);
        let codex = cursor_fixture("codex", "same.jsonl", 20);
        db.put_usage_sync_cursor(&claude).unwrap();
        db.put_usage_sync_cursor(&codex).unwrap();

        claude.resource_path = None;
        claude.resource_identity = Some("unix:updated".into());
        claude.modified_at_ns = 111;
        claude.size_bytes = 222;
        claude.byte_offset = 30;
        claude.line_offset = 13;
        claude.parser_state_json = Some(r#"{"version":2}"#.into());
        claude.last_success_at = 333;
        db.put_usage_sync_cursor(&claude).unwrap();

        assert_eq!(
            db.get_usage_sync_cursor("claude", "same.jsonl").unwrap(),
            Some(claude)
        );
        assert_eq!(
            db.get_usage_sync_cursor("codex", "same.jsonl").unwrap(),
            Some(codex)
        );
    }

    #[test]
    fn cursor_list_returns_only_the_requested_source() {
        let db = Database::memory().unwrap();
        let claude_a = cursor_fixture("claude", "a.jsonl", 10);
        let claude_b = cursor_fixture("claude", "b.jsonl", 20);
        let codex = cursor_fixture("codex", "a.jsonl", 30);
        db.put_usage_sync_cursor(&claude_b).unwrap();
        db.put_usage_sync_cursor(&codex).unwrap();
        db.put_usage_sync_cursor(&claude_a).unwrap();

        assert_eq!(
            db.list_usage_sync_cursors("claude").unwrap(),
            vec![claude_a, claude_b]
        );
        assert_eq!(db.list_usage_sync_cursors("missing").unwrap(), Vec::new());
    }

    #[test]
    fn connection_level_cursor_upsert_participates_in_the_callers_transaction(
    ) -> Result<(), AppError> {
        let db = Database::memory().unwrap();
        let cursor = cursor_fixture("claude", "transaction.jsonl", 10);

        {
            let mut conn = lock_conn!(db.conn);
            let transaction = conn.transaction().unwrap();
            Database::put_usage_sync_cursor_on_conn(&transaction, &cursor).unwrap();
            transaction.rollback().unwrap();
        }

        assert_eq!(
            db.get_usage_sync_cursor("claude", "transaction.jsonl")
                .unwrap(),
            None
        );
        Ok(())
    }

    #[test]
    fn cursor_delete_uses_the_composite_source_key() {
        let db = Database::memory().unwrap();
        let claude = cursor_fixture("claude", "same.jsonl", 10);
        let codex = cursor_fixture("codex", "same.jsonl", 20);
        db.put_usage_sync_cursor(&claude).unwrap();
        db.put_usage_sync_cursor(&codex).unwrap();

        db.delete_usage_sync_cursor("claude", "same.jsonl").unwrap();

        assert_eq!(
            db.get_usage_sync_cursor("claude", "same.jsonl").unwrap(),
            None
        );
        assert_eq!(
            db.get_usage_sync_cursor("codex", "same.jsonl").unwrap(),
            Some(codex)
        );
    }

    #[test]
    fn cursor_rejects_negative_sizes_and_offsets_before_sql() {
        let db = Database::memory().unwrap();

        for (field, cursor) in [
            ("size_bytes", {
                let mut cursor = cursor_fixture("codex", "negative-size", 0);
                cursor.size_bytes = -1;
                cursor
            }),
            ("byte_offset", {
                let mut cursor = cursor_fixture("codex", "negative-byte", 0);
                cursor.byte_offset = -1;
                cursor
            }),
            ("line_offset", {
                let mut cursor = cursor_fixture("codex", "negative-line", 0);
                cursor.line_offset = -1;
                cursor
            }),
        ] {
            let error = db
                .put_usage_sync_cursor(&cursor)
                .expect_err("negative cursor field must be rejected");
            assert!(
                error.to_string().contains(field),
                "error should identify {field}: {error}"
            );
            assert_eq!(
                db.get_usage_sync_cursor(&cursor.source, &cursor.cursor_key)
                    .unwrap(),
                None,
                "invalid cursor must not reach SQL"
            );
        }
    }
}

mod schema_v14_cursor_migration_tests {
    use super::*;
    use crate::usage::source_roots::UsageSourceRoots;
    use std::path::PathBuf;

    fn roots() -> UsageSourceRoots {
        UsageSourceRoots {
            claude: PathBuf::from("/Users/test/.claude/projects"),
            codex: PathBuf::from("/Users/test/.codex"),
            gemini: PathBuf::from("/Users/test/.gemini/tmp"),
            opencode: PathBuf::from("/Users/test/.local/share/opencode"),
        }
    }

    fn v13_current_tables() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        Database::create_tables_on_conn(&conn).unwrap();
        crate::usage::migration::migrate_v12_to_v13(&conn).unwrap();
        Database::set_user_version(&conn, 13).unwrap();
        conn
    }

    #[test]
    fn migration_v13_to_current_archives_line_state_and_sets_version_once() {
        let conn = v13_current_tables();
        conn.execute(
            "INSERT INTO session_log_sync
             (file_path, last_modified, last_line_offset, last_synced_at)
             VALUES (?1, 11, 3, 33)",
            ["/Users/test/.claude/projects/p/session.jsonl"],
        )
        .unwrap();

        Database::apply_schema_migrations_on_conn_with_roots(&conn, &roots()).unwrap();

        assert_eq!(Database::get_user_version(&conn).unwrap(), SCHEMA_VERSION);
        assert!(!Database::table_exists(&conn, "session_log_sync").unwrap());
        assert!(Database::table_exists(&conn, "session_log_sync_v13_archive").unwrap());
        let cursor = conn
            .query_row(
                "SELECT source, byte_offset, line_offset
                 FROM usage_sync_cursors WHERE cursor_key = ?1",
                ["/Users/test/.claude/projects/p/session.jsonl"],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(cursor, ("claude".to_string(), 0, 3));

        Database::create_tables_on_conn(&conn).unwrap();
        assert!(
            !Database::table_exists(&conn, "session_log_sync").unwrap(),
            "v14 startup must not recreate the writable v13 table"
        );
    }

    #[test]
    fn v14_migration_failure_rolls_back_archive_cursor_and_version() {
        let conn = v13_current_tables();
        conn.execute("DROP TABLE IF EXISTS usage_sync_cursors", [])
            .unwrap();
        conn.execute_batch(
            "CREATE TABLE usage_sync_cursors (
                 source TEXT NOT NULL CHECK (source != 'claude'),
                 cursor_key TEXT NOT NULL,
                 resource_path TEXT,
                 resource_identity TEXT,
                 modified_at_ns INTEGER NOT NULL DEFAULT 0,
                 size_bytes INTEGER NOT NULL DEFAULT 0,
                 byte_offset INTEGER NOT NULL DEFAULT 0,
                 line_offset INTEGER NOT NULL DEFAULT 0,
                 parser_state_json TEXT,
                 last_success_at INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY (source, cursor_key)
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_log_sync
             (file_path, last_modified, last_line_offset, last_synced_at)
             VALUES (?1, 11, 3, 33)",
            ["/Users/test/.claude/projects/p/session.jsonl"],
        )
        .unwrap();

        Database::apply_schema_migrations_on_conn_with_roots(&conn, &roots())
            .expect_err("forced cursor insert failure must roll back the outer savepoint");

        assert_eq!(Database::get_user_version(&conn).unwrap(), 13);
        assert!(Database::table_exists(&conn, "session_log_sync").unwrap());
        assert!(!Database::table_exists(&conn, "session_log_sync_v13_archive").unwrap());
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM usage_sync_cursors", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn incomplete_v14_is_rejected_instead_of_silently_repaired() {
        let conn = Connection::open_in_memory().unwrap();
        Database::create_tables_on_conn(&conn).unwrap();
        conn.execute("DROP TABLE IF EXISTS usage_sync_cursors", [])
            .unwrap();
        Database::set_user_version(&conn, 14).unwrap();

        let error = Database::apply_schema_migrations_on_conn_with_roots(&conn, &roots())
            .expect_err("incomplete v14 must fail closed");

        assert!(error.to_string().contains("incomplete schema v14"));
        assert!(!Database::table_exists(&conn, "usage_sync_cursors").unwrap());
        assert!(!Database::table_exists(&conn, "session_log_sync_v13_archive").unwrap());
    }

    #[test]
    fn malformed_v14_cursor_columns_and_primary_key_are_rejected() {
        let conn = Connection::open_in_memory().unwrap();
        Database::create_tables_on_conn(&conn).unwrap();
        conn.execute(
            "ALTER TABLE session_log_sync RENAME TO session_log_sync_v13_archive",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE usage_sync_cursors (source TEXT, cursor_key TEXT)",
            [],
        )
        .unwrap();
        Database::set_user_version(&conn, 14).unwrap();

        let error = Database::apply_schema_migrations_on_conn_with_roots(&conn, &roots())
            .expect_err("malformed cursor schema must fail closed");

        assert!(error.to_string().contains("usage_sync_cursors"));
    }

    #[test]
    fn malformed_v14_archive_columns_are_rejected() {
        let conn = Connection::open_in_memory().unwrap();
        Database::create_tables_on_conn(&conn).unwrap();
        conn.execute("DROP TABLE session_log_sync", []).unwrap();
        conn.execute(
            "CREATE TABLE session_log_sync_v13_archive (file_path TEXT PRIMARY KEY)",
            [],
        )
        .unwrap();
        Database::create_usage_sync_cursors_table_on_conn(&conn).unwrap();
        Database::set_user_version(&conn, 14).unwrap();

        let error = Database::apply_schema_migrations_on_conn_with_roots(&conn, &roots())
            .expect_err("malformed archive schema must fail closed");

        assert!(error.to_string().contains("session_log_sync_v13_archive"));
    }

    #[test]
    fn composite_v14_archive_primary_key_is_rejected() {
        let conn = Connection::open_in_memory().unwrap();
        Database::create_tables_on_conn(&conn).unwrap();
        conn.execute("DROP TABLE session_log_sync", []).unwrap();
        conn.execute_batch(
            "CREATE TABLE session_log_sync_v13_archive (
                 file_path TEXT NOT NULL,
                 last_modified INTEGER NOT NULL,
                 last_line_offset INTEGER NOT NULL DEFAULT 0,
                 last_synced_at INTEGER NOT NULL,
                 PRIMARY KEY (file_path, last_modified)
             );",
        )
        .unwrap();
        Database::create_usage_sync_cursors_table_on_conn(&conn).unwrap();
        Database::set_user_version(&conn, 14).unwrap();

        let error = Database::apply_schema_migrations_on_conn_with_roots(&conn, &roots())
            .expect_err("archive composite primary key must fail closed");

        assert!(error.to_string().contains("expected file_path primary key"));
    }

    #[test]
    fn rollback_failure_reports_uncertain_connection_state() {
        let conn = Connection::open_in_memory().unwrap();
        let error = Database::rollback_schema_migration_error(
            &conn,
            AppError::Database("original migration failure".to_string()),
        );
        let message = error.to_string();
        assert!(message.contains("original migration failure"));
        assert!(message.contains("state is uncertain"));
        assert!(message.contains("rollback="));
        assert!(message.contains("release="));
    }
}

mod schema_v15_dashboard_module_migration_tests {
    use super::*;
    use crate::usage::source_roots::UsageSourceRoots;
    use std::path::PathBuf;

    fn roots() -> UsageSourceRoots {
        UsageSourceRoots {
            claude: PathBuf::from("/Users/test/.claude/projects"),
            codex: PathBuf::from("/Users/test/.codex"),
            gemini: PathBuf::from("/Users/test/.gemini/tmp"),
            opencode: PathBuf::from("/Users/test/.local/share/opencode"),
        }
    }

    fn v14_usage_fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        Database::create_tables_on_conn(&conn).unwrap();
        crate::usage::migration::migrate_v12_to_v13(&conn).unwrap();
        crate::usage::cursor_migration::migrate_v13_to_v14(&conn, &roots()).unwrap();
        Database::set_user_version(&conn, 14).unwrap();
        conn
    }

    fn insert_provider(conn: &Connection, id: &str, billing_kind: &str, product_group_id: &str) {
        conn.execute(
            "INSERT INTO usage_providers (
                 id, name, billing_kind, product_group_id, token_sources,
                 enabled, needs_review, created_at, updated_at
             ) VALUES (?1, ?1, ?2, ?3, '[]', 1, 0, 10, 10)",
            params![id, billing_kind, product_group_id],
        )
        .unwrap();
    }

    #[test]
    fn migration_v14_to_v15_seeds_modules_and_backfills_known_providers() {
        let conn = v14_usage_fixture();
        insert_provider(&conn, "codex-plan", "subscription", "codex");
        insert_provider(&conn, "claude-plan", "subscription", "anthropic");
        insert_provider(&conn, "kimi-plan", "subscription", "moonshot");
        insert_provider(&conn, "openrouter", "metered", "openrouter");

        Database::apply_schema_migrations_on_conn_with_roots(&conn, &roots()).unwrap();

        assert_eq!(Database::get_user_version(&conn).unwrap(), SCHEMA_VERSION);
        let modules = conn
            .prepare(
                "SELECT id, name, kind, sort_order, visible, is_system
                 FROM dashboard_modules ORDER BY sort_order, id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            modules,
            vec![
                (
                    "codex".into(),
                    "Codex".into(),
                    "subscription".into(),
                    0,
                    1,
                    0
                ),
                (
                    "claude-code".into(),
                    "Claude Code".into(),
                    "subscription".into(),
                    1,
                    1,
                    0,
                ),
                (
                    "kimi-coding-plan".into(),
                    "Kimi Coding Plan".into(),
                    "subscription".into(),
                    2,
                    1,
                    0,
                ),
                ("api".into(), "API".into(), "api".into(), 3, 1, 1),
            ]
        );

        let memberships = conn
            .prepare("SELECT id, dashboard_module_id FROM usage_providers ORDER BY id")
            .unwrap()
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let mut expected_memberships = vec![
            ("claude-plan".into(), Some("claude-code".into())),
            ("codex-plan".into(), Some("codex".into())),
            ("kimi-plan".into(), Some("kimi-coding-plan".into())),
            ("openrouter".into(), None),
        ];
        expected_memberships.extend(
            crate::usage::system_providers::system_provider_definitions()
                .into_iter()
                .map(|definition| (definition.id.to_string(), None)),
        );
        expected_memberships.sort_by(|left, right| left.0.cmp(&right.0));
        assert_eq!(memberships, expected_memberships,);
    }

    #[test]
    fn migration_v14_to_v15_routes_unknown_subscription_to_review_module() {
        let conn = v14_usage_fixture();
        insert_provider(&conn, "unknown-plan", "subscription", "gemini");

        Database::apply_schema_migrations_on_conn_with_roots(&conn, &roots()).unwrap();

        let module = conn
            .query_row(
                "SELECT name, kind, is_system FROM dashboard_modules
                 WHERE id = 'other-subscriptions'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(module, ("其他订阅".into(), "subscription".into(), 0));
        let provider = conn
            .query_row(
                "SELECT dashboard_module_id, needs_review FROM usage_providers
                 WHERE id = 'unknown-plan'",
                [],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap();
        assert_eq!(provider, (Some("other-subscriptions".into()), 1));
    }

    #[test]
    fn migration_v14_to_v15_rolls_back_column_rows_and_version_on_seed_failure() {
        let conn = v14_usage_fixture();
        conn.execute_batch(
            "CREATE TABLE dashboard_modules (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL CHECK (name = 'forced failure'),
                 kind TEXT NOT NULL,
                 sort_order INTEGER NOT NULL,
                 visible INTEGER NOT NULL,
                 is_system INTEGER NOT NULL,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL
             );",
        )
        .unwrap();

        Database::apply_schema_migrations_on_conn_with_roots(&conn, &roots())
            .expect_err("default module seed must fail and roll back the outer savepoint");

        assert_eq!(Database::get_user_version(&conn).unwrap(), 14);
        assert!(!Database::has_column(&conn, "usage_providers", "dashboard_module_id").unwrap());
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM dashboard_modules", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(rows, 0);
    }
}

mod migration_v15_to_v16 {
    use super::*;
    use crate::usage::agent_module_migration::{
        migrate_v15_to_v16_with_failure, MigrationFailurePoint,
    };
    use crate::usage::source_roots::UsageSourceRoots;
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    pub(super) fn roots() -> UsageSourceRoots {
        UsageSourceRoots {
            claude: PathBuf::from("/Users/test/.claude/projects"),
            codex: PathBuf::from("/Users/test/.codex"),
            gemini: PathBuf::from("/Users/test/.gemini/tmp"),
            opencode: PathBuf::from("/Users/test/.local/share/opencode"),
        }
    }

    pub(super) fn v15_usage_fixture() -> Connection {
        let conn = Connection::open_in_memory().expect("open v15 fixture");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("enable foreign keys");
        Database::create_tables_on_conn(&conn).expect("create application tables");
        crate::usage::migration::migrate_v12_to_v13(&conn).expect("create v13 usage schema");
        crate::usage::cursor_migration::migrate_v13_to_v14(&conn, &roots())
            .expect("create v14 cursor schema");
        Database::set_user_version(&conn, 14).expect("set v14 version");
        crate::usage::module_migration::migrate_v14_to_v15(&conn)
            .expect("create v15 module schema");
        Database::set_user_version(&conn, 15).expect("set v15 version");
        conn
    }

    fn migrate_fixture_to_v16(conn: &Connection) {
        Database::validate_schema_v15_complete(conn).expect("validate complete v15 fixture");
        crate::usage::agent_module_migration::migrate_v15_to_v16(conn)
            .expect("migrate fixture from v15 to v16");
        Database::set_user_version(conn, 16).expect("set v16 fixture version");
        crate::usage::agent_module_migration::validate_schema_v16_complete(conn)
            .expect("validate complete v16 fixture");
    }

    fn migrate_v12_fixture_to_v16(conn: &Connection) {
        crate::usage::migration::migrate_v12_to_v13(conn).expect("migrate fixture to v13");
        Database::set_user_version(conn, 13).expect("set v13 fixture version");
        crate::usage::cursor_migration::migrate_v13_to_v14(conn, &roots())
            .expect("migrate fixture to v14");
        Database::set_user_version(conn, 14).expect("set v14 fixture version");
        crate::usage::module_migration::migrate_v14_to_v15(conn).expect("migrate fixture to v15");
        Database::set_user_version(conn, 15).expect("set v15 fixture version");
        migrate_fixture_to_v16(conn);
    }

    fn insert_provider(
        conn: &Connection,
        id: &str,
        billing_kind: &str,
        dashboard_module_id: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO usage_providers (
                 id, name, billing_kind, product_group_id, token_sources,
                 dashboard_module_id, enabled, needs_review, created_at, updated_at
             ) VALUES (?1, ?1, ?2, ?1, '[\"session_log\"]', ?3, 1, 0, 10, 10)",
            params![id, billing_kind, dashboard_module_id],
        )
        .expect("insert usage provider");
    }

    fn insert_event(
        conn: &Connection,
        event_id: &str,
        source: &str,
        legacy_request_id: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO usage_events (
                 event_id, source, provider_id, product_group_id, occurred_at,
                 model, cost_source, legacy_request_id, created_at
             ) VALUES (?1, ?2, 'history-provider', 'history', 100,
                       'model', 'unavailable', ?3, 100)",
            params![event_id, source, legacy_request_id],
        )
        .expect("insert usage event");
    }

    fn insert_legacy_log(
        conn: &Connection,
        request_id: &str,
        app_type: &str,
        provider_type: Option<&str>,
        data_source: &str,
    ) {
        conn.execute(
            "INSERT INTO proxy_request_logs (
                 request_id, provider_id, app_type, model, latency_ms,
                 status_code, provider_type, created_at, data_source
             ) VALUES (?1, 'legacy-provider', ?2, 'model', 1, 200, ?3, 100, ?4)",
            params![request_id, app_type, provider_type, data_source],
        )
        .expect("insert legacy request log");
    }

    fn agent_for(conn: &Connection, event_id: &str) -> Option<String> {
        conn.query_row(
            "SELECT agent_module_id FROM usage_events WHERE event_id = ?1",
            [event_id],
            |row| row.get(0),
        )
        .expect("query migrated agent identity")
    }

    fn trigger_sql(conn: &Connection, trigger: &str) -> String {
        conn.query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'trigger' AND name = ?1",
            [trigger],
            |row| row.get(0),
        )
        .expect("query trigger SQL")
    }

    fn table_columns(conn: &Connection, table: &str) -> HashSet<String> {
        let mut statement = conn
            .prepare(&format!("PRAGMA table_info('{table}')"))
            .expect("inspect table columns");
        statement
            .query_map([], |row| row.get(1))
            .expect("query table columns")
            .collect::<Result<_, _>>()
            .expect("collect table columns")
    }

    #[test]
    fn migration_v15_to_v16_seeds_agents_preserves_custom_modules_and_backfills_only_proven_bindings(
    ) {
        let conn = v15_usage_fixture();
        conn.execute(
            "INSERT INTO dashboard_modules (
                 id, name, kind, sort_order, visible, is_system, created_at, updated_at
             ) VALUES ('custom-workbench', 'Custom Workbench', 'subscription', 7, 0, 0, 10, 10)",
            [],
        )
        .expect("insert custom v15 module");
        insert_provider(&conn, "codex-source", "subscription", Some("api"));
        insert_provider(&conn, "claude-source", "subscription", Some("claude-code"));
        insert_provider(
            &conn,
            "custom-provider",
            "subscription",
            Some("custom-workbench"),
        );
        insert_provider(
            &conn,
            "kimi-provider",
            "subscription",
            Some("kimi-coding-plan"),
        );
        insert_provider(&conn, "codex-default-only", "subscription", Some("codex"));
        insert_provider(
            &conn,
            "claude-default-only",
            "subscription",
            Some("claude-code"),
        );
        conn.execute_batch(
            "UPDATE dashboard_modules SET sort_order = 9, visible = 0 WHERE id = 'codex';
             INSERT INTO usage_source_bindings (source, provider_id, updated_at)
             VALUES ('codex', 'codex-source', 10), ('claude', 'claude-source', 10);",
        )
        .expect("seed exact source binding evidence");

        let provider_count = super::count(&conn, "usage_providers");
        migrate_fixture_to_v16(&conn);

        assert_eq!(Database::get_user_version(&conn).unwrap(), 16);
        assert_eq!(super::count(&conn, "usage_providers"), provider_count);
        assert_eq!(super::count(&conn, "dashboard_modules"), 5);
        assert_eq!(
            super::scalar_i64(
                &conn,
                "SELECT COUNT(*) FROM dashboard_modules WHERE id = 'api' AND kind = 'api'"
            ),
            1,
            "v15 display data remains frozen for compatibility"
        );

        let fixed = conn
            .prepare(
                "SELECT id, name, sort_order, visible
                 FROM agent_modules WHERE is_fixed = 1 ORDER BY id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            fixed,
            vec![
                ("claude-code".into(), "Claude Code".into(), 1, 1),
                ("codex".into(), "Codex".into(), 9, 0),
                ("hermes".into(), "Hermes".into(), 4, 1),
                ("openclaw".into(), "OpenClaw".into(), 3, 1),
                ("opencode".into(), "OpenCode".into(), 2, 1),
            ]
        );
        assert_eq!(
            super::scalar_i64(
                &conn,
                "SELECT COUNT(*) FROM agent_modules WHERE id IN ('api', 'other-subscriptions')"
            ),
            0
        );

        let custom_rows = conn
            .prepare(
                "SELECT id, name, sort_order, visible, is_fixed
                 FROM agent_modules WHERE is_fixed = 0 ORDER BY id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            custom_rows,
            vec![
                (
                    "custom-workbench".into(),
                    "Custom Workbench".into(),
                    7,
                    0,
                    0,
                ),
                (
                    "kimi-coding-plan".into(),
                    "Kimi Coding Plan".into(),
                    2,
                    1,
                    0,
                ),
            ]
        );

        let bindings = conn
            .prepare(
                "SELECT id, agent_module_id, provider_id, enabled,
                        api_key_fingerprint, credential_slot, credential_version
                 FROM agent_provider_bindings ORDER BY agent_module_id, provider_id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<Vec<u8>>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(bindings.len(), 3);
        assert_eq!(
            bindings
                .iter()
                .map(|row| (row.1.as_str(), row.2.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("claude-code", "claude-source"),
                ("codex", "codex-source"),
                ("custom-workbench", "custom-provider"),
            ]
        );
        for (id, _, _, enabled, fingerprint, slot, version) in &bindings {
            uuid::Uuid::parse_str(id).expect("migration binding id is a UUID");
            assert_eq!(*enabled, 0);
            assert_eq!(fingerprint, &None);
            assert_eq!(slot, &None);
            assert_eq!(*version, 0);
        }
        assert_eq!(
            super::scalar_i64(
                &conn,
                "SELECT COUNT(*) FROM agent_provider_bindings
                 WHERE provider_id IN (
                     'kimi-provider', 'codex-default-only', 'claude-default-only'
                 )"
            ),
            0,
            "legacy default membership is not independent Agent evidence"
        );

        assert!(conn
            .execute(
                "UPDATE agent_provider_bindings
                 SET api_key_fingerprint = ?1,
                     credential_slot = 'text-fingerprint',
                     credential_version = 1
                 WHERE id = ?2",
                params!["x".repeat(32), &bindings[0].0],
            )
            .is_err());
        assert!(conn
            .execute(
                "UPDATE agent_provider_bindings
                 SET api_key_fingerprint = NULL,
                     credential_slot = 'orphan-slot',
                     credential_version = 1
                 WHERE id = ?1",
                [&bindings[0].0],
            )
            .is_err());
        assert!(conn
            .execute(
                "UPDATE agent_provider_bindings
                 SET api_key_fingerprint = ?1,
                     credential_slot = NULL,
                     credential_version = 1
                 WHERE id = ?2",
                params![vec![7_u8; 32], &bindings[0].0],
            )
            .is_err());
        assert!(conn
            .execute(
                "UPDATE agent_provider_bindings
                 SET credential_version = -1
                 WHERE id = ?1",
                [&bindings[0].0],
            )
            .is_err());

        let binding_columns = table_columns(&conn, "agent_provider_bindings");
        for required in [
            "id",
            "agent_module_id",
            "provider_id",
            "enabled",
            "api_key_fingerprint",
            "credential_slot",
            "credential_version",
            "created_at",
            "updated_at",
        ] {
            assert!(binding_columns.contains(required), "missing {required}");
        }
        assert!(Database::table_exists(&conn, "agent_credential_operations").unwrap());

        let agent_event_column = conn
            .query_row(
                "SELECT \"notnull\", dflt_value
                 FROM pragma_table_info('usage_events')
                 WHERE name = 'agent_module_id'",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .expect("inspect nullable event Agent column");
        assert_eq!(agent_event_column, (0, None));
        for table in [
            "agent_modules",
            "agent_provider_bindings",
            "agent_credential_operations",
        ] {
            assert_eq!(
                super::scalar_i64(
                    &conn,
                    &format!("SELECT \"notnull\" FROM pragma_table_info('{table}') WHERE pk = 1")
                ),
                1,
                "{table} stable identity must be explicitly NOT NULL"
            );
        }

        assert!(conn
            .execute(
                "INSERT INTO agent_modules (
                     id, name, sort_order, visible, is_fixed, archived_at,
                     created_at, updated_at
                 ) VALUES (NULL, 'Null Agent', 10, 1, 0, NULL, 10, 10)",
                [],
            )
            .is_err());
        assert!(conn
            .execute(
                "INSERT INTO agent_provider_bindings (
                     id, agent_module_id, provider_id, enabled,
                     api_key_fingerprint, credential_slot, credential_version,
                     created_at, updated_at
                 ) VALUES (
                     NULL, 'codex', 'codex-source', 0,
                     NULL, NULL, 0, 10, 10
                 )",
                [],
            )
            .is_err());
        assert!(conn
            .execute(
                "INSERT INTO agent_credential_operations (
                     operation_id, binding_id, generation, operation_kind,
                     status, staging_slot, previous_slot, created_at, updated_at
                 ) VALUES (NULL, ?1, 1, 'set', 'pending', 'slot', NULL, 10, 10)",
                [&bindings[0].0],
            )
            .is_err());
        assert!(conn
            .execute(
                "INSERT INTO agent_credential_operations (
                     operation_id, binding_id, generation, operation_kind,
                     status, staging_slot, previous_slot, created_at, updated_at
                 ) VALUES ('zero-generation', ?1, 0, 'set', 'pending', 'slot', NULL, 10, 10)",
                [&bindings[0].0],
            )
            .is_err());
        assert!(conn
            .execute(
                "INSERT INTO agent_credential_operations (
                     operation_id, binding_id, generation, operation_kind,
                     status, staging_slot, previous_slot, created_at, updated_at
                 ) VALUES ('missing-staging', ?1, 1, 'set', 'pending', NULL, NULL, 10, 10)",
                [&bindings[0].0],
            )
            .is_err());

        conn.execute(
            "UPDATE agent_modules SET sort_order = 0, visible = 1 WHERE id = 'codex'",
            [],
        )
        .expect("fixed Agent can be reordered and shown");
        assert!(conn
            .execute(
                "UPDATE agent_modules SET name = 'Renamed' WHERE id = 'codex'",
                []
            )
            .is_err());
        assert!(conn
            .execute(
                "UPDATE agent_modules SET archived_at = 20 WHERE id = 'codex'",
                []
            )
            .is_err());
        assert!(conn
            .execute(
                "UPDATE agent_modules SET created_at = 20 WHERE id = 'codex'",
                []
            )
            .is_err());
        assert!(conn
            .execute(
                "INSERT INTO agent_modules (
                     id, name, sort_order, visible, is_fixed,
                     archived_at, created_at, updated_at
                 ) VALUES ('custom-fixed', 'Custom Fixed', 20, 1, 1, NULL, 20, 20)",
                [],
            )
            .is_err());
        assert!(conn
            .execute(
                "INSERT INTO agent_modules (
                     id, name, sort_order, visible, is_fixed,
                     archived_at, created_at, updated_at
                 ) VALUES ('api', 'API', 20, 1, 0, NULL, 20, 20)",
                [],
            )
            .is_err());
        assert!(conn
            .execute("DELETE FROM agent_modules WHERE id = 'codex'", [])
            .is_err());

        let unused = v15_usage_fixture();
        migrate_fixture_to_v16(&unused);
        assert_eq!(super::count(&unused, "usage_providers"), 0);
        assert_eq!(super::count(&unused, "usage_events"), 0);
        assert_eq!(
            super::scalar_i64(
                &unused,
                "SELECT COUNT(*) FROM agent_modules WHERE id = 'kimi-coding-plan'"
            ),
            0,
            "an untouched, unused Kimi default is omitted"
        );
    }

    #[test]
    fn migration_v15_to_v16_attributes_only_exact_history_and_nulls_uncertain_link_components() {
        let conn = v15_usage_fixture();
        insert_provider(&conn, "history-provider", "metered", None);

        insert_event(
            &conn,
            "exact-claude-session",
            "session_log",
            Some("log-claude-session"),
        );
        insert_legacy_log(
            &conn,
            "log-claude-session",
            "claude",
            Some("session_log"),
            "session",
        );
        insert_event(
            &conn,
            "exact-codex-session",
            "session_log",
            Some("log-codex-session"),
        );
        insert_legacy_log(
            &conn,
            "log-codex-session",
            "codex",
            Some("codex_session"),
            "codex_session",
        );
        insert_event(
            &conn,
            "compat-claude-session",
            "session_log",
            Some("log-compat-claude-session"),
        );
        insert_legacy_log(
            &conn,
            "log-compat-claude-session",
            "claude",
            Some("session_log"),
            "session_log",
        );
        insert_event(
            &conn,
            "canonical-codex-session",
            "session_log",
            Some("log-canonical-codex-session"),
        );
        insert_legacy_log(
            &conn,
            "log-canonical-codex-session",
            "codex",
            Some("codex_session"),
            "session",
        );
        insert_event(&conn, "claude-session:prefix", "session_log", None);
        insert_event(&conn, "codex-session:prefix", "session_log", None);
        insert_event(&conn, "Claude-session:wrong-case", "session_log", None);
        insert_event(&conn, "claude-session:", "session_log", None);
        insert_event(&conn, "bare-session", "session_log", None);

        for (event_id, app_type, provider_type, data_source) in [
            (
                "wrong-claude-compat-pair",
                "claude",
                "session_log",
                "codex_session",
            ),
            (
                "wrong-codex-compat-pair",
                "codex",
                "codex_session",
                "session_log",
            ),
            ("wrong-session-app-case", "Claude", "session_log", "session"),
        ] {
            let request_id = format!("log-{event_id}");
            insert_event(&conn, event_id, "session_log", Some(&request_id));
            insert_legacy_log(
                &conn,
                &request_id,
                app_type,
                Some(provider_type),
                data_source,
            );
        }

        insert_provider(&conn, "anthropic", "metered", None);
        conn.execute(
            "INSERT INTO usage_events (
                 event_id, source, provider_id, product_group_id, occurred_at,
                 model, cost_source, created_at
             ) VALUES (
                 'vendor-product-only', 'session_log', 'anthropic', 'anthropic',
                 100, 'model', 'unavailable', 100
             )",
            [],
        )
        .expect("insert vendor/product-only history");

        insert_event(
            &conn,
            "claude-session:direct-conflict",
            "session_log",
            Some("log-direct-conflict"),
        );
        insert_legacy_log(
            &conn,
            "log-direct-conflict",
            "codex",
            Some("codex_session"),
            "session",
        );

        for (event_id, app_type) in [
            ("proxy-codex", "codex"),
            ("proxy-claude", "claude"),
            ("proxy-opencode", " OpenCode "),
            ("proxy-openclaw", "openclaw"),
            ("proxy-hermes", "HERMES"),
        ] {
            let request_id = format!("log-{event_id}");
            insert_event(&conn, event_id, "proxy", Some(&request_id));
            insert_legacy_log(&conn, &request_id, app_type, None, "proxy");
        }
        for app_type in ["openai", "anthropic", "moonshot", "kimi", "api"] {
            let event_id = format!("unproven-{app_type}");
            let request_id = format!("log-{event_id}");
            insert_event(&conn, &event_id, "proxy", Some(&request_id));
            insert_legacy_log(&conn, &request_id, app_type, None, "proxy");
        }
        insert_event(
            &conn,
            "wrong-session-label",
            "session_log",
            Some("log-wrong-session"),
        );
        insert_legacy_log(
            &conn,
            "log-wrong-session",
            "anthropic",
            Some("session_log"),
            "session_log",
        );

        for event_id in [
            "claude-session:conflict-a",
            "claude-session:conflict-b",
            "codex-session:conflict-c",
            "claude-session:one-sided",
            "one-sided-unknown",
            "reverse-unknown-canonical",
            "claude-session:reverse-proven-duplicate",
            "claude-session:agree-a",
            "claude-session:agree-b",
        ] {
            insert_event(&conn, event_id, "session_log", None);
        }
        conn.execute_batch(
            "INSERT INTO usage_event_links (
                 canonical_event_id, duplicate_event_id, link_kind, link_value, created_at
             ) VALUES
             ('claude-session:conflict-a', 'claude-session:conflict-b', 'session_id', 'conflict-1', 100),
             ('claude-session:conflict-b', 'codex-session:conflict-c', 'session_id', 'conflict-2', 100),
             ('codex-session:conflict-c', 'claude-session:conflict-a', 'request_id', 'conflict-cycle', 100),
             ('claude-session:conflict-a', 'claude-session:conflict-b', 'request_id', 'conflict-multi-edge', 100),
             ('claude-session:one-sided', 'one-sided-unknown', 'session_id', 'one-sided', 100),
             ('reverse-unknown-canonical', 'claude-session:reverse-proven-duplicate', 'session_id', 'reverse', 100),
             ('claude-session:agree-a', 'claude-session:agree-b', 'session_id', 'agree', 100);
             INSERT INTO quota_snapshots (
                 snapshot_id, provider_id, fetched_at, raw_payload, created_at
             ) VALUES ('quota-history', 'history-provider', 100, '{}', 100);
             INSERT INTO quota_fetch_state (provider_id, stale)
             VALUES ('history-provider', 0);",
        )
        .expect("seed linked history and quota rows");

        let before = HashMap::from([
            ("providers", super::count(&conn, "usage_providers")),
            ("events", super::count(&conn, "usage_events")),
            ("links", super::count(&conn, "usage_event_links")),
            ("quota", super::count(&conn, "quota_snapshots")),
            ("quota_state", super::count(&conn, "quota_fetch_state")),
        ]);
        let delete_trigger_before = trigger_sql(&conn, "usage_events_immutable_delete");

        migrate_fixture_to_v16(&conn);

        assert_eq!(
            agent_for(&conn, "exact-claude-session").as_deref(),
            Some("claude-code")
        );
        assert_eq!(
            agent_for(&conn, "exact-codex-session").as_deref(),
            Some("codex")
        );
        assert_eq!(
            agent_for(&conn, "compat-claude-session").as_deref(),
            Some("claude-code")
        );
        assert_eq!(
            agent_for(&conn, "canonical-codex-session").as_deref(),
            Some("codex")
        );
        assert_eq!(
            agent_for(&conn, "claude-session:prefix").as_deref(),
            Some("claude-code")
        );
        assert_eq!(
            agent_for(&conn, "codex-session:prefix").as_deref(),
            Some("codex")
        );
        assert_eq!(agent_for(&conn, "proxy-codex").as_deref(), Some("codex"));
        assert_eq!(
            agent_for(&conn, "proxy-claude").as_deref(),
            Some("claude-code")
        );
        assert_eq!(
            agent_for(&conn, "proxy-opencode").as_deref(),
            Some("opencode")
        );
        assert_eq!(
            agent_for(&conn, "proxy-openclaw").as_deref(),
            Some("openclaw")
        );
        assert_eq!(agent_for(&conn, "proxy-hermes").as_deref(), Some("hermes"));

        for event_id in [
            "bare-session",
            "Claude-session:wrong-case",
            "claude-session:",
            "wrong-claude-compat-pair",
            "wrong-codex-compat-pair",
            "wrong-session-app-case",
            "vendor-product-only",
            "claude-session:direct-conflict",
            "wrong-session-label",
            "unproven-openai",
            "unproven-anthropic",
            "unproven-moonshot",
            "unproven-kimi",
            "unproven-api",
            "claude-session:conflict-a",
            "claude-session:conflict-b",
            "codex-session:conflict-c",
            "one-sided-unknown",
            "reverse-unknown-canonical",
        ] {
            assert_eq!(
                agent_for(&conn, event_id),
                None,
                "{event_id} must stay unassigned"
            );
        }
        assert_eq!(
            agent_for(&conn, "claude-session:one-sided").as_deref(),
            Some("claude-code"),
            "links never propagate identity to an unproven neighbor"
        );
        assert_eq!(
            agent_for(&conn, "claude-session:agree-a").as_deref(),
            Some("claude-code")
        );
        assert_eq!(
            agent_for(&conn, "claude-session:agree-b").as_deref(),
            Some("claude-code")
        );
        assert_eq!(
            agent_for(&conn, "claude-session:reverse-proven-duplicate").as_deref(),
            Some("claude-code"),
            "reverse traversal audits the component without propagating identity"
        );

        assert_eq!(super::count(&conn, "usage_providers"), before["providers"]);
        assert_eq!(super::count(&conn, "usage_events"), before["events"]);
        assert_eq!(super::count(&conn, "usage_event_links"), before["links"]);
        assert_eq!(super::count(&conn, "quota_snapshots"), before["quota"]);
        assert_eq!(
            super::count(&conn, "quota_fetch_state"),
            before["quota_state"]
        );
        assert_eq!(
            trigger_sql(&conn, "usage_events_immutable_delete"),
            delete_trigger_before,
            "the append-only DELETE trigger is preserved byte-for-byte"
        );
        assert!(conn
            .execute(
                "UPDATE usage_events SET agent_module_id = NULL WHERE event_id = 'proxy-codex'",
                []
            )
            .is_err());
        assert!(conn
            .execute(
                "DELETE FROM usage_events WHERE event_id = 'proxy-codex'",
                []
            )
            .is_err());
    }

    #[test]
    fn migration_v15_to_v16_continues_v12_chain_and_validates_complete_schema() {
        let conn = super::true_v12_usage_fixture();
        migrate_v12_fixture_to_v16(&conn);
        assert_eq!(Database::get_user_version(&conn).unwrap(), 16);
        assert!(Database::table_exists(&conn, "agent_modules").unwrap());
        assert!(Database::table_exists(&conn, "agent_provider_bindings").unwrap());
        assert!(Database::has_column(&conn, "usage_events", "agent_module_id").unwrap());

        let incomplete = v15_usage_fixture();
        Database::set_user_version(&incomplete, 16).expect("pretend incomplete v16");
        let error = Database::apply_schema_migrations_on_conn_with_roots(&incomplete, &roots())
            .expect_err("v16 completeness validation must fail closed");
        assert!(error.to_string().contains("incomplete schema v16"));
    }

    #[test]
    fn migration_v15_to_v16_rejects_duplicate_non_null_credential_slots() {
        let conn = v15_usage_fixture();
        insert_provider(&conn, "slot-provider-a", "subscription", Some("codex"));
        insert_provider(
            &conn,
            "slot-provider-b",
            "subscription",
            Some("claude-code"),
        );
        conn.execute_batch(
            "INSERT INTO usage_source_bindings (source, provider_id, updated_at)
             VALUES ('codex', 'slot-provider-a', 10),
                    ('claude', 'slot-provider-b', 10);",
        )
        .expect("seed binding evidence");
        migrate_fixture_to_v16(&conn);

        let binding_ids = conn
            .prepare("SELECT id FROM agent_provider_bindings ORDER BY id")
            .expect("prepare binding query")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query bindings")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect bindings");
        assert_eq!(binding_ids.len(), 2);

        conn.execute(
            "UPDATE agent_provider_bindings
             SET api_key_fingerprint = ?1,
                 credential_slot = 'shared-keychain-slot',
                 credential_version = 1
             WHERE id = ?2",
            params![vec![1_u8; 32], &binding_ids[0]],
        )
        .expect("assign first credential slot");
        let error = conn
            .execute(
                "UPDATE agent_provider_bindings
                 SET api_key_fingerprint = ?1,
                     credential_slot = 'shared-keychain-slot',
                     credential_version = 1
                 WHERE id = ?2",
                params![vec![2_u8; 32], &binding_ids[1]],
            )
            .expect_err("non-null credential slots must be globally unique");
        assert!(error.to_string().contains("UNIQUE constraint failed"));
    }

    #[test]
    fn current_v16_validator_rejects_same_named_but_malformed_objects() {
        for corruption in [
            "DROP INDEX IF EXISTS idx_agent_provider_bindings_credential_slot;
             CREATE INDEX idx_agent_provider_bindings_credential_slot
             ON agent_provider_bindings(provider_id);",
            "DROP INDEX idx_agent_provider_bindings_fingerprint;
             CREATE INDEX idx_agent_provider_bindings_fingerprint
             ON agent_provider_bindings(provider_id);",
            "DROP TRIGGER usage_events_immutable_update;
             CREATE TRIGGER usage_events_immutable_update
             AFTER INSERT ON usage_events
             BEGIN
                 SELECT 1;
             END;",
            "DROP TRIGGER usage_events_immutable_update;
             CREATE TRIGGER usage_events_immutable_update
             BEFORE UPDATE ON usage_events
             WHEN 0
             BEGIN
                 SELECT RAISE(ABORT, 'usage_events are immutable');
             END;",
            "DROP TRIGGER agent_modules_fixed_delete;
             CREATE TRIGGER agent_modules_fixed_delete
             BEFORE DELETE ON agent_modules
             WHEN OLD.is_fixed = 1
             BEGIN
                 SELECT 1;
             END;",
            "DROP INDEX idx_agent_credential_operations_pending_binding;
             CREATE UNIQUE INDEX idx_agent_credential_operations_pending_binding
             ON agent_credential_operations(binding_id)
             WHERE status = 'pending' AND 0;",
            "DROP TRIGGER agent_modules_identity_immutable;
             CREATE TRIGGER agent_modules_identity_immutable
             BEFORE UPDATE OF id ON agent_modules
             WHEN NEW.id IS NOT OLD.id AND 0
             BEGIN
                 SELECT RAISE(ABORT, 'agent module identity cannot be changed');
             END;",
        ] {
            let conn = v15_usage_fixture();
            migrate_fixture_to_v16(&conn);
            conn.execute_batch(corruption)
                .expect("replace required object with malformed namesake");

            let error = Database::apply_schema_migrations_on_conn_with_roots(&conn, &roots())
                .expect_err("v16 validator must reject a malformed namesake");
            assert!(error.to_string().contains("incomplete schema v16"));
        }
    }

    #[test]
    fn migration_v15_to_v16_rolls_back_column_rows_tables_and_triggers_at_every_failure_point() {
        for failure_point in [
            MigrationFailurePoint::AfterTriggerDrop,
            MigrationFailurePoint::DuringHistoryBackfill,
            MigrationFailurePoint::BeforeTriggerRecreation,
        ] {
            let conn = v15_usage_fixture();
            insert_provider(&conn, "history-provider", "metered", None);
            insert_event(&conn, "claude-session:rollback", "session_log", None);
            let update_trigger_before = trigger_sql(&conn, "usage_events_immutable_update");
            let delete_trigger_before = trigger_sql(&conn, "usage_events_immutable_delete");
            let event_count = super::count(&conn, "usage_events");

            conn.execute("SAVEPOINT schema_migration", [])
                .expect("start migration savepoint");
            let cause = migrate_v15_to_v16_with_failure(&conn, failure_point)
                .expect_err("injected v16 failure must abort migration");
            let error = Database::rollback_schema_migration_error(&conn, cause);
            assert!(error.to_string().contains("injected v16 migration failure"));

            assert_eq!(Database::get_user_version(&conn).unwrap(), 15);
            assert!(!Database::has_column(&conn, "usage_events", "agent_module_id").unwrap());
            assert!(!Database::table_exists(&conn, "agent_modules").unwrap());
            assert!(!Database::table_exists(&conn, "agent_provider_bindings").unwrap());
            assert!(!Database::table_exists(&conn, "agent_credential_operations").unwrap());
            assert_eq!(super::count(&conn, "usage_events"), event_count);
            assert_eq!(
                trigger_sql(&conn, "usage_events_immutable_update"),
                update_trigger_before
            );
            assert_eq!(
                trigger_sql(&conn, "usage_events_immutable_delete"),
                delete_trigger_before
            );
            assert!(conn
                .execute(
                    "UPDATE usage_events SET model = 'changed' WHERE event_id = 'claude-session:rollback'",
                    []
                )
                .is_err());
        }
    }
}

mod migration_v16_to_v17 {
    use super::*;

    fn v16_usage_fixture() -> Connection {
        let conn = super::migration_v15_to_v16::v15_usage_fixture();
        crate::usage::agent_module_migration::migrate_v15_to_v16(&conn)
            .expect("create complete v16 fixture");
        Database::set_user_version(&conn, 16).expect("set v16 fixture version");
        assert_eq!(Database::get_user_version(&conn).unwrap(), 16);
        conn
    }

    #[test]
    fn migration_v16_to_v17_seeds_complete_system_catalog_and_preserves_custom_rows() {
        let conn = v16_usage_fixture();
        conn.execute(
            "INSERT INTO usage_providers (
                 id, name, billing_kind, product_group_id, token_sources,
                 quota_source, quota_interval_seconds, route_app_type, route_config,
                 quota_config, enabled, needs_review, legacy_app_type,
                 legacy_provider_id, created_at, updated_at
             ) VALUES (
                 'custom-provider', 'Custom Provider', 'metered', 'custom-product',
                 '[\"proxy\",\"session_log\"]', NULL, NULL, 'gemini',
                 '{\"baseUrl\":\"https://custom.example/v1\"}', NULL,
                 0, 1, NULL, NULL, 41, 42
             )",
            [],
        )
        .expect("insert custom v16 provider");
        conn.execute(
            "INSERT INTO usage_source_bindings (source, provider_id, updated_at)
             VALUES ('codex', 'custom-provider', 45)",
            [],
        )
        .expect("insert existing Codex source owner");
        conn.execute(
            "INSERT INTO agent_provider_bindings (
                 id, agent_module_id, provider_id, enabled,
                 api_key_fingerprint, credential_slot, credential_version,
                 created_at, updated_at
             ) VALUES (
                 'custom-binding', 'codex', 'custom-provider', 0,
                 NULL, NULL, 0, 43, 44
             )",
            [],
        )
        .expect("insert custom v16 binding");

        Database::apply_schema_migrations_on_conn_with_roots(
            &conn,
            &super::migration_v15_to_v16::roots(),
        )
        .expect("migrate v16 to v17");

        assert_eq!(Database::get_user_version(&conn).unwrap(), SCHEMA_VERSION);

        let system_rows = conn
            .prepare(
                "SELECT id, system_preset_key, name, billing_kind
                 FROM usage_providers
                 WHERE system_preset_key IS NOT NULL
                 ORDER BY system_preset_key",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            system_rows,
            vec![
                (
                    "system-anthropic-api".into(),
                    "anthropic-api".into(),
                    "Anthropic API".into(),
                    "metered".into(),
                ),
                (
                    "system-cerebras-api".into(),
                    "cerebras-api".into(),
                    "Cerebras API".into(),
                    "metered".into(),
                ),
                (
                    "system-chatgpt-subscription".into(),
                    "chatgpt-subscription".into(),
                    "ChatGPT".into(),
                    "subscription".into(),
                ),
                (
                    "system-claude-subscription".into(),
                    "claude-subscription".into(),
                    "Claude Pro/Max".into(),
                    "subscription".into(),
                ),
                (
                    "system-deepseek-api".into(),
                    "deepseek-api".into(),
                    "DeepSeek API".into(),
                    "metered".into(),
                ),
                (
                    "system-fireworks-api".into(),
                    "fireworks-api".into(),
                    "Fireworks AI".into(),
                    "metered".into(),
                ),
                (
                    "system-gemini-api".into(),
                    "gemini-api".into(),
                    "Google Gemini API".into(),
                    "metered".into(),
                ),
                (
                    "system-glm-api".into(),
                    "glm-api".into(),
                    "GLM / Z.AI API".into(),
                    "metered".into(),
                ),
                (
                    "system-groq-api".into(),
                    "groq-api".into(),
                    "Groq API".into(),
                    "metered".into(),
                ),
                (
                    "system-kimi-api".into(),
                    "kimi-api".into(),
                    "Kimi / Moonshot API".into(),
                    "metered".into(),
                ),
                (
                    "system-minimax-api".into(),
                    "minimax-api".into(),
                    "MiniMax API".into(),
                    "metered".into(),
                ),
                (
                    "system-mistral-api".into(),
                    "mistral-api".into(),
                    "Mistral AI API".into(),
                    "metered".into(),
                ),
                (
                    "system-nvidia-nim-api".into(),
                    "nvidia-nim-api".into(),
                    "NVIDIA NIM API".into(),
                    "metered".into(),
                ),
                (
                    "system-openai-api".into(),
                    "openai-api".into(),
                    "OpenAI API".into(),
                    "metered".into(),
                ),
                (
                    "system-openrouter-api".into(),
                    "openrouter-api".into(),
                    "OpenRouter".into(),
                    "metered".into(),
                ),
                (
                    "system-perplexity-api".into(),
                    "perplexity-api".into(),
                    "Perplexity API".into(),
                    "metered".into(),
                ),
                (
                    "system-qwen-api".into(),
                    "qwen-api".into(),
                    "Qwen / DashScope API".into(),
                    "metered".into(),
                ),
                (
                    "system-siliconflow-api".into(),
                    "siliconflow-api".into(),
                    "SiliconFlow API".into(),
                    "metered".into(),
                ),
                (
                    "system-together-api".into(),
                    "together-api".into(),
                    "Together AI".into(),
                    "metered".into(),
                ),
                (
                    "system-xai-api".into(),
                    "xai-api".into(),
                    "xAI (Grok) API".into(),
                    "metered".into(),
                ),
            ]
        );

        let default_bindings = conn
            .prepare(
                "SELECT agent_module_id, provider_id, route_protocol
                 FROM agent_provider_bindings
                 WHERE provider_id LIKE 'system-%'
                 ORDER BY agent_module_id, provider_id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            default_bindings,
            vec![
                (
                    "claude-code".into(),
                    "system-claude-subscription".into(),
                    None,
                ),
                (
                    "codex".into(),
                    "system-chatgpt-subscription".into(),
                    Some("codex".into()),
                ),
                (
                    "hermes".into(),
                    "system-openrouter-api".into(),
                    Some("hermes".into()),
                ),
                (
                    "openclaw".into(),
                    "system-openrouter-api".into(),
                    Some("openclaw".into()),
                ),
                (
                    "opencode".into(),
                    "system-openrouter-api".into(),
                    Some("opencode".into()),
                ),
            ]
        );
        assert_eq!(
            super::scalar_i64(
                &conn,
                "SELECT COUNT(*) FROM agent_provider_bindings
                 WHERE provider_id IN ('system-openai-api','system-anthropic-api')",
            ),
            0
        );
        assert_eq!(super::count(&conn, "provider_api_credentials"), 18);
        assert_eq!(
            super::scalar_i64(
                &conn,
                "SELECT COUNT(*) FROM provider_api_credentials
                 WHERE provider_id IN (
                     'system-openai-api','system-anthropic-api','system-openrouter-api'
                 )
                   AND api_key_fingerprint IS NULL
                   AND credential_slot IS NULL
                   AND credential_version = 0",
            ),
            3
        );
        assert_eq!(
            super::scalar_i64(
                &conn,
                "SELECT COUNT(*) FROM usage_providers
                 WHERE system_preset_key IN (
                     'gemini-api','xai-api','deepseek-api','kimi-api','glm-api',
                     'qwen-api','minimax-api','mistral-api','groq-api','together-api',
                     'fireworks-api','perplexity-api','siliconflow-api',
                     'nvidia-nim-api','cerebras-api'
                 ) AND enabled = 0",
            ),
            15
        );
        assert_eq!(
            conn.query_row(
                "SELECT name, billing_kind, product_group_id, token_sources,
                        route_app_type, route_config, enabled, needs_review,
                        created_at, updated_at, system_preset_key
                 FROM usage_providers WHERE id = 'custom-provider'",
                [],
                |row| Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, Option<String>>(10)?,
                )),
            )
            .unwrap(),
            (
                "Custom Provider".into(),
                "metered".into(),
                "custom-product".into(),
                "[\"proxy\",\"session_log\"]".into(),
                Some("gemini".into()),
                Some("{\"baseUrl\":\"https://custom.example/v1\"}".into()),
                0,
                1,
                41,
                42,
                None,
            )
        );
        assert_eq!(
            conn.query_row(
                "SELECT id, agent_module_id, provider_id, enabled,
                        credential_version, created_at, updated_at, route_protocol
                 FROM agent_provider_bindings WHERE id = 'custom-binding'",
                [],
                |row| Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<String>>(7)?,
                )),
            )
            .unwrap(),
            (
                "custom-binding".into(),
                "codex".into(),
                "custom-provider".into(),
                0,
                0,
                43,
                44,
                Some("gemini".into()),
            )
        );
        assert_eq!(
            conn.query_row(
                "SELECT value FROM settings
                 WHERE key = 'system_provider_default_bindings_v1_seeded'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "true"
        );
        assert_eq!(
            conn.prepare(
                "SELECT source, provider_id
                 FROM usage_source_bindings ORDER BY source",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
            vec![
                ("claude".into(), "system-claude-subscription".into()),
                ("codex".into(), "custom-provider".into()),
            ]
        );
        assert_eq!(
            conn.query_row(
                "SELECT updated_at FROM usage_source_bindings WHERE source = 'codex'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            45
        );
    }

    #[test]
    fn system_catalog_reconciliation_is_idempotent_and_never_restores_a_deleted_default_binding() {
        let conn = v16_usage_fixture();
        Database::apply_schema_migrations_on_conn_with_roots(
            &conn,
            &super::migration_v15_to_v16::roots(),
        )
        .expect("migrate v16 to v17");
        conn.execute(
            "DELETE FROM agent_provider_bindings
             WHERE agent_module_id = 'opencode'
               AND provider_id = 'system-openrouter-api'",
            [],
        )
        .expect("user removes a default binding");
        conn.execute(
            "UPDATE usage_providers SET daily_budget_usd = '25.5'
             WHERE id = 'system-openai-api'",
            [],
        )
        .expect("user configures a metered system Provider budget");

        let assert_catalog_metadata = || {
            let chatgpt = conn
                .query_row(
                    "SELECT quota_source, quota_interval_seconds
                     FROM usage_providers WHERE id = 'system-chatgpt-subscription'",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<u64>>(1)?,
                        ))
                    },
                )
                .unwrap();
            assert_eq!(chatgpt.0.as_deref(), Some("codex_oauth"));
            assert_eq!(chatgpt.1, Some(300));

            let claude = conn
                .query_row(
                    "SELECT quota_source, quota_interval_seconds
                     FROM usage_providers WHERE id = 'system-claude-subscription'",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<u64>>(1)?,
                        ))
                    },
                )
                .unwrap();
            assert_eq!(claude.0.as_deref(), Some("claude_local"));
            assert_eq!(claude.1, Some(300));

            assert_eq!(
                conn.query_row(
                    "SELECT daily_budget_usd FROM usage_providers
                     WHERE id = 'system-openai-api'",
                    [],
                    |row| row.get::<_, Option<String>>(0),
                )
                .unwrap()
                .as_deref(),
                Some("25.5")
            );
        };

        crate::usage::system_provider_migration::reconcile_system_provider_catalog(&conn)
            .expect("reconcile canonical card metadata");
        assert_catalog_metadata();
        crate::usage::system_provider_migration::reconcile_system_provider_catalog(&conn)
            .expect("repeat catalog reconciliation");
        assert_catalog_metadata();

        assert_eq!(
            super::scalar_i64(
                &conn,
                "SELECT COUNT(*) FROM usage_providers
                 WHERE system_preset_key IS NOT NULL",
            ),
            20
        );
        assert_eq!(
            super::scalar_i64(
                &conn,
                "SELECT COUNT(*) FROM agent_provider_bindings
                 WHERE provider_id LIKE 'system-%'",
            ),
            4
        );
        assert_eq!(super::count(&conn, "provider_api_credentials"), 18);
        assert_eq!(
            super::scalar_i64(
                &conn,
                "SELECT COUNT(*) FROM agent_provider_bindings
                 WHERE agent_module_id = 'opencode'
                   AND provider_id = 'system-openrouter-api'",
            ),
            0
        );
    }

    #[test]
    fn current_schema_reconciles_catalog_growth_before_validation() {
        let conn = v16_usage_fixture();
        Database::apply_schema_migrations_on_conn_with_roots(
            &conn,
            &super::migration_v15_to_v16::roots(),
        )
        .expect("create current database");

        conn.execute_batch(
            "DELETE FROM provider_api_credentials
             WHERE provider_id NOT IN (
                 'system-openai-api','system-anthropic-api','system-openrouter-api'
             );
             DROP TRIGGER usage_providers_system_delete;
             DELETE FROM usage_providers
             WHERE system_preset_key NOT IN (
                 'chatgpt-subscription','claude-subscription','openai-api',
                 'anthropic-api','openrouter-api'
             );
             CREATE TRIGGER usage_providers_system_delete
             BEFORE DELETE ON usage_providers
             WHEN OLD.system_preset_key IS NOT NULL
             BEGIN
                 SELECT RAISE(ABORT, 'system provider cannot be deleted');
             END;",
        )
        .expect("simulate database from the original five-entry catalog");
        assert_eq!(
            super::scalar_i64(
                &conn,
                "SELECT COUNT(*) FROM usage_providers
                 WHERE system_preset_key IS NOT NULL",
            ),
            5
        );

        Database::apply_schema_migrations_on_conn_with_roots(
            &conn,
            &super::migration_v15_to_v16::roots(),
        )
        .expect("reconcile expanded catalog without a schema bump");

        assert_eq!(
            super::scalar_i64(
                &conn,
                "SELECT COUNT(*) FROM usage_providers
                 WHERE system_preset_key IS NOT NULL",
            ),
            20
        );
        assert_eq!(super::count(&conn, "provider_api_credentials"), 18);
        assert_eq!(
            conn.query_row(
                "SELECT enabled FROM usage_providers WHERE id = 'system-kimi-api'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn system_provider_identity_and_rows_are_sql_protected() {
        let conn = v16_usage_fixture();
        Database::apply_schema_migrations_on_conn_with_roots(
            &conn,
            &super::migration_v15_to_v16::roots(),
        )
        .expect("migrate v16 to v17");

        assert!(conn
            .execute(
                "UPDATE usage_providers
                 SET system_preset_key = 'renamed'
                 WHERE id = 'system-openai-api'",
                [],
            )
            .is_err());
        assert!(conn
            .execute(
                "UPDATE usage_providers
                 SET id = 'renamed-system-openai-api'
                 WHERE id = 'system-openai-api'",
                [],
            )
            .is_err());
        assert!(conn
            .execute(
                "DELETE FROM usage_providers WHERE id = 'system-openai-api'",
                [],
            )
            .is_err());
        assert!(conn
            .execute(
                "INSERT INTO usage_providers (
                     id, name, billing_kind, product_group_id, token_sources,
                     enabled, needs_review, created_at, updated_at, system_preset_key
                 ) VALUES (
                     'duplicate-system-key', 'Duplicate', 'metered', 'duplicate',
                     '[\"proxy\"]', 1, 0, 1, 1, 'openai-api'
                 )",
                [],
            )
            .is_err());
    }

    #[test]
    fn migration_v16_to_v17_rolls_back_every_object_and_row_on_catalog_collision() {
        let conn = v16_usage_fixture();
        conn.execute(
            "INSERT INTO usage_providers (
                 id, name, billing_kind, product_group_id, token_sources,
                 enabled, needs_review, created_at, updated_at
             ) VALUES (
                 'system-openai-api', 'User Collision', 'metered', 'collision',
                 '[\"proxy\"]', 1, 0, 1, 1
             )",
            [],
        )
        .expect("insert reserved-id collision in v16");

        let error = Database::apply_schema_migrations_on_conn_with_roots(
            &conn,
            &super::migration_v15_to_v16::roots(),
        )
        .expect_err("reserved system id collision must fail closed");
        assert!(error.to_string().contains("reserved system Provider id"));
        assert_eq!(Database::get_user_version(&conn).unwrap(), 16);
        assert!(!Database::has_column(&conn, "usage_providers", "system_preset_key").unwrap());
        assert!(!Database::has_column(&conn, "agent_provider_bindings", "route_protocol").unwrap());
        assert!(!Database::table_exists(&conn, "provider_api_credentials").unwrap());
        assert!(!Database::table_exists(&conn, "provider_credential_operations").unwrap());
        assert_eq!(
            super::scalar_i64(
                &conn,
                "SELECT COUNT(*) FROM usage_providers
                 WHERE id = 'system-openai-api' AND name = 'User Collision'",
            ),
            1
        );
        for object in [
            "idx_usage_providers_system_preset_key",
            "usage_providers_system_identity_immutable",
            "usage_providers_system_delete",
        ] {
            assert_eq!(
                super::scalar_i64(
                    &conn,
                    &format!("SELECT COUNT(*) FROM sqlite_schema WHERE name = '{object}'"),
                ),
                0
            );
        }
    }

    #[test]
    fn current_v17_validator_rejects_same_named_but_malformed_objects() {
        for corruption in [
            "DROP INDEX idx_usage_providers_system_preset_key;
             CREATE INDEX idx_usage_providers_system_preset_key
             ON usage_providers(system_preset_key);",
            "DROP TRIGGER usage_providers_system_delete;
             CREATE TRIGGER usage_providers_system_delete
             AFTER INSERT ON usage_providers
             BEGIN
                 SELECT 1;
             END;",
        ] {
            let conn = v16_usage_fixture();
            Database::apply_schema_migrations_on_conn_with_roots(
                &conn,
                &super::migration_v15_to_v16::roots(),
            )
            .expect("create complete v17 schema");
            conn.execute_batch(corruption)
                .expect("replace required object with malformed namesake");

            let error = Database::apply_schema_migrations_on_conn_with_roots(
                &conn,
                &super::migration_v15_to_v16::roots(),
            )
            .expect_err("v17 validator must reject a malformed namesake");
            assert!(error.to_string().contains("incomplete schema v17"));
        }
    }

    #[test]
    fn migration_continues_from_v12_through_current_schema() {
        let conn = super::true_v12_usage_fixture();
        Database::apply_schema_migrations_on_conn_with_roots(
            &conn,
            &super::migration_v15_to_v16::roots(),
        )
        .expect("migrate continuously from v12 through current schema");
        assert_eq!(Database::get_user_version(&conn).unwrap(), SCHEMA_VERSION);
        assert!(Database::has_column(&conn, "usage_providers", "system_preset_key").unwrap());
        assert!(Database::has_column(&conn, "usage_providers", "daily_budget_usd").unwrap());
        assert!(Database::has_column(&conn, "quota_fetch_state", "consecutive_failures").unwrap());
        assert!(Database::has_column(&conn, "agent_provider_bindings", "route_protocol").unwrap());
        assert!(Database::table_exists(&conn, "provider_api_credentials").unwrap());
        assert!(Database::table_exists(&conn, "provider_credential_operations").unwrap());
    }
}

mod migration_v17_to_v18 {
    use super::*;
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

    fn v17_usage_fixture() -> Connection {
        let conn = super::migration_v15_to_v16::v15_usage_fixture();
        crate::usage::agent_module_migration::migrate_v15_to_v16(&conn)
            .expect("create complete v16 fixture");
        Database::set_user_version(&conn, 16).expect("set v16 fixture version");
        crate::usage::system_provider_migration::migrate_v16_to_v17(&conn)
            .expect("create complete v17 fixture");
        Database::set_user_version(&conn, 17).expect("set v17 fixture version");
        crate::usage::system_provider_migration::validate_schema_v17_complete(&conn)
            .expect("validate complete v17 fixture");
        conn
    }

    #[test]
    fn migration_from_v17_adds_current_usage_columns_and_preserves_daily_budget() {
        let conn = v17_usage_fixture();
        conn.execute(
            "INSERT INTO usage_providers (
                 id, name, billing_kind, product_group_id, token_sources,
                 quota_source, quota_interval_seconds, route_app_type, route_config,
                 quota_config, enabled, needs_review, legacy_app_type,
                 legacy_provider_id, created_at, updated_at
             ) VALUES (
                 'custom-metered', 'Custom Metered', 'metered', 'custom-metered',
                 '[\"proxy\"]', NULL, NULL, NULL, NULL, NULL,
                 1, 0, NULL, NULL, 41, 42
             )",
            [],
        )
        .expect("insert custom v17 Provider");
        crate::usage::system_provider_migration::reconcile_system_provider_catalog(&conn)
            .expect("reconcile v17 system Provider catalog");

        Database::apply_schema_migrations_on_conn_with_roots(
            &conn,
            &super::migration_v15_to_v16::roots(),
        )
        .expect("migrate v17 to current schema");

        assert_eq!(
            Database::get_user_version(&conn).unwrap(),
            super::SCHEMA_VERSION
        );
        assert!(Database::has_column(&conn, "usage_providers", "daily_budget_usd").unwrap());
        assert!(Database::has_column(&conn, "quota_fetch_state", "consecutive_failures").unwrap());
        assert_eq!(
            conn.query_row(
                "SELECT upper(type), \"notnull\", dflt_value
                 FROM pragma_table_info('usage_providers')
                 WHERE name = 'daily_budget_usd'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .unwrap(),
            ("TEXT".to_string(), 0, None),
        );
        assert_eq!(
            conn.query_row(
                "SELECT daily_budget_usd FROM usage_providers WHERE id = 'custom-metered'",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap(),
            None,
        );

        conn.execute(
            "UPDATE usage_providers SET daily_budget_usd = '25.5'
             WHERE id = 'system-openai-api'",
            [],
        )
        .unwrap();
        crate::usage::system_provider_migration::reconcile_system_provider_catalog(&conn)
            .expect("reconcile v18 system Provider catalog");
        assert_eq!(
            conn.query_row(
                "SELECT daily_budget_usd FROM usage_providers
                 WHERE id = 'system-openai-api'",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap(),
            Some("25.5".to_string()),
        );
    }

    #[test]
    fn migration_v17_to_v18_rolls_back_column_and_version_on_failure() {
        let conn = v17_usage_fixture();
        conn.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Pragma {
                pragma_name: "user_version",
                pragma_value: Some("18"),
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }))
        .expect("install forced v18 migration failure");

        let error = crate::usage::budget_migration::migrate_v17_to_v18(&conn)
            .expect_err("forced v18 migration failure must roll back");

        conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .expect("clear forced v18 migration failure");
        assert!(error.to_string().contains("not authorized"), "{error}");
        assert_eq!(Database::get_user_version(&conn).unwrap(), 17);
        assert!(!Database::has_column(&conn, "usage_providers", "daily_budget_usd").unwrap());
    }

    #[test]
    fn migration_chain_rolls_back_to_v0_when_v18_step_fails() {
        let conn = Connection::open_in_memory().expect("open v0 fixture");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("enable foreign keys");
        Database::create_tables_on_conn(&conn).expect("create application tables");
        assert_eq!(Database::get_user_version(&conn).unwrap(), 0);
        conn.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Pragma {
                pragma_name: "user_version",
                pragma_value: Some("18"),
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }))
        .expect("install forced v18 chain failure");

        let error = Database::apply_schema_migrations_on_conn_with_roots(
            &conn,
            &super::migration_v15_to_v16::roots(),
        )
        .expect_err("forced v18 chain failure must roll back every migration");

        conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .expect("clear forced v18 chain failure");
        assert!(error.to_string().contains("not authorized"), "{error}");
        assert_eq!(Database::get_user_version(&conn).unwrap(), 0);
        assert!(!Database::has_column(&conn, "usage_providers", "daily_budget_usd").unwrap());
    }

    #[test]
    fn current_v18_validator_rejects_database_missing_daily_budget_column() {
        let conn = v17_usage_fixture();
        Database::set_user_version(&conn, 18).expect("claim incomplete v18 schema");

        let error = Database::apply_schema_migrations_on_conn_with_roots(
            &conn,
            &super::migration_v15_to_v16::roots(),
        )
        .expect_err("v18 completeness validation must fail closed");

        assert!(error.to_string().contains("incomplete schema v18"));
        assert!(!Database::has_column(&conn, "usage_providers", "daily_budget_usd").unwrap());
    }
}

#[test]
fn identity_discriminator_v13_database_preserves_unenumerated_text_and_json() -> Result<(), AppError>
{
    let conn = Connection::open_in_memory().expect("create fixed v13 database");
    conn.execute_batch(
        r#"
        CREATE TABLE providers (
            id TEXT NOT NULL,
            app_type TEXT NOT NULL,
            name TEXT NOT NULL,
            settings_config TEXT NOT NULL,
            meta TEXT NOT NULL DEFAULT '{}',
            PRIMARY KEY (id, app_type)
        );
        CREATE TABLE mcp_servers (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            server_config TEXT NOT NULL
        );
        CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT);
        CREATE TABLE profiles (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            payload TEXT NOT NULL
        );
        PRAGMA user_version = 13;
        SAVEPOINT identity_discriminator_v14_test;
        INSERT INTO providers (id, app_type, name, settings_config, meta)
        VALUES ('external-cc-switch-provider', 'claude', 'External',
                '{"remoteRoot":"cc-switch-sync"}',
                '{"affiliate":"cc_switch"}');
        INSERT INTO mcp_servers (id, name, server_config)
        VALUES ('external-wire', 'External MCP', '{"protocol":"cc-switch-webdav-sync"}');
        INSERT INTO profiles (id, name, payload)
        VALUES ('legacy-profile', 'Imported', '{"providerId":"ccswitch"}');
        INSERT INTO settings (key, value)
        VALUES ('external_remote_root', 'cc-switch-sync');
        "#,
    )
    .expect("seed identity boundary values");

    let before: Vec<String> = [
        "SELECT settings_config || '|' || meta FROM providers WHERE id='external-cc-switch-provider'",
        "SELECT server_config FROM mcp_servers WHERE id='external-wire'",
        "SELECT payload FROM profiles WHERE id='legacy-profile'",
        "SELECT value FROM settings WHERE key='external_remote_root'",
    ]
    .into_iter()
    .map(|sql| conn.query_row(sql, [], |row| row.get(0)).unwrap())
    .collect();

    super::schema::migrate_app_owned_identity_v14(&conn)
        .expect("validate explicit no-op identity boundary");

    let after: Vec<String> = [
        "SELECT settings_config || '|' || meta FROM providers WHERE id='external-cc-switch-provider'",
        "SELECT server_config FROM mcp_servers WHERE id='external-wire'",
        "SELECT payload FROM profiles WHERE id='legacy-profile'",
        "SELECT value FROM settings WHERE key='external_remote_root'",
    ]
    .into_iter()
    .map(|sql| conn.query_row(sql, [], |row| row.get(0)).unwrap())
    .collect();

    assert_eq!(after, before);
    conn.execute_batch(
        "ROLLBACK TO identity_discriminator_v14_test; RELEASE identity_discriminator_v14_test;",
    )
    .expect("rollback identity boundary fixture");
    Ok(())
}

#[test]
fn identity_discriminator_v14_boundary_rejects_missing_expected_schema() {
    let conn = Connection::open_in_memory().expect("open empty sqlite database");
    let error = super::schema::migrate_app_owned_identity_v14(&conn)
        .expect_err("missing v13 tables must fail closed");
    assert!(error.to_string().contains("providers"));
}

#[test]
fn identity_discriminator_v14_boundary_rejects_non_v13_schema() {
    let conn = Connection::open_in_memory().expect("open wrong-version sqlite database");
    conn.execute_batch(
        r#"
        CREATE TABLE providers (id TEXT);
        CREATE TABLE mcp_servers (id TEXT);
        CREATE TABLE settings (key TEXT);
        CREATE TABLE profiles (id TEXT);
        PRAGMA user_version = 12;
        "#,
    )
    .expect("create wrong-version identity boundary");
    let error = super::schema::migrate_app_owned_identity_v14(&conn)
        .expect_err("non-v13 boundary must fail closed");
    assert!(error.to_string().contains("expected v13, found v12"));
}

const LEGACY_SCHEMA_SQL: &str = r#"
    CREATE TABLE providers (
        id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        name TEXT NOT NULL,
        settings_config TEXT NOT NULL,
        PRIMARY KEY (id, app_type)
    );
    CREATE TABLE provider_endpoints (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        provider_id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        url TEXT NOT NULL
    );
    CREATE TABLE mcp_servers (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        server_config TEXT NOT NULL
    );
    CREATE TABLE prompts (
        id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        name TEXT NOT NULL,
        content TEXT NOT NULL,
        PRIMARY KEY (id, app_type)
    );
    CREATE TABLE skills (
        key TEXT PRIMARY KEY,
        installed BOOLEAN NOT NULL DEFAULT 0
    );
    CREATE TABLE skill_repos (
        owner TEXT NOT NULL,
        name TEXT NOT NULL,
        PRIMARY KEY (owner, name)
    );
    CREATE TABLE settings (
        key TEXT PRIMARY KEY,
        value TEXT
    );
"#;

// v3.8.x（schema v1）的真实表结构快照：用于验证从 v3.8.* 升级到当前版本的迁移链路
// 参考：tag v3.8.3 的 src-tauri/src/database/schema.rs
const V3_8_SCHEMA_V1_SQL: &str = r#"
    CREATE TABLE providers (
        id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        name TEXT NOT NULL,
        settings_config TEXT NOT NULL,
        website_url TEXT,
        category TEXT,
        created_at INTEGER,
        sort_index INTEGER,
        notes TEXT,
        icon TEXT,
        icon_color TEXT,
        meta TEXT NOT NULL DEFAULT '{}',
        is_current BOOLEAN NOT NULL DEFAULT 0,
        PRIMARY KEY (id, app_type)
    );
    CREATE TABLE provider_endpoints (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        provider_id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        url TEXT NOT NULL,
        added_at INTEGER,
        FOREIGN KEY (provider_id, app_type) REFERENCES providers(id, app_type) ON DELETE CASCADE
    );
    CREATE TABLE mcp_servers (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        server_config TEXT NOT NULL,
        description TEXT,
        homepage TEXT,
        docs TEXT,
        tags TEXT NOT NULL DEFAULT '[]',
        enabled_claude BOOLEAN NOT NULL DEFAULT 0,
        enabled_codex BOOLEAN NOT NULL DEFAULT 0,
        enabled_gemini BOOLEAN NOT NULL DEFAULT 0
    );
    CREATE TABLE prompts (
        id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        name TEXT NOT NULL,
        content TEXT NOT NULL,
        description TEXT,
        enabled BOOLEAN NOT NULL DEFAULT 1,
        created_at INTEGER,
        updated_at INTEGER,
        PRIMARY KEY (id, app_type)
    );
    CREATE TABLE skills (
        key TEXT PRIMARY KEY,
        installed BOOLEAN NOT NULL DEFAULT 0,
        installed_at INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE skill_repos (
        owner TEXT NOT NULL,
        name TEXT NOT NULL,
        branch TEXT NOT NULL DEFAULT 'main',
        enabled BOOLEAN NOT NULL DEFAULT 1,
        PRIMARY KEY (owner, name)
    );
    CREATE TABLE settings (
        key TEXT PRIMARY KEY,
        value TEXT
    );
"#;

#[derive(Debug)]
struct ColumnInfo {
    r#type: String,
    notnull: i64,
    default: Option<String>,
}

fn get_column_info(conn: &Connection, table: &str, column: &str) -> ColumnInfo {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info(\"{table}\");"))
        .expect("prepare pragma");
    let mut rows = stmt.query([]).expect("query pragma");
    while let Some(row) = rows.next().expect("read row") {
        let column_name: String = row.get(1).expect("name");
        if column_name.eq_ignore_ascii_case(column) {
            return ColumnInfo {
                r#type: row.get::<_, String>(2).expect("type"),
                notnull: row.get::<_, i64>(3).expect("notnull"),
                default: row.get::<_, Option<String>>(4).ok().flatten(),
            };
        }
    }
    panic!("column {table}.{column} not found");
}

fn normalize_default(default: &Option<String>) -> Option<String> {
    default
        .as_ref()
        .map(|s| s.trim_matches('\'').trim_matches('"').to_string())
}

#[test]
fn schema_migration_sets_user_version_when_missing() {
    let conn = Connection::open_in_memory().expect("open memory db");

    Database::create_tables_on_conn(&conn).expect("create tables");
    assert_eq!(
        Database::get_user_version(&conn).expect("read version before"),
        0
    );

    Database::apply_schema_migrations_on_conn(&conn).expect("apply migration");

    assert_eq!(
        Database::get_user_version(&conn).expect("read version after"),
        SCHEMA_VERSION
    );
}

#[test]
fn schema_migration_v20_to_v21_drops_retired_tables_and_preserves_live_data() {
    let db = Database::memory().expect("create current in-memory database");
    let conn = db.conn.lock().expect("lock in-memory database");

    let retired_tables = [
        "mcp_servers",
        "prompts",
        "profiles",
        "provider_health",
        "skills",
        "skill_repos",
        "proxy_config",
        "proxy_live_backup",
        "stream_check_logs",
    ];
    for table in retired_tables {
        assert!(
            !Database::table_exists(&conn, table).unwrap(),
            "fresh current schema should not create retired table {table}"
        );
        conn.execute(&format!("CREATE TABLE {table} (id TEXT)"), [])
            .unwrap_or_else(|error| panic!("create retired table {table}: {error}"));
        conn.execute(&format!("INSERT INTO {table} (id) VALUES ('retired')"), [])
            .unwrap_or_else(|error| panic!("seed retired table {table}: {error}"));
    }

    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES ('v21_survivor', 'kept')",
        [],
    )
    .expect("seed surviving settings row");
    conn.execute(
        "INSERT INTO proxy_request_logs (
            request_id, provider_id, app_type, model, latency_ms, status_code, created_at
         ) VALUES ('v21-survivor-log', 'v21-survivor-provider', 'codex', 'gpt-test', 1, 200, 1)",
        [],
    )
    .expect("seed surviving usage log");
    Database::set_user_version(&conn, 20).expect("mark fixture as schema v20");

    Database::apply_schema_migrations_on_conn(&conn).expect("migrate v20 through current schema");

    assert_eq!(Database::get_user_version(&conn).unwrap(), SCHEMA_VERSION);
    for table in retired_tables {
        assert!(
            !Database::table_exists(&conn, table).unwrap(),
            "retired table {table} should be dropped"
        );
    }
    assert!(Database::table_exists(&conn, "proxy_request_logs").unwrap());
    assert!(Database::table_exists(&conn, "provider_endpoints").unwrap());
    assert_eq!(
        conn.query_row(
            "SELECT value FROM settings WHERE key = 'v21_survivor'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "kept"
    );
    assert_eq!(
        conn.query_row(
            "SELECT model FROM proxy_request_logs WHERE request_id = 'v21-survivor-log'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "gpt-test"
    );
}

#[test]
fn schema_migration_v21_to_v22_backfills_eligible_costs_and_restores_immutability() {
    use crate::usage::metering::calculator::{CostCalculator, ModelPricing};
    use crate::usage::metering::parser::TokenUsage;
    use crate::usage::system_providers::CHATGPT_SUBSCRIPTION_ID;
    use rust_decimal::Decimal;

    let db = Database::memory().expect("create current in-memory database");
    let conn = db.conn.lock().expect("lock in-memory database");

    let pricing =
        ModelPricing::from_strings("3.25", "7.5", "0.55", "4.75").expect("create expected pricing");
    conn.execute(
        "INSERT INTO model_pricing (
            model_id, display_name, input_cost_per_million, output_cost_per_million,
            cache_read_cost_per_million, cache_creation_cost_per_million
         ) VALUES ('v22-priced-model', 'v22-priced-model', ?1, ?2, ?3, ?4)",
        params!["3.25", "7.5", "0.55", "4.75"],
    )
    .expect("seed v22 model pricing");
    conn.execute(
        "INSERT INTO usage_events (
            event_id, source, provider_id, product_group_id, occurred_at, model,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            input_cost_usd, output_cost_usd, cache_read_cost_usd,
            cache_creation_cost_usd, total_cost_usd, cost_source, created_at,
            agent_module_id, pricing_origin
         ) VALUES
            ('v22-priced', 'session_log', ?1, 'codex', 1, 'v22-priced-model',
             1500, 200, 400, 50, NULL, NULL, NULL, NULL, NULL,
             'unavailable', 1, 'codex', NULL),
            ('v22-unpriced', 'session_log', ?1, 'codex', 2, 'v22-unknown-model',
             100, 20, 0, 0, NULL, NULL, NULL, NULL, NULL,
             'unavailable', 2, 'codex', NULL),
            ('v22-existing-cost', 'session_log', ?1, 'codex', 3, 'v22-priced-model',
             100, 20, 0, 0, '0.11', '0.22', '0.33', '0.44', '9.99',
             'upstream', 3, 'codex', NULL),
            ('v22-zero-token', 'session_log', ?1, 'codex', 4, 'v22-priced-model',
             0, 0, 0, 0, NULL, NULL, NULL, NULL, NULL,
             'unavailable', 4, 'codex', NULL)",
        [CHATGPT_SUBSCRIPTION_ID],
    )
    .expect("seed v21 usage events");
    Database::set_user_version(&conn, 21).expect("mark fixture as schema v21");

    Database::apply_schema_migrations_on_conn(&conn).expect("migrate v21 to v22");

    assert_eq!(Database::get_user_version(&conn).unwrap(), SCHEMA_VERSION);

    let usage = TokenUsage {
        input_tokens: 1500,
        output_tokens: 200,
        cache_read_tokens: 400,
        cache_creation_tokens: 50,
        ..TokenUsage::default()
    };
    let expected = CostCalculator::calculate_for_app("codex", &usage, &pricing, Decimal::ONE);
    #[derive(Debug, PartialEq)]
    struct StoredCost {
        input: Option<String>,
        output: Option<String>,
        cache_read: Option<String>,
        cache_creation: Option<String>,
        total: Option<String>,
        source: String,
        origin: Option<String>,
    }
    let priced = conn
        .query_row(
            "SELECT input_cost_usd, output_cost_usd, cache_read_cost_usd,
                    cache_creation_cost_usd, total_cost_usd, cost_source, pricing_origin
             FROM usage_events WHERE event_id = 'v22-priced'",
            [],
            |row| {
                Ok(StoredCost {
                    input: row.get(0)?,
                    output: row.get(1)?,
                    cache_read: row.get(2)?,
                    cache_creation: row.get(3)?,
                    total: row.get(4)?,
                    source: row.get(5)?,
                    origin: row.get(6)?,
                })
            },
        )
        .expect("read backfilled event");
    assert_eq!(
        priced,
        StoredCost {
            input: Some(expected.input_cost.to_string()),
            output: Some(expected.output_cost.to_string()),
            cache_read: Some(expected.cache_read_cost.to_string()),
            cache_creation: Some(expected.cache_creation_cost.to_string()),
            total: Some(expected.total_cost.to_string()),
            source: "estimated".to_string(),
            origin: Some("official".to_string()),
        }
    );

    let unpriced: (Option<String>, String, Option<String>) = conn
        .query_row(
            "SELECT total_cost_usd, cost_source, pricing_origin
             FROM usage_events WHERE event_id = 'v22-unpriced'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read still-unpriced event");
    assert_eq!(unpriced, (None, "unavailable".to_string(), None));

    let existing_cost: (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    ) = conn
        .query_row(
            "SELECT input_cost_usd, output_cost_usd, cache_read_cost_usd,
                    cache_creation_cost_usd, total_cost_usd
             FROM usage_events WHERE event_id = 'v22-existing-cost'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("read pre-costed event");
    assert_eq!(
        existing_cost,
        (
            Some("0.11".to_string()),
            Some("0.22".to_string()),
            Some("0.33".to_string()),
            Some("0.44".to_string()),
            "9.99".to_string(),
        )
    );

    let zero_token: (Option<String>, String) = conn
        .query_row(
            "SELECT total_cost_usd, cost_source
             FROM usage_events WHERE event_id = 'v22-zero-token'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read zero-token event");
    assert_eq!(zero_token, (None, "unavailable".to_string()));

    let update_error = conn
        .execute(
            "UPDATE usage_events SET model = model WHERE event_id = 'v22-priced'",
            [],
        )
        .expect_err("usage_events update trigger must be restored");
    assert!(
        update_error
            .to_string()
            .contains("usage_events are immutable"),
        "unexpected immutable update error: {update_error}"
    );
}

#[test]
fn schema_migration_v22_to_v23_preserves_rates_and_makes_them_nullable() {
    let db = Database::memory().expect("create current in-memory database");
    let conn = db.conn.lock().expect("lock in-memory database");
    conn.execute_batch(
        "DROP TABLE provider_model_pricing;
         CREATE TABLE provider_model_pricing (
            provider_id TEXT NOT NULL,
            model_id TEXT NOT NULL,
            display_name TEXT NOT NULL DEFAULT '',
            input_cost_per_million TEXT NOT NULL,
            output_cost_per_million TEXT NOT NULL,
            cache_read_cost_per_million TEXT NOT NULL DEFAULT '0',
            cache_creation_cost_per_million TEXT NOT NULL DEFAULT '0',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (provider_id, model_id),
            FOREIGN KEY (provider_id) REFERENCES usage_providers(id) ON DELETE CASCADE
         );
         INSERT INTO provider_model_pricing (
            provider_id, model_id, display_name,
            input_cost_per_million, output_cost_per_million,
            cache_read_cost_per_million, cache_creation_cost_per_million,
            created_at, updated_at
         ) VALUES (
            'system-openai-api', 'preserved-model', 'Preserved Model',
            '1.25', '10', '0.125', '2.5', 123, 456
         );
         PRAGMA user_version = 22;",
    )
    .expect("build v22 provider pricing fixture");

    Database::apply_schema_migrations_on_conn(&conn).expect("migrate v22 to v23");

    assert_eq!(Database::get_user_version(&conn).unwrap(), SCHEMA_VERSION);
    let preserved: (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        i64,
        i64,
    ) = conn
        .query_row(
            "SELECT provider_id, model_id, display_name,
                    input_cost_per_million, output_cost_per_million,
                    cache_read_cost_per_million, cache_creation_cost_per_million,
                    created_at, updated_at
             FROM provider_model_pricing WHERE model_id = 'preserved-model'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )
        .expect("read migrated custom price");
    assert_eq!(
        preserved,
        (
            "system-openai-api".to_string(),
            "preserved-model".to_string(),
            "Preserved Model".to_string(),
            "1.25".to_string(),
            "10".to_string(),
            "0.125".to_string(),
            "2.5".to_string(),
            123,
            456,
        )
    );
    for column in [
        "input_cost_per_million",
        "output_cost_per_million",
        "cache_read_cost_per_million",
        "cache_creation_cost_per_million",
    ] {
        let info = get_column_info(&conn, "provider_model_pricing", column);
        assert_eq!(info.notnull, 0, "{column} should be nullable");
        assert_eq!(info.default, None, "{column} should not default to zero");
    }
}

#[test]
fn schema_migration_rejects_future_version() {
    let conn = Connection::open_in_memory().expect("open memory db");
    Database::create_tables_on_conn(&conn).expect("create tables");
    Database::set_user_version(&conn, SCHEMA_VERSION + 1).expect("set future version");

    let err =
        Database::apply_schema_migrations_on_conn(&conn).expect_err("should reject higher version");
    assert!(
        err.to_string().contains("数据库版本过新"),
        "unexpected error: {err}"
    );
}

#[test]
fn schema_migration_adds_missing_columns_for_providers() {
    let conn = Connection::open_in_memory().expect("open memory db");

    // 创建旧版 providers 表，缺少新增列
    conn.execute_batch(LEGACY_SCHEMA_SQL)
        .expect("seed old schema");

    Database::apply_schema_migrations_on_conn(&conn).expect("apply migrations");

    // 验证关键新增列已补齐
    for (table, column) in [
        ("providers", "meta"),
        ("providers", "is_current"),
        ("provider_endpoints", "added_at"),
    ] {
        assert!(
            Database::has_column(&conn, table, column).expect("check column"),
            "{table}.{column} should exist after migration"
        );
    }

    // 验证 meta 列约束保持一致
    let meta = get_column_info(&conn, "providers", "meta");
    assert_eq!(meta.notnull, 1, "meta should be NOT NULL");
    assert_eq!(
        normalize_default(&meta.default).as_deref(),
        Some("{}"),
        "meta default should be '{{}}'"
    );

    assert_eq!(
        Database::get_user_version(&conn).expect("version after migration"),
        SCHEMA_VERSION
    );
}

#[test]
fn schema_migration_aligns_column_defaults_and_types() {
    let conn = Connection::open_in_memory().expect("open memory db");
    conn.execute_batch(LEGACY_SCHEMA_SQL)
        .expect("seed old schema");

    Database::apply_schema_migrations_on_conn(&conn).expect("apply migrations");

    let is_current = get_column_info(&conn, "providers", "is_current");
    assert_eq!(is_current.r#type, "BOOLEAN");
    assert_eq!(is_current.notnull, 1);
    assert_eq!(normalize_default(&is_current.default).as_deref(), Some("0"));
}

#[test]
fn schema_create_tables_include_pricing_model_columns() {
    let conn = Connection::open_in_memory().expect("open memory db");
    Database::create_tables_on_conn(&conn).expect("create tables");

    let request_model = get_column_info(&conn, "proxy_request_logs", "request_model");
    assert_eq!(request_model.r#type, "TEXT");
    assert_eq!(request_model.notnull, 0);
}

#[test]
fn schema_migration_v4_adds_pricing_model_columns() {
    let conn = Connection::open_in_memory().expect("open memory db");
    conn.execute_batch(
        r#"
        CREATE TABLE providers (
            id TEXT NOT NULL,
            app_type TEXT NOT NULL,
            name TEXT NOT NULL,
            settings_config TEXT NOT NULL DEFAULT '{}',
            meta TEXT NOT NULL DEFAULT '{}',
            PRIMARY KEY (id, app_type)
        );
        CREATE TABLE proxy_config (
            app_type TEXT PRIMARY KEY,
            proxy_enabled INTEGER NOT NULL DEFAULT 0,
            listen_address TEXT NOT NULL DEFAULT '127.0.0.1',
            listen_port INTEGER NOT NULL DEFAULT 15722,
            enable_logging INTEGER NOT NULL DEFAULT 1,
            enabled INTEGER NOT NULL DEFAULT 0,
            auto_failover_enabled INTEGER NOT NULL DEFAULT 0,
            max_retries INTEGER NOT NULL DEFAULT 3,
            streaming_first_byte_timeout INTEGER NOT NULL DEFAULT 60,
            streaming_idle_timeout INTEGER NOT NULL DEFAULT 120,
            non_streaming_timeout INTEGER NOT NULL DEFAULT 600,
            circuit_failure_threshold INTEGER NOT NULL DEFAULT 4,
            circuit_success_threshold INTEGER NOT NULL DEFAULT 2,
            circuit_timeout_seconds INTEGER NOT NULL DEFAULT 60,
            circuit_error_rate_threshold REAL NOT NULL DEFAULT 0.6,
            circuit_min_requests INTEGER NOT NULL DEFAULT 10,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE proxy_request_logs (request_id TEXT PRIMARY KEY, model TEXT NOT NULL);
        CREATE TABLE mcp_servers (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            server_config TEXT NOT NULL,
            enabled_claude INTEGER NOT NULL DEFAULT 0,
            enabled_codex INTEGER NOT NULL DEFAULT 0,
            enabled_gemini INTEGER NOT NULL DEFAULT 0,
            enabled_opencode INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .expect("seed v4 schema");
    Database::set_user_version(&conn, 4).expect("set user_version=4");
    Database::migrate_v4_to_v5(&conn).expect("apply v4 to v5 migration");
    Database::set_user_version(&conn, 5).expect("set user_version=5");

    let multiplier = get_column_info(&conn, "proxy_config", "default_cost_multiplier");
    assert_eq!(multiplier.r#type, "TEXT");
    assert_eq!(multiplier.notnull, 1);
    assert_eq!(normalize_default(&multiplier.default).as_deref(), Some("1"));

    let pricing_source = get_column_info(&conn, "proxy_config", "pricing_model_source");
    assert_eq!(pricing_source.r#type, "TEXT");
    assert_eq!(pricing_source.notnull, 1);
    assert_eq!(
        normalize_default(&pricing_source.default).as_deref(),
        Some("response")
    );

    let request_model = get_column_info(&conn, "proxy_request_logs", "request_model");
    assert_eq!(request_model.r#type, "TEXT");
    assert_eq!(request_model.notnull, 0);

    assert_eq!(
        Database::get_user_version(&conn).expect("version after migration"),
        5
    );
}

#[test]
fn migration_v10_to_v11_rebuilds_rollups_with_request_model_dimension() {
    let conn = Connection::open_in_memory().expect("open memory db");

    // 模拟 v10 形状的 rollup 表（主键不含 request_model）+ 一行历史聚合数据，
    // 以及 v10 形状的明细表（无 pricing_model 列）
    conn.execute_batch(
        r#"
        CREATE TABLE proxy_request_logs (
            request_id TEXT PRIMARY KEY,
            model TEXT NOT NULL,
            request_model TEXT
        );
        CREATE TABLE usage_daily_rollups (
            date TEXT NOT NULL,
            app_type TEXT NOT NULL,
            provider_id TEXT NOT NULL,
            model TEXT NOT NULL,
            request_count INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            total_cost_usd TEXT NOT NULL DEFAULT '0',
            avg_latency_ms INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (date, app_type, provider_id, model)
        );
        INSERT INTO usage_daily_rollups
            (date, app_type, provider_id, model, request_count, success_count,
             input_tokens, output_tokens, total_cost_usd, avg_latency_ms)
        VALUES ('2026-05-01', 'claude', 'p1', 'kimi-k2', 7, 7, 1000, 500, '0.07', 120);
        "#,
    )
    .expect("seed v10 rollup table");
    Database::set_user_version(&conn, 10).expect("set user_version=10");
    Database::migrate_v10_to_v11(&conn).expect("apply v10 to v11 migration");
    Database::set_user_version(&conn, 11).expect("set user_version=11");

    // 新列存在且 NOT NULL DEFAULT ''
    let request_model = get_column_info(&conn, "usage_daily_rollups", "request_model");
    assert_eq!(request_model.r#type, "TEXT");
    assert_eq!(request_model.notnull, 1);
    let rollup_pricing_model = get_column_info(&conn, "usage_daily_rollups", "pricing_model");
    assert_eq!(rollup_pricing_model.r#type, "TEXT");
    assert_eq!(rollup_pricing_model.notnull, 1);

    // 明细表补上 pricing_model 列（可空，历史行 NULL）
    let pricing_model = get_column_info(&conn, "proxy_request_logs", "pricing_model");
    assert_eq!(pricing_model.r#type, "TEXT");
    assert_eq!(pricing_model.notnull, 0);

    // 历史行保留，request_model 填 ''（未知）
    let (rm, count, input, cost): (String, i64, i64, String) = conn
        .query_row(
            "SELECT request_model, request_count, input_tokens, total_cost_usd
             FROM usage_daily_rollups WHERE model = 'kimi-k2'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("migrated row");
    assert_eq!(rm, "");
    assert_eq!(count, 7);
    assert_eq!(input, 1000);
    assert_eq!(cost, "0.07");

    // 主键包含 request_model：同 model 不同别名可共存
    conn.execute(
        "INSERT INTO usage_daily_rollups
            (date, app_type, provider_id, model, request_model, request_count)
         VALUES ('2026-05-01', 'claude', 'p1', 'kimi-k2', 'claude-sonnet-4-6', 1)",
        [],
    )
    .expect("insert row with same model but different request_model");

    assert_eq!(
        Database::get_user_version(&conn).expect("version after migration"),
        11
    );
}

#[test]
fn migration_from_v3_8_schema_v1_to_current_schema_v3() {
    let conn = Connection::open_in_memory().expect("open memory db");
    conn.execute("PRAGMA foreign_keys = ON;", [])
        .expect("enable foreign keys");

    // 模拟 v3.8.* 用户的数据库（schema v1）
    conn.execute_batch(V3_8_SCHEMA_V1_SQL)
        .expect("seed v3.8 schema v1");
    Database::set_user_version(&conn, 1).expect("set user_version=1");

    // 插入一条旧版 Provider + Skill（用于验证迁移不会破坏既有数据）
    conn.execute(
        "INSERT INTO providers (
            id, app_type, name, settings_config, website_url, category,
            created_at, sort_index, notes, icon, icon_color, meta, is_current
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            "p1",
            "claude",
            "Test Provider",
            serde_json::to_string(&json!({ "anthropicApiKey": "sk-test" })).unwrap(),
            Option::<String>::None,
            Option::<String>::None,
            Option::<i64>::None,
            Option::<usize>::None,
            Option::<String>::None,
            Option::<String>::None,
            Option::<String>::None,
            "{}",
            1,
        ],
    )
    .expect("seed provider");

    // 按应用启动流程：先 create_tables（补齐新增表），再 apply_schema_migrations（按 user_version 迁移）
    Database::create_tables_on_conn(&conn).expect("create tables");
    Database::apply_schema_migrations_on_conn(&conn).expect("apply migrations");

    assert_eq!(
        Database::get_user_version(&conn).expect("user_version after migration"),
        SCHEMA_VERSION
    );

    // v1 -> v2：providers 新增字段必须补齐
    for column in [
        "cost_multiplier",
        "limit_daily_usd",
        "limit_monthly_usd",
        "provider_type",
        "in_failover_queue",
    ] {
        assert!(
            Database::has_column(&conn, "providers", column).expect("check column"),
            "providers.{column} should exist after migration"
        );
    }

    // 旧 provider 不应丢失，且新增字段应有默认值
    let provider_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM providers WHERE id = 'p1' AND app_type = 'claude'",
            [],
            |r| r.get(0),
        )
        .expect("count providers");
    assert_eq!(provider_count, 1);

    let cost_multiplier: String = conn
        .query_row(
            "SELECT cost_multiplier FROM providers WHERE id = 'p1' AND app_type = 'claude'",
            [],
            |r| r.get(0),
        )
        .expect("read cost_multiplier");
    assert_eq!(cost_multiplier, "1.0");

    // model_pricing 应具备默认数据（迁移时会 seed）
    let pricing_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM model_pricing", [], |r| r.get(0))
        .expect("count model_pricing rows");
    assert!(pricing_rows > 0, "model_pricing should be seeded");
}

#[test]
fn schema_dry_run_does_not_write_to_disk() {
    // Create minimal valid config for migration
    let mut apps = HashMap::new();
    apps.insert("claude".to_string(), ProviderManager::default());

    let config = MultiAppConfig {
        version: 2,
        apps,
        mcp: Default::default(),
        prompts: Default::default(),
        common_config_snippets: Default::default(),
        claude_common_config_snippet: None,
    };

    // Dry-run should succeed without any file I/O errors
    let result = Database::migrate_from_json_dry_run(&config);
    assert!(
        result.is_ok(),
        "Dry-run should succeed with valid config: {result:?}"
    );
}

#[test]
fn dry_run_validates_schema_compatibility() {
    // Create config with actual provider data
    let mut providers = IndexMap::new();
    providers.insert(
        "test-provider".to_string(),
        Provider {
            id: "test-provider".to_string(),
            name: "Test Provider".to_string(),
            settings_config: json!({
                "anthropicApiKey": "sk-test-123",
            }),
            website_url: None,
            category: None,
            created_at: Some(1234567890),
            sort_index: None,
            notes: None,
            meta: None,
            icon: None,
            icon_color: None,
            in_failover_queue: false,
        },
    );

    let manager = ProviderManager {
        providers,
        current: "test-provider".to_string(),
    };

    let mut apps = HashMap::new();
    apps.insert("claude".to_string(), manager);

    let config = MultiAppConfig {
        version: 2,
        apps,
        mcp: Default::default(),
        prompts: Default::default(),
        common_config_snippets: Default::default(),
        claude_common_config_snippet: None,
    };

    // Dry-run should validate the full migration path
    let result = Database::migrate_from_json_dry_run(&config);
    assert!(
        result.is_ok(),
        "Dry-run should succeed with provider data: {result:?}"
    );
}

#[test]
fn schema_model_pricing_is_seeded_on_init() {
    let db = Database::memory().expect("create memory db");

    let conn = db.conn.lock().expect("lock conn");

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM model_pricing", [], |row| row.get(0))
        .expect("count pricing");

    assert!(
        count > 0,
        "模型定价数据应该在初始化时自动填充，实际数量: {}",
        count
    );

    // 验证包含 Claude 模型
    let claude_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM model_pricing WHERE model_id LIKE 'claude-%'",
            [],
            |row| row.get(0),
        )
        .expect("check claude");
    assert!(
        claude_count > 0,
        "应该包含 Claude 模型定价，实际数量: {}",
        claude_count
    );

    // 验证包含 GPT 模型
    let gpt_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM model_pricing WHERE model_id LIKE 'gpt-%'",
            [],
            |row| row.get(0),
        )
        .expect("check gpt");
    assert!(
        gpt_count > 0,
        "应该包含 GPT 模型定价，实际数量: {}",
        gpt_count
    );

    // 验证包含 Gemini 模型
    let gemini_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM model_pricing WHERE model_id LIKE 'gemini-%'",
            [],
            |row| row.get(0),
        )
        .expect("check gemini");
    assert!(
        gemini_count > 0,
        "应该包含 Gemini 模型定价，实际数量: {}",
        gemini_count
    );
}

#[test]
fn model_pricing_seed_repairs_known_outdated_builtin_prices() {
    let db = Database::memory().expect("create memory db");

    {
        let conn = db.conn.lock().expect("lock conn");
        conn.execute(
            "UPDATE model_pricing
             SET input_cost_per_million = '1.68',
                 output_cost_per_million = '3.36',
                 cache_read_cost_per_million = '0.14',
                 cache_creation_cost_per_million = '0'
             WHERE model_id = 'deepseek-v4-pro'",
            [],
        )
        .expect("restore old DeepSeek price");
        conn.execute(
            "UPDATE model_pricing
             SET input_cost_per_million = '9',
                 output_cost_per_million = '9',
                 cache_read_cost_per_million = '9',
                 cache_creation_cost_per_million = '0'
             WHERE model_id = 'glm-5.1'",
            [],
        )
        .expect("set custom GLM price");
    }

    db.ensure_model_pricing_seeded()
        .expect("ensure pricing seeded");

    let conn = db.conn.lock().expect("lock conn");
    let deepseek: (String, String, String) = conn
        .query_row(
            "SELECT input_cost_per_million, output_cost_per_million, cache_read_cost_per_million
             FROM model_pricing WHERE model_id = 'deepseek-v4-pro'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("query DeepSeek price");
    assert_eq!(
        deepseek,
        (
            "0.435".to_string(),
            "0.87".to_string(),
            "0.003625".to_string()
        )
    );

    let glm: (String, String, String) = conn
        .query_row(
            "SELECT input_cost_per_million, output_cost_per_million, cache_read_cost_per_million
             FROM model_pricing WHERE model_id = 'glm-5.1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("query GLM price");
    assert_eq!(glm, ("9".to_string(), "9".to_string(), "9".to_string()));
}

#[test]
fn ensure_incremental_auto_vacuum_rebuilds_existing_file_db() {
    let temp = NamedTempFile::new().expect("create temp db file");
    let path = temp.path().to_path_buf();

    let conn = Connection::open(&path).expect("open temp db");
    conn.execute("PRAGMA auto_vacuum = NONE;", [])
        .expect("set none auto_vacuum");
    Database::create_tables_on_conn(&conn).expect("create tables");

    assert_eq!(
        Database::get_auto_vacuum_mode(&conn).expect("auto_vacuum before rebuild"),
        0,
        "existing file db should start with NONE auto_vacuum"
    );

    let rebuilt =
        Database::ensure_incremental_auto_vacuum_on_conn(&conn).expect("enable incremental mode");
    assert!(rebuilt, "existing db should require rebuild via VACUUM");
    drop(conn);

    let reopened = Connection::open(&path).expect("reopen temp db");
    assert_eq!(
        Database::get_auto_vacuum_mode(&reopened).expect("auto_vacuum after rebuild"),
        2,
        "file db should persist INCREMENTAL auto_vacuum after VACUUM rebuild"
    );
}

const V12_USAGE_MIGRATION_FIXTURE_SQL: &str = r#"
    CREATE TABLE providers (
        id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        name TEXT NOT NULL,
        settings_config TEXT NOT NULL,
        website_url TEXT,
        category TEXT,
        created_at INTEGER,
        sort_index INTEGER,
        notes TEXT,
        icon TEXT,
        icon_color TEXT,
        meta TEXT NOT NULL DEFAULT '{}',
        is_current BOOLEAN NOT NULL DEFAULT 0,
        in_failover_queue BOOLEAN NOT NULL DEFAULT 0,
        PRIMARY KEY (id, app_type)
    );
    CREATE TABLE proxy_request_logs (
        request_id TEXT PRIMARY KEY,
        provider_id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        model TEXT NOT NULL,
        request_model TEXT,
        pricing_model TEXT,
        input_tokens INTEGER NOT NULL DEFAULT 0,
        output_tokens INTEGER NOT NULL DEFAULT 0,
        cache_read_tokens INTEGER NOT NULL DEFAULT 0,
        cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
        input_cost_usd TEXT NOT NULL DEFAULT '0',
        output_cost_usd TEXT NOT NULL DEFAULT '0',
        cache_read_cost_usd TEXT NOT NULL DEFAULT '0',
        cache_creation_cost_usd TEXT NOT NULL DEFAULT '0',
        total_cost_usd TEXT NOT NULL DEFAULT '0',
        latency_ms INTEGER NOT NULL,
        first_token_ms INTEGER,
        duration_ms INTEGER,
        status_code INTEGER NOT NULL,
        error_message TEXT,
        session_id TEXT,
        provider_type TEXT,
        is_streaming INTEGER NOT NULL DEFAULT 0,
        cost_multiplier TEXT NOT NULL DEFAULT '1.0',
        created_at INTEGER NOT NULL,
        data_source TEXT NOT NULL DEFAULT 'proxy'
    );
"#;

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .expect("count table rows")
}

fn scalar_i64(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0))
        .expect("query integer scalar")
}

fn scalar_text(conn: &Connection, sql: &str) -> String {
    conn.query_row(sql, [], |row| row.get(0))
        .expect("query text scalar")
}

fn insert_v12_provider(
    conn: &Connection,
    id: &str,
    app_type: &str,
    name: &str,
    category: Option<&str>,
    settings_config: serde_json::Value,
    meta: serde_json::Value,
) {
    conn.execute(
        "INSERT INTO providers
         (id, app_type, name, settings_config, category, meta, is_current)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
        params![
            id,
            app_type,
            name,
            settings_config.to_string(),
            category,
            meta.to_string()
        ],
    )
    .expect("insert v12 provider");
}

fn insert_v12_log(
    conn: &Connection,
    request_id: &str,
    provider_id: &str,
    app_type: &str,
    data_source: &str,
) {
    conn.execute(
        "INSERT INTO proxy_request_logs (
            request_id, provider_id, app_type, model, request_model, pricing_model,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            input_cost_usd, output_cost_usd, cache_read_cost_usd,
            cache_creation_cost_usd, total_cost_usd, latency_ms, status_code,
            session_id, created_at, data_source
         ) VALUES (
            ?1, ?2, ?3, 'claude-sonnet-4-6', 'requested-model', 'priced-model',
            11, 7, 3, 2, '0.11', '0.21', '0.03', '0.02', '0.37', 42, 200,
            'session-stable-id', 1720000000, ?4
         )",
        params![request_id, provider_id, app_type, data_source],
    )
    .expect("insert v12 proxy log");
}

fn true_v12_usage_fixture() -> Connection {
    let conn = Connection::open_in_memory().expect("open v12 fixture");
    conn.execute_batch(V12_USAGE_MIGRATION_FIXTURE_SQL)
        .expect("create v12 fixture schema");
    Database::create_tables_on_conn(&conn).expect("complete real startup table set");

    insert_v12_provider(
        &conn,
        "codex-oauth",
        "claude",
        "Codex OAuth",
        Some("official"),
        json!({"env": {"ANTHROPIC_BASE_URL": "https://chatgpt.com/backend-api/codex"}}),
        json!({"providerType": "codex_oauth"}),
    );
    insert_v12_provider(
        &conn,
        "copilot",
        "claude",
        "GitHub Copilot",
        Some("official"),
        json!({"env": {"ANTHROPIC_BASE_URL": "https://api.githubcopilot.com"}}),
        json!({"providerType": "github_copilot"}),
    );
    insert_v12_provider(
        &conn,
        "token-plan",
        "codex",
        "Token Plan",
        Some("custom"),
        json!({"base_url": "https://plan.example.com"}),
        json!({"usage_script": {"enabled": true, "templateType": "token_plan"}}),
    );
    insert_v12_provider(
        &conn,
        "official-subscription",
        "claude",
        "Official Subscription",
        Some("official"),
        json!({"base_url": "https://subscription.example.com"}),
        json!({"usage_script": {"enabled": true, "templateType": "official_subscription"}}),
    );
    insert_v12_provider(
        &conn,
        "metered",
        "claude",
        "Metered",
        Some("custom"),
        json!({"env": {"ANTHROPIC_BASE_URL": "https://metered.example.com", "ANTHROPIC_AUTH_TOKEN": "secret"}}),
        json!({}),
    );
    insert_v12_provider(
        &conn,
        "ambiguous",
        "gemini",
        "Ambiguous Official",
        Some("official"),
        json!({"base_url": "https://ambiguous.example.com"}),
        json!({}),
    );
    insert_v12_provider(
        &conn,
        "_session",
        "claude",
        "Session Placeholder",
        Some("custom"),
        json!({}),
        json!({}),
    );

    insert_v12_log(&conn, "request-proxy", "metered", "claude", "proxy");
    insert_v12_log(&conn, "request-placeholder", "_session", "claude", "proxy");
    insert_v12_log(
        &conn,
        "request-session",
        "codex-oauth",
        "claude",
        "session_log",
    );
    Database::set_user_version(&conn, 12).expect("set v12 user_version");
    conn
}

#[test]
fn migration_v12_to_v13_preserves_legacy_rows_and_imports_only_proxy_events() {
    let conn = true_v12_usage_fixture();
    let legacy_provider_count = count(&conn, "providers");
    let legacy_log_count = count(&conn, "proxy_request_logs");

    Database::apply_schema_migrations_on_conn(&conn).expect("migrate v12 to v13");

    assert_eq!(Database::get_user_version(&conn).unwrap(), SCHEMA_VERSION);
    assert_eq!(count(&conn, "providers"), legacy_provider_count);
    assert_eq!(count(&conn, "proxy_request_logs"), legacy_log_count);
    assert_eq!(
        count(&conn, "usage_providers"),
        legacy_provider_count
            + crate::usage::system_providers::system_provider_definitions().len() as i64
    );
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT COUNT(*) FROM usage_providers WHERE system_preset_key IS NULL"
        ),
        legacy_provider_count
    );
    assert_eq!(count(&conn, "usage_events"), 1);
    assert_eq!(
        scalar_text(&conn, "SELECT cost_source FROM usage_events LIMIT 1"),
        "estimated"
    );
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT COUNT(*) FROM usage_events
             WHERE legacy_request_id='request-placeholder'"
        ),
        0
    );
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT COUNT(*) FROM usage_events WHERE provider_id='claude:_session'"
        ),
        0
    );
    assert_eq!(
        scalar_text(&conn, "SELECT event_id FROM usage_events LIMIT 1"),
        "legacy:request-proxy"
    );
    assert_eq!(
        scalar_text(&conn, "SELECT provider_id FROM usage_events LIMIT 1"),
        "claude:metered"
    );
    assert_eq!(
        scalar_text(&conn, "SELECT request_id FROM usage_events LIMIT 1"),
        "request-proxy"
    );
    assert_eq!(
        scalar_text(&conn, "SELECT total_cost_usd FROM usage_events LIMIT 1"),
        "0.37"
    );

    let provider_rows: Vec<(String, String, String, i64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT id, billing_kind, token_sources, needs_review
                 FROM usage_providers ORDER BY id",
            )
            .expect("prepare provider classifications");
        stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("query provider classifications")
        .collect::<Result<_, _>>()
        .expect("collect provider classifications")
    };

    for id in [
        "claude:codex-oauth",
        "claude:copilot",
        "codex:token-plan",
        "claude:official-subscription",
    ] {
        assert!(provider_rows.iter().any(|row| {
            row.0 == id && row.1 == "subscription" && row.2 == "[\"session_log\"]" && row.3 == 0
        }));
    }
    assert!(provider_rows.iter().any(|row| {
        row.0 == "claude:metered" && row.1 == "metered" && row.2 == "[\"proxy\"]" && row.3 == 0
    }));
    assert!(provider_rows.iter().any(|row| {
        row.0 == "gemini:ambiguous" && row.1 == "metered" && row.2 == "[\"proxy\"]" && row.3 == 1
    }));

    let route_config: serde_json::Value = serde_json::from_str(&scalar_text(
        &conn,
        "SELECT route_config FROM usage_providers WHERE id='claude:metered'",
    ))
    .expect("route_config should remain JSON");
    assert_eq!(
        route_config,
        json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://metered.example.com",
                "ANTHROPIC_AUTH_TOKEN": "secret"
            }
        })
    );

    Database::apply_schema_migrations_on_conn(&conn).expect("second migration is idempotent");
    assert_eq!(
        count(&conn, "usage_providers"),
        legacy_provider_count
            + crate::usage::system_providers::system_provider_definitions().len() as i64
    );
    assert_eq!(count(&conn, "usage_events"), 1);

    assert!(conn
        .execute(
            "UPDATE usage_events SET model='changed' WHERE event_id='legacy:request-proxy'",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "DELETE FROM usage_events WHERE event_id='legacy:request-proxy'",
            [],
        )
        .is_err());
}

#[test]
fn migration_v12_to_v13_rejects_quota_snapshot_updates_and_preserves_the_row() {
    let conn = true_v12_usage_fixture();
    Database::apply_schema_migrations_on_conn(&conn).expect("migrate v12 to v13");
    conn.execute(
        "INSERT INTO quota_snapshots (
            snapshot_id, provider_id, fetched_at, five_hour_utilization_percent,
            raw_payload, created_at
         ) VALUES ('quota-1', 'claude:metered', 1720000000, '25.5', '{}', 1720000000)",
        [],
    )
    .expect("insert quota snapshot");

    let update = conn.execute(
        "UPDATE quota_snapshots
         SET five_hour_utilization_percent='99.9'
         WHERE snapshot_id='quota-1'",
        [],
    );

    assert!(update.is_err(), "quota snapshot UPDATE must abort");
    assert_eq!(
        scalar_text(
            &conn,
            "SELECT five_hour_utilization_percent
             FROM quota_snapshots WHERE snapshot_id='quota-1'"
        ),
        "25.5"
    );
}

#[test]
fn migration_v12_to_v13_rejects_quota_snapshot_deletes_and_preserves_the_row() {
    let conn = true_v12_usage_fixture();
    Database::apply_schema_migrations_on_conn(&conn).expect("migrate v12 to v13");
    conn.execute(
        "INSERT INTO quota_snapshots (
            snapshot_id, provider_id, fetched_at, five_hour_utilization_percent,
            raw_payload, created_at
         ) VALUES ('quota-1', 'claude:metered', 1720000000, '25.5', '{}', 1720000000)",
        [],
    )
    .expect("insert quota snapshot");

    let delete = conn.execute(
        "DELETE FROM quota_snapshots WHERE snapshot_id='quota-1'",
        [],
    );

    assert!(delete.is_err(), "quota snapshot DELETE must abort");
    assert_eq!(count(&conn, "quota_snapshots"), 1);
    assert_eq!(
        scalar_text(
            &conn,
            "SELECT five_hour_utilization_percent
             FROM quota_snapshots WHERE snapshot_id='quota-1'"
        ),
        "25.5"
    );
}

#[test]
fn migration_v12_to_v13_rolls_back_tables_rows_and_version_on_trigger_failure() {
    let conn = true_v12_usage_fixture();
    conn.execute_batch(
        "CREATE TABLE usage_events (
            event_id TEXT PRIMARY KEY,
            source TEXT NOT NULL,
            provider_id TEXT NOT NULL,
            product_group_id TEXT NOT NULL,
            occurred_at INTEGER NOT NULL,
            model TEXT NOT NULL,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            request_id TEXT,
            session_id TEXT,
            upstream_correlation_id TEXT,
            input_cost_usd TEXT,
            output_cost_usd TEXT,
            cache_read_cost_usd TEXT,
            cache_creation_cost_usd TEXT,
            total_cost_usd TEXT,
            cost_source TEXT NOT NULL,
            legacy_request_id TEXT UNIQUE,
            created_at INTEGER NOT NULL
        );
        CREATE TRIGGER force_usage_event_import_failure
        BEFORE INSERT ON usage_events
        BEGIN
            SELECT RAISE(ABORT, 'forced v13 import failure');
        END;",
    )
    .expect("install forced failure trigger");

    let error = Database::apply_schema_migrations_on_conn(&conn)
        .expect_err("forced trigger should fail migration");
    assert!(error.to_string().contains("forced v13 import failure"));
    assert_eq!(Database::get_user_version(&conn).unwrap(), 12);
    assert_eq!(count(&conn, "providers"), 7);
    assert_eq!(count(&conn, "proxy_request_logs"), 3);
    assert_eq!(count(&conn, "usage_events"), 0);

    for table in [
        "usage_providers",
        "route_bindings",
        "usage_source_bindings",
        "usage_event_links",
        "quota_snapshots",
        "quota_fetch_state",
    ] {
        assert_eq!(
            scalar_i64(
                &conn,
                &format!(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='{table}'"
                )
            ),
            0,
            "{table} should roll back"
        );
    }
}
