# Task 8 Backend Acceptance Report

## Scope

- Added one Rust integration test that exercises the real `ProxyServer` request path.
- Added only the minimal public re-exports required by an external integration-test crate: `ProxyServer` and `ProxyConfig`.
- No frontend or `docs/` files were changed in this backend-only unit.

## Acceptance flow

1. Start an Axum mock upstream on `127.0.0.1:0`.
2. Create an in-memory SQLite database and seed one enabled metered usage Provider plus the static Claude route binding.
3. Start the real proxy server on `127.0.0.1:0` with usage logging enabled.
4. Send a real non-streaming Claude request whose upstream response includes exact tokens and explicit cost fields.
5. Send a second real request whose response includes tokens but no cost fields.
6. Poll the Provider/time-bounded event query until both asynchronous ingestion tasks complete, then query the product dashboard.
7. Start a second real proxy with a separate in-memory database and no route binding; verify its request returns local HTTP 503 and does not increment the mock upstream hit counter.

## Assertions

- Explicit response is attributed to `metered-e2e` within the requested half-open time range.
- Exact input/output/cache tokens and explicit cost components are preserved.
- Explicit total cost is `0.42` with `cost_source=upstream`.
- The no-cost response is priced from seeded model pricing with `cost_source=estimated` and total `0.00012`.
- Dashboard totals include both events and report one upstream plus one estimated event.
- Route-less request returns `503`, a `proxy_error` body mentioning Claude, and zero additional upstream hits.

## TDD evidence

- RED: `cargo test --manifest-path src-tauri/Cargo.toml --test usage_dashboard_proxy_e2e -- --nocapture` failed with unresolved root imports for `ProxyConfig` and `ProxyServer`.
- GREEN: after the two minimal re-exports, the same command passed 1/1.

## Isolation

- Both application databases use `Database::memory()`.
- All listeners use operating-system-assigned port `0`.
- The test does not start Tauri, use the app data directory, or read/write `~/.cc-switch`.
- The route credential is a fixture-only value accepted by the local mock upstream.

## Concerns

- Usage persistence is intentionally asynchronous after the HTTP response, so the test uses a bounded five-second poll instead of a fixed sleep.
- `ProxyServer` and `ProxyConfig` are now public library re-exports solely to allow black-box integration testing; no runtime behavior changed.
- The final Task 8 legacy-symbol `rg` still finds the retained compatibility-only `select_providers` implementation in `provider_router.rs` and one explanatory comment in `forwarder.rs`; removing or relocating those matches belongs to the separate legacy-main-path exit work, not this backend acceptance commit.
