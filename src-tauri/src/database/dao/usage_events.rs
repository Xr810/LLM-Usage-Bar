use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::usage::domain::{CostSource, TokenSource, UsageEvent, UsageEventLink, UsageEventPage};
use rusqlite::{params, types::Type, OptionalExtension, Row};

fn token_source_value(source: TokenSource) -> &'static str {
    match source {
        TokenSource::Proxy => "proxy",
        TokenSource::SessionLog => "session_log",
    }
}

fn cost_source_value(source: CostSource) -> &'static str {
    match source {
        CostSource::Upstream => "upstream",
        CostSource::Estimated => "estimated",
        CostSource::Unavailable => "unavailable",
    }
}

fn enum_from_text<T: serde::de::DeserializeOwned>(
    value: String,
    column: usize,
) -> rusqlite::Result<T> {
    serde_json::from_str(&format!("\"{value}\"")).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(error))
    })
}

fn nonnegative_u64(row: &Row<'_>, column: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(column)?;
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(column, Type::Integer, Box::new(error))
    })
}

fn usage_event_from_row(row: &Row<'_>) -> rusqlite::Result<UsageEvent> {
    Ok(UsageEvent {
        event_id: row.get(0)?,
        source: enum_from_text(row.get(1)?, 1)?,
        provider_id: row.get(2)?,
        product_group_id: row.get(3)?,
        occurred_at: row.get(4)?,
        model: row.get(5)?,
        input_tokens: nonnegative_u64(row, 6)?,
        output_tokens: nonnegative_u64(row, 7)?,
        cache_read_tokens: nonnegative_u64(row, 8)?,
        cache_creation_tokens: nonnegative_u64(row, 9)?,
        request_id: row.get(10)?,
        session_id: row.get(11)?,
        upstream_correlation_id: row.get(12)?,
        input_cost_usd: row.get(13)?,
        output_cost_usd: row.get(14)?,
        cache_read_cost_usd: row.get(15)?,
        cache_creation_cost_usd: row.get(16)?,
        total_cost_usd: row.get(17)?,
        cost_source: enum_from_text(row.get(18)?, 18)?,
        legacy_request_id: row.get(19)?,
        created_at: row.get(20)?,
    })
}

const EVENT_COLUMNS: &str = "event_id, source, provider_id, product_group_id, occurred_at,
    model, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
    request_id, session_id, upstream_correlation_id, input_cost_usd, output_cost_usd,
    cache_read_cost_usd, cache_creation_cost_usd, total_cost_usd, cost_source,
    legacy_request_id, created_at";

fn token_count(value: u64) -> Result<i64, AppError> {
    i64::try_from(value).map_err(|_| AppError::Message("token count is too large".to_string()))
}

impl Database {
    pub fn insert_usage_event(&self, event: &UsageEvent) -> Result<bool, AppError> {
        let conn = lock_conn!(self.conn);
        let inserted = conn.execute(
            "INSERT INTO usage_events (
                event_id, source, provider_id, product_group_id, occurred_at, model,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                request_id, session_id, upstream_correlation_id, input_cost_usd,
                output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd,
                total_cost_usd, cost_source, legacy_request_id, created_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21
             ) ON CONFLICT(event_id) DO NOTHING",
            params![
                event.event_id,
                token_source_value(event.source),
                event.provider_id,
                event.product_group_id,
                event.occurred_at,
                event.model,
                token_count(event.input_tokens)?,
                token_count(event.output_tokens)?,
                token_count(event.cache_read_tokens)?,
                token_count(event.cache_creation_tokens)?,
                event.request_id,
                event.session_id,
                event.upstream_correlation_id,
                event.input_cost_usd,
                event.output_cost_usd,
                event.cache_read_cost_usd,
                event.cache_creation_cost_usd,
                event.total_cost_usd,
                cost_source_value(event.cost_source),
                event.legacy_request_id,
                event.created_at,
            ],
        )?;
        Ok(inserted == 1)
    }

    pub fn find_matching_usage_event(
        &self,
        provider_id: &str,
        event: &UsageEvent,
    ) -> Result<Option<UsageEvent>, AppError> {
        if provider_id != event.provider_id {
            return Err(AppError::Message(
                "provider_id must match event.provider_id".to_string(),
            ));
        }
        let conn = lock_conn!(self.conn);
        for (column, value) in [
            ("request_id", event.request_id.as_deref()),
            ("session_id", event.session_id.as_deref()),
            (
                "upstream_correlation_id",
                event.upstream_correlation_id.as_deref(),
            ),
        ] {
            let Some(value) = value.filter(|value| !value.is_empty()) else {
                continue;
            };
            let sql = format!(
                "SELECT {EVENT_COLUMNS} FROM usage_events
                 WHERE provider_id = ?1 AND {column} = ?2
                 ORDER BY occurred_at DESC, event_id DESC LIMIT 1"
            );
            if let Some(matched) = conn
                .query_row(&sql, params![provider_id, value], usage_event_from_row)
                .optional()?
            {
                return Ok(Some(matched));
            }
        }
        Ok(None)
    }

    pub fn insert_usage_event_link(&self, link: &UsageEventLink) -> Result<bool, AppError> {
        let conn = lock_conn!(self.conn);
        let canonical = conn
            .query_row(
                "SELECT source, provider_id FROM usage_events WHERE event_id = ?1",
                [&link.canonical_event_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let duplicate = conn
            .query_row(
                "SELECT source, provider_id FROM usage_events WHERE event_id = ?1",
                [&link.duplicate_event_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let (
            Some((canonical_source, canonical_provider)),
            Some((duplicate_source, duplicate_provider)),
        ) = (canonical, duplicate)
        else {
            return Err(AppError::Message("usage event not found".to_string()));
        };
        if canonical_provider != duplicate_provider {
            return Err(AppError::Message(
                "usage event links require same provider".to_string(),
            ));
        }
        if canonical_source == duplicate_source {
            return Err(AppError::Message(
                "usage event links require different sources".to_string(),
            ));
        }

        let inserted = conn.execute(
            "INSERT INTO usage_event_links (
                canonical_event_id, duplicate_event_id, link_kind, link_value, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(canonical_event_id, duplicate_event_id, link_kind, link_value)
             DO NOTHING",
            params![
                link.canonical_event_id,
                link.duplicate_event_id,
                link.link_kind,
                link.link_value,
                link.created_at,
            ],
        )?;
        Ok(inserted == 1)
    }

    pub fn list_usage_events(
        &self,
        provider_id: &str,
        start_at: i64,
        end_at: i64,
        page: u32,
        page_size: u32,
    ) -> Result<UsageEventPage, AppError> {
        if !(1..=200).contains(&page_size) {
            return Err(AppError::Message(
                "page size must be between 1 and 200".to_string(),
            ));
        }
        if page == 0 {
            return Err(AppError::Message("page must be at least 1".to_string()));
        }
        let offset = u64::from(page - 1)
            .checked_mul(u64::from(page_size))
            .and_then(|value| i64::try_from(value).ok())
            .ok_or_else(|| AppError::Message("pagination offset is too large".to_string()))?;

        let conn = lock_conn!(self.conn);
        let total = conn.query_row(
            "SELECT COUNT(*) FROM usage_events
             WHERE provider_id = ?1 AND occurred_at >= ?2 AND occurred_at < ?3",
            params![provider_id, start_at, end_at],
            |row| row.get::<_, u64>(0),
        )?;
        let mut statement = conn.prepare(&format!(
            "SELECT {EVENT_COLUMNS} FROM usage_events
             WHERE provider_id = ?1 AND occurred_at >= ?2 AND occurred_at < ?3
             ORDER BY occurred_at DESC, event_id DESC LIMIT ?4 OFFSET ?5"
        ))?;
        let items = statement
            .query_map(
                params![provider_id, start_at, end_at, page_size, offset],
                usage_event_from_row,
            )?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(UsageEventPage {
            items,
            total,
            page,
            page_size,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::database::Database;
    use crate::usage::domain::{
        BillingKind, CostSource, TokenSource, UsageEvent, UsageEventLink, UsageProviderInput,
    };

    fn save_provider(db: &Database, id: &str) {
        db.save_usage_provider(&UsageProviderInput {
            id: id.to_string(),
            name: id.to_string(),
            billing_kind: BillingKind::Metered,
            product_group_id: "claude".to_string(),
            token_sources: vec![TokenSource::Proxy, TokenSource::SessionLog],
            session_source_bindings: None,
            quota_source: None,
            quota_interval_seconds: None,
            route_app_type: None,
            route_config: None,
            quota_config: None,
            dashboard_module_id: None,
            enabled: true,
        })
        .unwrap();
    }

    fn event(id: &str, provider_id: &str, source: TokenSource, occurred_at: i64) -> UsageEvent {
        UsageEvent {
            event_id: id.to_string(),
            source,
            provider_id: provider_id.to_string(),
            product_group_id: "claude".to_string(),
            occurred_at,
            model: "claude-sonnet".to_string(),
            input_tokens: 10,
            output_tokens: 20,
            cache_read_tokens: 3,
            cache_creation_tokens: 4,
            request_id: None,
            session_id: None,
            upstream_correlation_id: None,
            input_cost_usd: Some("0.01".to_string()),
            output_cost_usd: Some("0.02".to_string()),
            cache_read_cost_usd: None,
            cache_creation_cost_usd: None,
            total_cost_usd: Some("0.03".to_string()),
            cost_source: CostSource::Upstream,
            legacy_request_id: None,
            created_at: occurred_at + 1,
        }
    }

    #[test]
    fn duplicate_event_id_does_not_overwrite_the_original() {
        let db = Database::memory().unwrap();
        save_provider(&db, "metered");
        let original = event("evt", "metered", TokenSource::Proxy, 100);
        assert!(db.insert_usage_event(&original).unwrap());

        let mut changed_duplicate = original.clone();
        changed_duplicate.model = "changed".to_string();
        changed_duplicate.input_tokens = 999;
        assert!(!db.insert_usage_event(&changed_duplicate).unwrap());

        let page = db.list_usage_events("metered", 0, 200, 1, 10).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items, vec![original]);
    }

    #[test]
    fn stable_match_requires_same_provider_and_an_exact_non_empty_identifier() {
        let db = Database::memory().unwrap();
        save_provider(&db, "one");
        save_provider(&db, "two");
        let mut stored = event("stored", "one", TokenSource::Proxy, 100);
        stored.request_id = Some("request-exact".to_string());
        stored.session_id = Some("session-exact".to_string());
        db.insert_usage_event(&stored).unwrap();

        let mut exact = event("probe", "one", TokenSource::SessionLog, 101);
        exact.request_id = Some("request-exact".to_string());
        assert_eq!(
            db.find_matching_usage_event("one", &exact)
                .unwrap()
                .unwrap()
                .event_id,
            "stored"
        );

        exact.request_id = Some("request".to_string());
        assert!(db
            .find_matching_usage_event("one", &exact)
            .unwrap()
            .is_none());
        let mut other_provider = exact.clone();
        other_provider.provider_id = "two".to_string();
        other_provider.request_id = Some("request-exact".to_string());
        assert!(db
            .find_matching_usage_event("two", &other_provider)
            .unwrap()
            .is_none());

        exact.request_id = Some(String::new());
        exact.session_id = Some(String::new());
        exact.upstream_correlation_id = None;
        assert!(db
            .find_matching_usage_event("one", &exact)
            .unwrap()
            .is_none());
    }

    #[test]
    fn matching_rejects_provider_parameter_mismatch() {
        let db = Database::memory().unwrap();
        save_provider(&db, "one");
        save_provider(&db, "two");
        let mut stored = event("stored", "one", TokenSource::Proxy, 100);
        stored.request_id = Some("request-exact".to_string());
        db.insert_usage_event(&stored).unwrap();

        let mut probe = event("probe", "two", TokenSource::SessionLog, 101);
        probe.request_id = Some("request-exact".to_string());
        assert_eq!(
            db.find_matching_usage_event("one", &probe)
                .unwrap_err()
                .to_string(),
            "provider_id must match event.provider_id"
        );
    }

    #[test]
    fn event_links_only_connect_events_from_different_sources_and_are_idempotent() {
        let db = Database::memory().unwrap();
        save_provider(&db, "metered");
        let proxy = event("proxy", "metered", TokenSource::Proxy, 100);
        let session = event("session", "metered", TokenSource::SessionLog, 101);
        let second_proxy = event("proxy-2", "metered", TokenSource::Proxy, 102);
        db.insert_usage_event(&proxy).unwrap();
        db.insert_usage_event(&session).unwrap();
        db.insert_usage_event(&second_proxy).unwrap();

        let link = UsageEventLink {
            canonical_event_id: "proxy".to_string(),
            duplicate_event_id: "session".to_string(),
            link_kind: "request_id".to_string(),
            link_value: "shared".to_string(),
            created_at: 103,
        };
        assert!(db.insert_usage_event_link(&link).unwrap());
        assert!(!db.insert_usage_event_link(&link).unwrap());

        let same_source = UsageEventLink {
            duplicate_event_id: "proxy-2".to_string(),
            ..link
        };
        assert_eq!(
            db.insert_usage_event_link(&same_source)
                .unwrap_err()
                .to_string(),
            "usage event links require different sources"
        );
    }

    #[test]
    fn event_links_reject_events_from_different_providers() {
        let db = Database::memory().unwrap();
        save_provider(&db, "one");
        save_provider(&db, "two");
        db.insert_usage_event(&event("proxy", "one", TokenSource::Proxy, 100))
            .unwrap();
        db.insert_usage_event(&event("session", "two", TokenSource::SessionLog, 101))
            .unwrap();

        let link = UsageEventLink {
            canonical_event_id: "proxy".to_string(),
            duplicate_event_id: "session".to_string(),
            link_kind: "request_id".to_string(),
            link_value: "shared".to_string(),
            created_at: 102,
        };
        assert_eq!(
            db.insert_usage_event_link(&link).unwrap_err().to_string(),
            "usage event links require same provider"
        );
    }

    #[test]
    fn pagination_is_deterministic_half_open_and_enforces_limits() {
        let db = Database::memory().unwrap();
        save_provider(&db, "metered");
        for (id, occurred_at) in [("before", 9), ("a", 10), ("b", 10), ("end", 20)] {
            db.insert_usage_event(&event(id, "metered", TokenSource::Proxy, occurred_at))
                .unwrap();
        }

        let first = db.list_usage_events("metered", 10, 20, 1, 1).unwrap();
        assert_eq!(first.total, 2);
        assert_eq!(first.items[0].event_id, "b");
        assert_eq!(first.page, 1);
        assert_eq!(first.page_size, 1);
        let second = db.list_usage_events("metered", 10, 20, 2, 1).unwrap();
        assert_eq!(second.items[0].event_id, "a");

        for invalid_size in [0, 201] {
            assert_eq!(
                db.list_usage_events("metered", 10, 20, 1, invalid_size)
                    .unwrap_err()
                    .to_string(),
                "page size must be between 1 and 200"
            );
        }
    }
}
