# Provider-Only Usage Monitoring Design

**Date:** 2026-07-17
**Status:** Approved
**Supersedes:** Agent-centric usage navigation, Agent–Provider attribution, and
LLM Usage Bar-managed Provider switching.

## Product Boundary

LLM Usage Bar is a Provider/account monitoring application. It shows usage,
cost, remaining quota or credit, reset time, refresh health, and budget status
directly for each configured Provider account.

LLM Usage Bar does not:

- display usage by Agent;
- maintain Agent–Provider bindings for monitoring;
- infer which Provider an Agent is currently using;
- switch an Agent's live Provider or model;
- compete with CC Switch for live configuration, proxy ownership, or its
  `~/.cc-switch` database.

CC Switch owns Provider switching. LLM Usage Bar remains read-only with respect
to CC Switch and external Agent configuration.

## Provider Account Identity

The primary display and accounting identity is one Provider account, not a
vendor name and not an Agent. Two accounts from the same vendor are separate
cards with separate credentials, quota snapshots, usage totals, budgets, and
refresh states. They must not be merged merely because they share a Provider
type.

The existing `usage_providers.id` remains the stable history key. Existing
Agent-tagged events are aggregated by `provider_id`; their Agent metadata may
remain in storage for compatibility but is not part of the product projection.
Historical events and quota snapshots are never rewritten to match current
Provider settings.

## Monitoring Sources

Each Provider card reports its evidence source and freshness:

1. Subscription quota collector: remaining window, reset time, and supported
   manual-reset counters.
2. Provider usage or billing API: tokens, spend, credit, and billing-period
   values that the Provider actually exposes.
3. Existing trusted Provider-scoped usage events: immutable request evidence
   retained from prior collection.
4. Unsupported: explicitly show that usage or remaining balance is unavailable.

A missing upstream field stays unknown. Zero is shown only when the source
explicitly reports zero. Quota snapshots are not converted into spend events,
and estimated cost is labelled separately from upstream-reported cost.

A future CC Switch import, if added, must be an explicit read-only SQLite Backup
snapshot with deterministic Provider-account mapping. LLM Usage Bar must never
open the live CC Switch database for writes or silently treat CC Switch logs as
its own authoritative database.

## User Experience

The main window and menu-bar popover are Provider-first:

- one card or row per Provider account;
- subscription and metered labels describe billing semantics, not navigation;
- each card can show used amount, remaining amount, reset/period end, tokens,
  cost, daily budget progress, last refresh, and source/error state;
- filtering may use Provider, billing type, or status, but never Agent;
- settings contain Provider credentials, source configuration, refresh interval,
  enabled/visible state, and budget only;
- no Agent tabs, Agent module editor, Agent binding controls, switch controller,
  route takeover, or model switching action appears in the monitoring UI.

## Compatibility and Removal Policy

Physical Agent tables and nullable Agent columns already present in schema v18
are retained temporarily so this scope change does not require a destructive
database migration. No new Agent switching schema is added. Dead Agent runtime,
commands, and UI may be removed in small verified changes after Provider-only
queries and screens no longer depend on them.

The built-in Provider catalog remains useful, but default Agent bindings and
Agent-specific credentials are obsolete. The original five entries stay enabled
for compatibility; additional common API Providers are built in but disabled
until the user opts in and configures that account. A built-in card represents
one monitorable Provider account; additional accounts require distinct records
rather than Agent bindings.

Custom Providers are a collection, not a singleton preset. The custom-Provider
creation entry remains visible after every save, each creation produces a
separate Provider record, and users can delete records they created. Built-in
Provider records remain protected from deletion.

## Acceptance Criteria

1. Main window and tray show Provider accounts without Agent navigation.
2. Provider totals are identical regardless of any stored Agent metadata.
3. Multiple accounts of one vendor stay separate.
4. Supported quota/credit values include source, freshness, and reset/period end.
5. Unsupported remaining balance is visibly unavailable rather than zero.
6. Refreshing or editing monitoring settings performs no Agent configuration or
   CC Switch write.
7. Existing history remains queryable by Provider after Agent UI removal.
8. The v19 Agent switching migration and its switching tables are absent.
