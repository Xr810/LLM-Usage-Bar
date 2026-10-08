use crate::config::source_roots::UsageSourceRoots;
use crate::error::AppError;
use crate::store::{Database, UsageSyncCursor};
use rusqlite::Connection;
use std::path::Path;

pub(crate) fn classify_legacy_cursor(path: &str, roots: &UsageSourceRoots) -> &'static str {
    let path = Path::new(path);
    if !path.is_absolute() {
        "legacy"
    } else if path.starts_with(&roots.claude) {
        "claude"
    } else if path.starts_with(&roots.codex) {
        "codex"
    } else if path.starts_with(&roots.gemini) {
        "gemini"
    } else if path.starts_with(&roots.opencode) {
        "opencode"
    } else {
        "legacy"
    }
}

pub(crate) fn migrate_v13_to_v14(
    conn: &Connection,
    roots: &UsageSourceRoots,
) -> Result<(), AppError> {
    const ARCHIVE_TABLE: &str = "session_log_sync_v13_archive";
    if Database::table_exists(conn, ARCHIVE_TABLE)? {
        return Err(AppError::Database(format!(
            "schema v14 migration cannot overwrite existing {ARCHIVE_TABLE}"
        )));
    }
    if !Database::table_exists(conn, "session_log_sync")? {
        return Err(AppError::Database(
            "schema v14 migration requires session_log_sync".to_string(),
        ));
    }

    Database::create_usage_sync_cursors_table_on_conn(conn)?;
    conn.execute(
        "ALTER TABLE session_log_sync RENAME TO session_log_sync_v13_archive",
        [],
    )
    .map_err(|error| AppError::Database(format!("archive v13 sync state: {error}")))?;

    let legacy_rows = {
        let mut statement = conn
            .prepare(
                "SELECT file_path, last_modified, last_line_offset, last_synced_at
                 FROM session_log_sync_v13_archive ORDER BY file_path",
            )
            .map_err(AppError::from)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(AppError::from)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)?
    };

    for (path, modified_at_ns, line_offset, last_success_at) in legacy_rows {
        let cursor = UsageSyncCursor {
            source: classify_legacy_cursor(&path, roots).to_string(),
            cursor_key: path.clone(),
            resource_path: Some(path),
            resource_identity: None,
            modified_at_ns,
            size_bytes: 0,
            byte_offset: 0,
            line_offset,
            parser_state_json: None,
            last_success_at,
        };
        Database::put_usage_sync_cursor_on_conn(conn, &cursor)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Database;
    use rusqlite::Connection;
    use std::path::PathBuf;

    fn roots() -> UsageSourceRoots {
        let home = if cfg!(windows) {
            PathBuf::from("C:/Users/test")
        } else {
            PathBuf::from("/Users/test")
        };
        UsageSourceRoots {
            claude: home.join(".claude/projects"),
            codex: home.join(".codex"),
            gemini: home.join(".gemini/tmp"),
            opencode: home.join(".local/share/opencode"),
        }
    }

    fn v13_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE providers (id TEXT);
             CREATE TABLE mcp_servers (id TEXT);
             CREATE TABLE settings (key TEXT);
             CREATE TABLE profiles (id TEXT);
             CREATE TABLE session_log_sync (
                 file_path TEXT PRIMARY KEY,
                 last_modified INTEGER NOT NULL DEFAULT 0,
                 last_line_offset INTEGER NOT NULL DEFAULT 0,
                 last_synced_at INTEGER NOT NULL DEFAULT 0
             );
             PRAGMA user_version = 13;",
        )
        .unwrap();
        conn
    }

    #[test]
    fn classifies_only_paths_below_resolved_external_roots() {
        let roots = roots();
        for (path, source) in [
            (roots.claude.join("p/a.jsonl"), "claude"),
            (roots.codex.join("sessions/a.jsonl"), "codex"),
            (roots.gemini.join("p/chats/session-a.json"), "gemini"),
            (roots.opencode.join("opencode.db"), "opencode"),
            (
                roots
                    .claude
                    .with_file_name("projects-other")
                    .join("a.jsonl"),
                "legacy",
            ),
            (
                roots.codex.with_file_name(".codex-other").join("a.jsonl"),
                "legacy",
            ),
            (
                roots.gemini.with_file_name("tmp-other").join("a.json"),
                "legacy",
            ),
            (
                roots.opencode.with_file_name("opencode-other").join("a.db"),
                "legacy",
            ),
            (
                roots.claude.parent().unwrap().join("outside.jsonl"),
                "legacy",
            ),
        ] {
            assert!(
                path.is_absolute(),
                "fixture must be native absolute: {path:?}"
            );
            assert_eq!(
                classify_legacy_cursor(path.to_str().unwrap(), &roots),
                source
            );
        }
        assert_eq!(classify_legacy_cursor("relative.jsonl", &roots), "legacy");
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_require_a_drive_or_unc_prefix_and_a_root() {
        let mut roots = roots();
        for path in [
            r"C:\Users\test\.claude\projects\p\a.jsonl",
            "C:/Users/test/.claude/projects/p/a.jsonl",
        ] {
            assert_eq!(classify_legacy_cursor(path, &roots), "claude");
        }
        for path in [
            r"C:Users\test\.claude\projects\p\a.jsonl",
            r"\Users\test\.claude\projects\p\a.jsonl",
            "/Users/test/.claude/projects/p/a.jsonl",
            r"D:\Users\test\.claude\projects\p\a.jsonl",
        ] {
            assert_eq!(classify_legacy_cursor(path, &roots), "legacy");
        }
        for root in [
            r"\\server\share\.claude\projects",
            r"\\?\C:\Users\test\.claude\projects",
        ] {
            roots.claude = PathBuf::from(root);
            assert!(roots.claude.is_absolute());
            let path = roots.claude.join(r"p\a.jsonl");
            assert_eq!(
                classify_legacy_cursor(path.to_str().unwrap(), &roots),
                "claude"
            );
            let outside = roots
                .claude
                .with_file_name("projects-other")
                .join("a.jsonl");
            assert_eq!(
                classify_legacy_cursor(outside.to_str().unwrap(), &roots),
                "legacy"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn windows_prefixes_do_not_make_unix_paths_absolute() {
        let mut roots = roots();
        for root in [
            r"C:\Users\test\.claude\projects",
            r"\\server\share\.claude\projects",
        ] {
            roots.claude = PathBuf::from(root);
            let path = roots.claude.join("a.jsonl");
            assert!(!path.is_absolute());
            assert_eq!(
                classify_legacy_cursor(path.to_str().unwrap(), &roots),
                "legacy"
            );
        }
    }

    #[test]
    fn migration_archives_v13_rows_and_builds_conservative_source_cursors() {
        let conn = v13_connection();
        let roots = roots();
        for (path, modified, line, synced) in [
            (roots.claude.join("p/a.jsonl"), 11, 3, 101),
            (roots.codex.join("sessions/a.jsonl"), 12, 4, 102),
            (roots.gemini.join("p/chats/session-a.json"), 13, 5, 103),
            (roots.opencode.join("opencode.db"), 14, 6, 104),
            (
                roots.claude.parent().unwrap().join("unknown.jsonl"),
                15,
                7,
                105,
            ),
        ] {
            conn.execute(
                "INSERT INTO session_log_sync VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![path.to_str().unwrap(), modified, line, synced],
            )
            .unwrap();
        }

        migrate_v13_to_v14(&conn, &roots).unwrap();

        assert!(!Database::table_exists(&conn, "session_log_sync").unwrap());
        assert!(Database::table_exists(&conn, "session_log_sync_v13_archive").unwrap());
        assert!(Database::table_exists(&conn, "usage_sync_cursors").unwrap());
        let archive_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM session_log_sync_v13_archive",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(archive_count, 5);

        let mut statement = conn
            .prepare(
                "SELECT source, cursor_key, resource_path, modified_at_ns,
                        size_bytes, byte_offset, line_offset, parser_state_json,
                        last_success_at
                 FROM usage_sync_cursors ORDER BY last_success_at",
            )
            .unwrap();
        let cursors = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(cursors.len(), 5);
        assert_eq!(cursors[0].0, "claude");
        assert_eq!(cursors[1].0, "codex");
        assert_eq!(cursors[2].0, "gemini");
        assert_eq!(cursors[3].0, "opencode");
        assert_eq!(cursors[4].0, "legacy");
        for (index, cursor) in cursors.iter().enumerate() {
            assert_eq!(cursor.1, cursor.2.as_deref().unwrap());
            assert_eq!(cursor.3, 11 + index as i64);
            assert_eq!(cursor.4, 0);
            assert_eq!(cursor.5, 0);
            assert_eq!(cursor.6, 3 + index as i64);
            assert_eq!(cursor.7, None);
            assert_eq!(cursor.8, 101 + index as i64);
        }
        assert_eq!(Database::get_user_version(&conn).unwrap(), 13);
    }

    #[test]
    fn existing_archive_fails_closed_without_overwriting_evidence() {
        let conn = v13_connection();
        conn.execute_batch(
            "CREATE TABLE session_log_sync_v13_archive (sentinel TEXT);
             INSERT INTO session_log_sync_v13_archive VALUES ('keep');",
        )
        .unwrap();

        let error = migrate_v13_to_v14(&conn, &roots()).expect_err("archive collision must fail");

        assert!(error.to_string().contains("session_log_sync_v13_archive"));
        assert!(Database::table_exists(&conn, "session_log_sync").unwrap());
        let sentinel: String = conn
            .query_row(
                "SELECT sentinel FROM session_log_sync_v13_archive",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sentinel, "keep");
    }
}
