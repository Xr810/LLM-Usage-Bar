//! Read-only session list for C14 (BE-22).
//!
//! Proxy rows, including ones that already have a `session_id`, collapse into
//! one `proxy_traffic` item. JSONL rows with a session id stay `session`.
//! Non-proxy rows with no session id are `unattributed`, not a fake session.
//! `projectAlias` is always null: that column does not exist until BE-15.
//! Amounts are summed with `rust_decimal`, not SQL REAL.
//!
//! TODO(BE-22): aggregate and paginate in SQL. This batch scans every row
//! and pages in memory.

use std::collections::HashMap;
use std::str::FromStr;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::store::Database;

pub const ACTIVE_WINDOW_SECS: i64 = 30 * 60;
pub const DEFAULT_SESSION_LIMIT: u32 = 50;
pub const MAX_SESSION_LIMIT: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionGroup {
    Session,
    ProxyTraffic,
    Unattributed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionSpendState {
    Ok,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionListItem {
    pub session_id: Option<String>,
    pub group: SessionGroup,
    /// Always null until BE-15 adds `project_alias`. The field is present.
    pub project_alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_tag: Option<String>,
    pub spent_usd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap_usd: Option<String>,
    pub state: SessionSpendState,
    pub last_seen_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionListPage {
    pub items: Vec<SessionListItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub oldest_available_at: Option<i64>,
    /// Rows whose `total_cost_usd` did not parse. Those rows still count as
    /// zero spend. The raw value is not returned.
    pub invalid_cost_rows: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct SessionCursor {
    last_seen_at: i64,
    session_key: String,
    group: String,
}

pub fn resolve_session_limit(limit: Option<u32>) -> Result<u32, AppError> {
    match limit {
        None => Ok(DEFAULT_SESSION_LIMIT),
        Some(value) if (1..=MAX_SESSION_LIMIT).contains(&value) => Ok(value),
        Some(_) => Err(AppError::invalid_argument(
            "limit is out of range.",
            "limit",
        )),
    }
}

pub fn list_sessions_at(
    db: &Database,
    now: i64,
    active_only: bool,
    limit: Option<u32>,
    cursor: Option<&str>,
) -> Result<SessionListPage, AppError> {
    let limit = resolve_session_limit(limit)?;
    let cursor = cursor
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(decode_cursor)
        .transpose()?;
    // TODO(BE-22): replace this full scan with SQL aggregation.
    let rows = load_rows(db)?;
    let oldest_available_at = rows.iter().map(|row| row.created_at).min();
    let mut groups = HashMap::<GroupKey, GroupAccum>::new();
    let mut invalid_cost_rows = 0_u32;
    for row in rows {
        let (spent, invalid) = parse_cost(&row.request_id, &row.total_cost_usd);
        if invalid {
            invalid_cost_rows = invalid_cost_rows.saturating_add(1);
        }
        let key = group_key(&row);
        let entry = groups.entry(key).or_default();
        entry.last_seen_at = entry.last_seen_at.max(row.created_at);
        entry.spent += spent;
    }
    let mut items = groups
        .into_iter()
        .map(|(key, accum)| item_from_group(key, accum))
        .filter(|item| !active_only || item.last_seen_at.saturating_add(ACTIVE_WINDOW_SECS) >= now)
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        right
            .last_seen_at
            .cmp(&left.last_seen_at)
            .then_with(|| sort_key(left).cmp(&sort_key(right)))
    });
    if let Some(cursor) = cursor.as_ref() {
        items.retain(|item| cursor_is_before(cursor, item));
    }
    let next_cursor = if items.len() > limit as usize {
        Some(encode_cursor(&items[limit as usize - 1])?)
    } else {
        None
    };
    items.truncate(limit as usize);
    Ok(SessionListPage {
        items,
        next_cursor,
        oldest_available_at,
        invalid_cost_rows,
    })
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct GroupKey {
    kind: GroupKind,
    session_key: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum GroupKind {
    Proxy,
    Session,
    Unattributed,
}

struct RawRow {
    request_id: String,
    data_source: String,
    session_id: Option<String>,
    total_cost_usd: String,
    created_at: i64,
}

#[derive(Default)]
struct GroupAccum {
    last_seen_at: i64,
    spent: Decimal,
}

fn load_rows(db: &Database) -> Result<Vec<RawRow>, AppError> {
    let conn = db
        .conn
        .lock()
        .map_err(|_| AppError::internal("Could not list sessions."))?;
    let mut statement = conn
        .prepare(
            "SELECT request_id, COALESCE(data_source, 'proxy'), session_id, total_cost_usd, created_at
             FROM proxy_request_logs",
        )
        .map_err(|_| AppError::internal("Could not list sessions."))?;
    let mut rows = statement
        .query([])
        .map_err(|_| AppError::internal("Could not list sessions."))?;
    let mut loaded = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|_| AppError::internal("Could not list sessions."))?
    {
        loaded.push(RawRow {
            request_id: row
                .get(0)
                .map_err(|_| AppError::internal("Could not list sessions."))?,
            data_source: row
                .get(1)
                .map_err(|_| AppError::internal("Could not list sessions."))?,
            session_id: row
                .get(2)
                .map_err(|_| AppError::internal("Could not list sessions."))?,
            total_cost_usd: row
                .get(3)
                .map_err(|_| AppError::internal("Could not list sessions."))?,
            created_at: row
                .get(4)
                .map_err(|_| AppError::internal("Could not list sessions."))?,
        });
    }
    Ok(loaded)
}

fn group_key(row: &RawRow) -> GroupKey {
    if row.data_source == "proxy" {
        return GroupKey {
            kind: GroupKind::Proxy,
            session_key: String::new(),
        };
    }
    let Some(session_key) = row
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return GroupKey {
            kind: GroupKind::Unattributed,
            session_key: String::new(),
        };
    };
    GroupKey {
        kind: GroupKind::Session,
        session_key: session_key.to_string(),
    }
}

fn item_from_group(key: GroupKey, accum: GroupAccum) -> SessionListItem {
    let (group, session_id) = match key.kind {
        GroupKind::Proxy => (SessionGroup::ProxyTraffic, None),
        GroupKind::Unattributed => (SessionGroup::Unattributed, None),
        GroupKind::Session => (SessionGroup::Session, Some(key.session_key.clone())),
    };
    SessionListItem {
        session_id,
        group,
        project_alias: None,
        client_tag: None,
        spent_usd: format_cost(accum.spent),
        cap_usd: None,
        state: SessionSpendState::Ok,
        last_seen_at: accum.last_seen_at,
    }
}

fn sort_key(item: &SessionListItem) -> String {
    let group = match item.group {
        SessionGroup::ProxyTraffic => "proxy",
        SessionGroup::Session => "session",
        SessionGroup::Unattributed => "unattributed",
    };
    format!(
        "{group}\u{0}{}",
        item.session_id.clone().unwrap_or_default()
    )
}

fn cursor_is_before(cursor: &SessionCursor, item: &SessionListItem) -> bool {
    item.last_seen_at < cursor.last_seen_at
        || (item.last_seen_at == cursor.last_seen_at && sort_key(item) > cursor.session_key)
}

fn encode_cursor(item: &SessionListItem) -> Result<String, AppError> {
    let cursor = SessionCursor {
        last_seen_at: item.last_seen_at,
        session_key: sort_key(item),
        group: match item.group {
            SessionGroup::Session => "session".to_string(),
            SessionGroup::ProxyTraffic => "proxy_traffic".to_string(),
            SessionGroup::Unattributed => "unattributed".to_string(),
        },
    };
    let bytes =
        serde_json::to_vec(&cursor).map_err(|_| AppError::internal("Could not list sessions."))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn decode_cursor(text: &str) -> Result<SessionCursor, AppError> {
    let bytes = URL_SAFE_NO_PAD.decode(text).map_err(|_| invalid_cursor())?;
    serde_json::from_slice(&bytes).map_err(|_| invalid_cursor())
}

fn invalid_cursor() -> AppError {
    AppError::invalid_argument("cursor is invalid.", "cursor")
}

fn parse_cost(request_id: &str, raw: &str) -> (Decimal, bool) {
    match Decimal::from_str(raw.trim()) {
        Ok(value) => (value, false),
        Err(_) => {
            log::warn!("invalid session cost; request_id={request_id}");
            (Decimal::ZERO, true)
        }
    }
}

fn format_cost(value: Decimal) -> String {
    value.normalize().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(
        db: &Database,
        request_id: &str,
        data_source: &str,
        session_id: Option<&str>,
        cost: &str,
        created_at: i64,
    ) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO proxy_request_logs (
                request_id, provider_id, app_type, model, latency_ms, status_code,
                total_cost_usd, session_id, created_at, data_source
             ) VALUES (?1, 'p', 'codex', 'm', 1, 200, ?2, ?3, ?4, ?5)",
            rusqlite::params![request_id, cost, session_id, created_at, data_source],
        )
        .unwrap();
    }

    #[test]
    fn limit_bounds_and_empty_table_keep_oldest_null() {
        assert_eq!(resolve_session_limit(None).unwrap(), 50);
        assert!(resolve_session_limit(Some(0)).is_err());
        assert!(resolve_session_limit(Some(201)).is_err());
        assert_eq!(resolve_session_limit(Some(200)).unwrap(), 200);
        let db = Database::memory().unwrap();
        let page = list_sessions_at(&db, 1_000, false, Some(10), None).unwrap();
        assert!(page.items.is_empty());
        assert!(page.next_cursor.is_none());
        let json = serde_json::to_value(&page).unwrap();
        assert!(json["oldestAvailableAt"].is_null());
        assert!(json.get("nextCursor").is_none());
        assert_eq!(page.invalid_cost_rows, 0);
    }

    #[test]
    fn proxy_rows_collapse_and_jsonl_sessions_page_stably() {
        let db = Database::memory().unwrap();
        insert(&db, "p1", "proxy", Some("should-not-split"), "0.10", 500);
        insert(&db, "p2", "proxy", None, "0.25", 800);
        insert(&db, "s1", "codex_session", Some("alpha"), "1.00", 700);
        insert(&db, "s2", "session", Some("alpha"), "0.50", 900);
        insert(&db, "s3", "session_log", Some("beta"), "2", 600);
        insert(&db, "n1", "session", None, "0.05", 400);
        insert(&db, "bad", "session", Some("gamma"), "not-a-number", 300);

        let now = 900;
        let page = list_sessions_at(&db, now, false, Some(2), None).unwrap();
        assert_eq!(page.oldest_available_at, Some(300));
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.invalid_cost_rows, 1);
        assert_eq!(page.items[0].session_id.as_deref(), Some("alpha"));
        assert_eq!(page.items[0].group, SessionGroup::Session);
        assert_eq!(page.items[0].spent_usd, "1.5");
        assert_eq!(page.items[0].last_seen_at, 900);
        assert_eq!(page.items[0].state, SessionSpendState::Ok);
        assert!(page.items[0].cap_usd.is_none());
        assert!(page.next_cursor.is_some());
        let json = serde_json::to_value(&page.items[0]).unwrap();
        assert!(json["projectAlias"].is_null());
        assert!(json.get("clientTag").is_none());

        let next = list_sessions_at(&db, now, false, Some(2), page.next_cursor.as_deref()).unwrap();
        let again =
            list_sessions_at(&db, now, false, Some(2), page.next_cursor.as_deref()).unwrap();
        assert_eq!(next, again);
        assert!(next
            .items
            .iter()
            .all(|item| item.session_id.as_deref() != Some("alpha")));
        let ids = page
            .items
            .iter()
            .chain(next.items.iter())
            .map(|item| (item.group, item.session_id.clone()))
            .collect::<Vec<_>>();
        assert_eq!(ids.len(), 4);
        let rest =
            list_sessions_at(&db, now, false, Some(10), next.next_cursor.as_deref()).unwrap();
        let all = page
            .items
            .iter()
            .chain(next.items.iter())
            .chain(rest.items.iter())
            .collect::<Vec<_>>();
        assert_eq!(all.len(), 5);
        let proxy = all
            .iter()
            .find(|item| item.group == SessionGroup::ProxyTraffic)
            .unwrap();
        assert!(proxy.session_id.is_none());
        assert_eq!(proxy.spent_usd, "0.35");
        assert_eq!(proxy.last_seen_at, 800);
        let gamma = all
            .iter()
            .find(|item| item.session_id.as_deref() == Some("gamma"))
            .unwrap();
        assert_eq!(gamma.spent_usd, "0");
        let unattributed = all
            .iter()
            .find(|item| item.group == SessionGroup::Unattributed)
            .unwrap();
        assert!(unattributed.session_id.is_none());
        assert_eq!(unattributed.spent_usd, "0.05");
        let serialized = serde_json::to_value(unattributed).unwrap();
        assert_eq!(serialized["group"], "unattributed");
        assert!(!serde_json::to_value(gamma)
            .unwrap()
            .to_string()
            .contains("not-a-number"));

        let active_now = 300 + ACTIVE_WINDOW_SECS + 1;
        let active = list_sessions_at(&db, active_now, true, None, None).unwrap();
        assert!(active
            .items
            .iter()
            .all(|item| item.last_seen_at.saturating_add(ACTIVE_WINDOW_SECS) >= active_now));
        assert!(active
            .items
            .iter()
            .all(|item| item.session_id.as_deref() != Some("gamma")));
        assert_eq!(active.oldest_available_at, Some(300));
        assert!(list_sessions_at(&db, 900, false, Some(1), Some("@@@")).is_err());
    }
}
