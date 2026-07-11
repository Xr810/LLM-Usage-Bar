# Usage Dashboard Acceptance Runbook

## Safety boundary

- Original CC Switch owns `~/.cc-switch/cc-switch.db` and may keep schema v11.
- LLM Usage Bar owns `~/.llm-usage-bar/cc-switch.db` and schema v13.
- Never point `app_config_dir` at `~/.cc-switch`, a symlink to it, or a case alias such as `~/.CC-SWITCH`.
- Do not run a development Tauri build with the real user home. Automated acceptance uses `Database::memory()` and operating-system-assigned port `0`.
- Any future import from original CC Switch must be an explicit read-only SQLite Backup snapshot. Copying a live database/WAL pair is not an accepted migration path.

## Automated real-proxy acceptance

Use the repository-pinned Rust toolchain and run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml \
  --test usage_dashboard_proxy_e2e -- --nocapture
```

The test creates this isolated fixture in memory:

```json
{
  "provider": {
    "id": "metered-e2e",
    "billingKind": "metered",
    "productGroupId": "claude-e2e",
    "tokenSources": ["proxy"],
    "routeAppType": "claude",
    "routeConfig": {
      "baseUrl": "http://127.0.0.1:<mock-port>",
      "apiKey": "fixture-only-key"
    },
    "enabled": true
  },
  "routeBinding": {
    "protocol": "claude",
    "providerId": "metered-e2e"
  },
  "proxy": {
    "listenAddress": "127.0.0.1",
    "listenPort": 0,
    "enableLogging": true
  }
}
```

Expected assertions:

1. A real request through `ProxyServer` reaches the Axum upstream exactly once and records the upstream response ID.
2. Explicit upstream Token and cost fields create an event with `costSource="upstream"` and exact decimal component/total costs.
3. A response without explicit cost uses seeded model pricing and creates `costSource="estimated"`.
4. Dashboard totals are queryable by the immutable Provider, product group and half-open time range.
5. A second proxy with no route binding returns HTTP 503 locally and does not increment the upstream hit counter.

The route-less response has this stable shape (the human-readable message may include protocol context):

```json
{
  "error": {
    "type": "proxy_error",
    "message": "...claude..."
  }
}
```

The recorded explicit event must include at least:

```text
providerId=metered-e2e
productGroupId=claude-e2e
source=proxy
upstreamCorrelationId=msg-e2e-0
inputTokens=10
outputTokens=2
cacheReadTokens=3
cacheCreationTokens=4
inputCostUsd=0.10
outputCostUsd=0.20
cacheReadCostUsd=0.03
cacheCreationCostUsd=0.09
totalCostUsd=0.42
costSource=upstream
```

## Read-only SQLite inspection

Run these only against an isolated LLM Usage Bar fixture/database. `immutable=1` is appropriate only for a database known not to be changing; otherwise use `mode=ro` and SQLite's normal WAL handling.

```bash
sqlite3 'file:/absolute/path/to/llm-usage-bar-fixture.db?mode=ro' \
  'PRAGMA query_only=ON; PRAGMA user_version;'
```

Expected schema version: `13`.

```sql
PRAGMA query_only = ON;

SELECT event_id, provider_id, product_group_id, source, occurred_at,
       input_tokens, output_tokens, total_cost_usd, cost_source
FROM usage_events
WHERE provider_id = 'metered-e2e'
  AND occurred_at >= :start_at
  AND occurred_at < :end_at
ORDER BY occurred_at, event_id;

SELECT canonical_event_id, duplicate_event_id, link_kind, link_value
FROM usage_event_links
ORDER BY created_at, duplicate_event_id;

SELECT snapshot_id, provider_id, fetched_at
FROM quota_snapshots
ORDER BY fetched_at;
```

## Accounting invariants

Quota is status, not usage:

```sql
SELECT COUNT(*) AS quota_rows_in_events
FROM usage_events AS event
JOIN quota_snapshots AS quota
  ON quota.snapshot_id = event.event_id;
```

Expected: `0`. Dashboard Token, request, cost and any future trend query must read `usage_events`; they must never union or join `quota_snapshots` into the numeric aggregate.

Linked duplicates are preserved in event detail but only `usage_event_links.duplicate_event_id` is excluded from aggregate queries. Events with no exact shared request/session/upstream correlation ID remain separate even when Provider, model, Token counts and timestamps are similar:

```sql
SELECT event_id, provider_id, model, occurred_at,
       input_tokens, output_tokens, request_id, session_id,
       upstream_correlation_id
FROM usage_events
WHERE provider_id = :provider_id
ORDER BY occurred_at, event_id;
```

Acceptance requires both similar no-ID rows to remain present and counted. Do not add time/Token/model fingerprint deduplication.

## Final gates

```bash
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
./node_modules/.bin/tsc --noEmit
./node_modules/.bin/vitest run
./node_modules/.bin/vite build
rg -n "get_effective_current_provider|get_failover_queue|select_providers" \
  src-tauri/src/proxy/provider_router.rs \
  src-tauri/src/proxy/handler_context.rs \
  src-tauri/src/proxy/forwarder.rs
git diff --check
```

The Rust, TypeScript, unit-test and renderer gates must pass. The request-path symbol scan must have no active request-path matches; compatibility-only helpers should be renamed or removed from these request-path files rather than waived. Legacy tables and source modules remain during the compatibility window.
