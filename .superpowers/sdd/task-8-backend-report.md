# Task 8 Backend Acceptance Report

## Scope

- Kept one Rust integration test that exercises the real proxy request path through the existing public `ProxyService` lifecycle.
- Removed the test-only crate-root/public re-exports of `ProxyServer` and `ProxyConfig`; production API visibility is unchanged from the pre-Task-8 baseline.
- No frontend or `docs/` files were changed in this backend-only unit.

## Acceptance flow

1. Start an Axum mock upstream on `127.0.0.1:0`.
2. Create an in-memory SQLite database and seed one enabled metered usage Provider plus the static Claude route binding.
3. Configure and start the real proxy through `ProxyService` on `127.0.0.1:0` with usage logging enabled.
4. Send a real non-streaming Claude request whose upstream response includes exact tokens and explicit cost fields.
5. Send a second real request whose response includes tokens but no cost fields.
6. Poll the Provider/time-bounded event query until both asynchronous ingestion tasks complete, then query the product dashboard.
7. Seed a second database with legacy Claude current/failover Providers that both point at the reachable mock upstream, but deliberately create no v13 route binding.
8. Start that second real proxy through `ProxyService`; verify the request returns local HTTP 503 and does not increment the mock upstream hit counter.

## Assertions

- Explicit response is attributed to `metered-e2e` within the requested half-open time range.
- Exact input/output/cache tokens and explicit cost components are preserved.
- Explicit total cost is `0.42` with `cost_source=upstream`.
- The no-cost response is priced from seeded model pricing with `cost_source=estimated` and total `0.00012`.
- The event page contains exactly two events.
- Product totals are exact: 30 input, 6 output, 3 cache-read, 4 cache-creation, and total cost `0.42012`.
- The full metered Provider summary reports the same exact totals, `event_count=2`, complete cost-source counts, route metadata, credential presence, and no quota state.
- Route-less request returns `503`, a `proxy_error` body mentioning Claude, and zero additional upstream hits even though reachable legacy current/failover candidates exist.
- The event/dashboard range ends ten seconds in the future, exceeding the five-second ingestion poll allowance with margin.

## TDD evidence

- The strengthened assertions passed against the already-correct static-binding implementation, so this review unit required test/API hardening rather than a runtime routing fix.
- Mutation RED: after temporarily restoring the forbidden legacy-current fallback in `select_bound_route`, `cargo test --manifest-path src-tauri/Cargo.toml --test usage_dashboard_proxy_e2e -- --nocapture` failed at the new route-less assertion with `left: 200`, `right: 503`. The mutation was immediately reverted and is absent from the diff.
- GREEN: with route bindings restored as the only request-path source and the test using `ProxyService`, the same target passed 1/1.

## Isolation

- Both application databases use `Database::memory()`; the seeded legacy candidates are fixture-only rows in the second in-memory database.
- All listeners use operating-system-assigned port `0`.
- The test does not start Tauri, use the app data directory, or read/write `~/.cc-switch`.
- The route credential is a fixture-only value accepted by the local mock upstream.

## Concerns

- Usage persistence is intentionally asynchronous after the HTTP response, so the test uses a bounded five-second poll instead of a fixed sleep.
- The test accesses the inferred proxy config returned by `ProxyService::get_config`; it does not name or expose the private config type.
- The final Task 8 legacy-symbol `rg` still finds the retained compatibility-only `select_providers` implementation in `provider_router.rs` and one explanatory comment in `forwarder.rs`; removing or relocating those matches belongs to the separate legacy-main-path exit work, not this backend acceptance commit.

## Verification

- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` — PASS.
- `cargo test --manifest-path src-tauri/Cargo.toml --test usage_dashboard_proxy_e2e -- --nocapture` — PASS, 1 passed / 0 failed / 0 ignored.
