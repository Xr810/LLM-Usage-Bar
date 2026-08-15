//! Durable source-aware session usage synchronization cursors.

use crate::error::AppError;
use crate::store::{lock_conn, Database};
use rusqlite::{params, Connection, OptionalExtension, Row};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageSyncCursor {
    pub source: String,
    pub cursor_key: String,
    pub resource_path: Option<String>,
    pub resource_identity: Option<String>,
    pub modified_at_ns: i64,
    pub size_bytes: i64,
    pub byte_offset: i64,
    pub line_offset: i64,
    pub parser_state_json: Option<String>,
    pub last_success_at: i64,
}

fn usage_sync_cursor_from_row(row: &Row<'_>) -> rusqlite::Result<UsageSyncCursor> {
    Ok(UsageSyncCursor {
        source: row.get(0)?,
        cursor_key: row.get(1)?,
        resource_path: row.get(2)?,
        resource_identity: row.get(3)?,
        modified_at_ns: row.get(4)?,
        size_bytes: row.get(5)?,
        byte_offset: row.get(6)?,
        line_offset: row.get(7)?,
        parser_state_json: row.get(8)?,
        last_success_at: row.get(9)?,
    })
}

fn validate_usage_sync_cursor(cursor: &UsageSyncCursor) -> Result<(), AppError> {
    for (field, value) in [
        ("size_bytes", cursor.size_bytes),
        ("byte_offset", cursor.byte_offset),
        ("line_offset", cursor.line_offset),
    ] {
        if value < 0 {
            return Err(AppError::InvalidInput(format!(
                "usage sync cursor {field} must be non-negative"
            )));
        }
    }
    Ok(())
}

impl Database {
    pub(crate) fn create_usage_sync_cursors_table_on_conn(
        conn: &Connection,
    ) -> Result<(), AppError> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS usage_sync_cursors (
                source TEXT NOT NULL,
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
            )",
            [],
        )
        .map_err(AppError::from)?;
        Ok(())
    }

    pub fn get_usage_sync_cursor(
        &self,
        source: &str,
        cursor_key: &str,
    ) -> Result<Option<UsageSyncCursor>, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT source, cursor_key, resource_path, resource_identity,
                    modified_at_ns, size_bytes, byte_offset, line_offset,
                    parser_state_json, last_success_at
             FROM usage_sync_cursors
             WHERE source = ?1 AND cursor_key = ?2",
            params![source, cursor_key],
            usage_sync_cursor_from_row,
        )
        .optional()
        .map_err(AppError::from)
    }

    pub fn list_usage_sync_cursors(&self, source: &str) -> Result<Vec<UsageSyncCursor>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut statement = conn.prepare(
            "SELECT source, cursor_key, resource_path, resource_identity,
                    modified_at_ns, size_bytes, byte_offset, line_offset,
                    parser_state_json, last_success_at
             FROM usage_sync_cursors
             WHERE source = ?1
             ORDER BY cursor_key",
        )?;
        let rows = statement.query_map([source], usage_sync_cursor_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn put_usage_sync_cursor(&self, cursor: &UsageSyncCursor) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        Self::put_usage_sync_cursor_on_conn(&conn, cursor)
    }

    /// Atomically publishes a cursor under a stable key and retires its legacy key.
    pub fn promote_usage_sync_cursor(
        &self,
        legacy_source: &str,
        legacy_cursor_key: &str,
        cursor: &UsageSyncCursor,
    ) -> Result<(), AppError> {
        validate_usage_sync_cursor(cursor)?;
        if legacy_source == cursor.source && legacy_cursor_key == cursor.cursor_key {
            return self.put_usage_sync_cursor(cursor);
        }

        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction()?;
        Self::put_usage_sync_cursor_on_conn(&transaction, cursor)?;
        transaction.execute(
            "DELETE FROM usage_sync_cursors
             WHERE source = ?1 AND cursor_key = ?2",
            params![legacy_source, legacy_cursor_key],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn put_usage_sync_cursor_on_conn(
        conn: &Connection,
        cursor: &UsageSyncCursor,
    ) -> Result<(), AppError> {
        validate_usage_sync_cursor(cursor)?;
        conn.execute(
            "INSERT INTO usage_sync_cursors (
                source, cursor_key, resource_path, resource_identity,
                modified_at_ns, size_bytes, byte_offset, line_offset,
                parser_state_json, last_success_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(source, cursor_key) DO UPDATE SET
                resource_path = excluded.resource_path,
                resource_identity = excluded.resource_identity,
                modified_at_ns = excluded.modified_at_ns,
                size_bytes = excluded.size_bytes,
                byte_offset = excluded.byte_offset,
                line_offset = excluded.line_offset,
                parser_state_json = excluded.parser_state_json,
                last_success_at = excluded.last_success_at",
            params![
                cursor.source,
                cursor.cursor_key,
                cursor.resource_path,
                cursor.resource_identity,
                cursor.modified_at_ns,
                cursor.size_bytes,
                cursor.byte_offset,
                cursor.line_offset,
                cursor.parser_state_json,
                cursor.last_success_at,
            ],
        )?;
        Ok(())
    }

    pub fn delete_usage_sync_cursor(&self, source: &str, cursor_key: &str) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "DELETE FROM usage_sync_cursors WHERE source = ?1 AND cursor_key = ?2",
            params![source, cursor_key],
        )?;
        Ok(())
    }
}
