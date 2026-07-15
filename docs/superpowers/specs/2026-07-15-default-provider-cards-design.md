# Fixed Default Provider Cards Design

**Date:** 2026-07-15
**Status:** Approved

## Summary

LLM Usage Bar will always expose five fixed system Provider cards:

1. ChatGPT Plus/Pro
2. Claude Pro/Max
3. OpenAI API
4. Anthropic API
5. OpenRouter

Users do not create these Providers before using them. Subscription cards start
the appropriate official account login, while API cards already know their
canonical endpoint and require only an API Key. System cards cannot be deleted or
renamed, but their authentication, enabled state, and Agent bindings remain under
user control.

The default Agent bindings are:

| System Provider | Default Agents |
| --- | --- |
| ChatGPT Plus/Pro | Codex |
| Claude Pro/Max | Claude Code |
| OpenAI API | None |
| Anthropic API | None |
| OpenRouter | OpenCode, OpenClaw, Hermes |

For the three fixed API Providers, a user enters one upstream API Key at the
Provider card. Every Agent binding owns a distinct local proxy credential, so one
shared upstream account can still produce reliable Agent-level attribution.

## Context

The repository already has:

- a Provider catalog and per-application preset definitions;
- a managed Codex OAuth account flow;
- Agent-centric dashboard modules and many-to-many Agent–Provider bindings;
- protected credential storage for binding credentials; and
- local proxy routing that freezes Agent and Provider identity before recording
  usage.

The existing official Provider seeding is one-time and deletable. The existing
direct API binding model also uses the same secret as both the local selection
credential and the upstream API credential. Neither behavior matches this design:
the five cards must remain present, and a system API Provider must accept one
upstream Key while retaining distinct local credentials for each Agent.

## Goals

- Make the five most common Providers visible and immediately actionable on every
  installation and upgrade.
- Keep subscription authentication distinct from metered API authentication.
- Seed useful default Agent bindings without preventing later user changes.
- Allow one fixed API Provider credential to be shared by multiple Agents without
  losing Agent attribution.
- Keep every raw secret out of SQLite, logs, diagnostics, exports, backups, sync
  artifacts, and ordinary read DTOs.
- Preserve existing custom Providers and user-created bindings.
- Fail closed before upstream network access when authentication or routing cannot
  be proven.

## Non-goals

- Automatically changing an external Agent's configuration.
- Automatically switching which Provider an external Agent currently uses.
- Converting existing custom Providers into one of the five system cards.
- Importing legacy plaintext API Keys into protected storage without explicit user
  action.
- Reading, copying, storing, or proxying with Claude Free/Pro/Max OAuth tokens.
- Removing the existing Add Provider flow or custom Provider presets.
- Expanding this release to additional fixed Providers.

## Considered Approaches

### 1. Persistent system Provider records

Store the five cards as real `usage_providers` records with stable system identities.
Reconcile their presence and canonical non-secret metadata at startup. Reject user
attempts to delete or rename them while allowing authentication and bindings to
change.

This is the selected approach. It gives bindings stable foreign-key targets,
supports durable connection state, and lets endpoint corrections ship as catalog
updates.

### 2. Clone ordinary Providers on first run

This would reuse more of the existing Add Provider flow, but cloned records could
be deleted or duplicated and would drift from future endpoint corrections. It also
cannot guarantee that the five cards always exist.

### 3. Virtual cards materialized after authentication

This would avoid initial database rows, but connection state and default binding
creation would become conditional UI behavior. Bindings would not have stable
Provider identities before the first login, and migration behavior would be harder
to reason about.

## Canonical System Providers

The system catalog owns the following stable keys and immutable presentation
identity:

| System key | Display name | Billing | Authentication | Canonical endpoint |
| --- | --- | --- | --- | --- |
| `chatgpt-subscription` | ChatGPT Plus/Pro | Subscription | Managed Codex OAuth | Existing Codex OAuth backend |
| `claude-subscription` | Claude Pro/Max | Subscription | Official `claude auth` CLI | Managed entirely by Claude Code |
| `openai-api` | OpenAI API | Metered | Provider API Key | `https://api.openai.com/v1` |
| `anthropic-api` | Anthropic API | Metered | Provider API Key | `https://api.anthropic.com` |
| `openrouter-api` | OpenRouter | Metered | Provider API Key | `https://openrouter.ai/api/v1` |

Canonical endpoint fields on system cards are read-only in user mutation commands.
Users who need another endpoint create a custom Provider. Internal catalog
reconciliation may update canonical non-secret metadata in a later release, but it
must not touch credentials, connection state, enabled state, or Agent bindings.

## Persistence Model

### System identity

Add a nullable, unique system-preset identity to `usage_providers`. Ordinary rows
keep it null. The five seeded rows use stable IDs and non-null preset identities.
Provider list DTOs expose whether a Provider is a system preset and which
authentication UI it requires.

The normal save, delete, and generic edit commands reject attempts to:

- delete a system Provider;
- change its stable ID or preset key;
- change its display name, billing kind, authentication kind, or canonical route;
  or
- turn an ordinary Provider into a system Provider.

Enabled state, authentication state, notes that are explicitly user-owned, and
Agent bindings remain mutable.

### Provider-level API credentials

Introduce Provider credential metadata with a one-to-one relationship to a fixed
API Provider. SQLite stores only:

- the Provider ID;
- an opaque protected-store slot;
- a domain-separated credential fingerprint;
- a monotonically increasing credential version;
- lifecycle/reconciliation state; and
- timestamps.

The raw upstream API Key lives in the OS protected credential store. Set, replace,
and clear use the same staged-slot, compare-and-swap, and reconciliation principles
as the existing protected binding credential service. Replacing a Provider Key does
not rotate local Agent credentials or change binding identity.

### Binding-level local credentials

For fixed API Providers, the existing binding credential becomes a local proxy
credential only. Each binding receives an independently generated, high-entropy
secret. Its raw value lives in the protected store and its fingerprint remains in
SQLite. The local credential is never forwarded upstream.

Ordinary list and diagnostics commands expose only configured/missing/unavailable
status. A dedicated, user-initiated “copy local Key” command may read one binding
secret and return it to the requesting UI. That command must be excluded from
command tracing, must not implement bulk reads, and must never log its input or
output. Rotation creates a new local secret and immediately invalidates the old
one.

Existing custom Providers retain their current binding-owned credential behavior in
this release. The provider-level shared-secret model applies to the three fixed API
cards only, avoiding an unsafe reinterpretation of existing credentials.

### Managed subscription credentials

The ChatGPT card references the existing managed Codex OAuth account state. The
Claude card launches the official `claude auth login` command and verifies the
result through `claude auth status`. LLM Usage Bar does not implement a Claude.ai
OAuth client and does not read, copy, store, inject, refresh, or proxy with the
Claude OAuth token. The Claude system Provider projects only CLI authentication
status, subscription/quota state, and Claude Code session attribution.

## Seeding and Reconciliation

Startup performs two separate idempotent operations:

1. **System card reconciliation:** ensure all five Provider records exist and repair
   only canonical non-secret catalog metadata. This runs on every startup so a
   damaged or incomplete database cannot permanently lose a system card.
2. **Default binding seeding:** create the approved default bindings once per
   database. Record completion separately. A binding the user later removes is not
   recreated on every startup.

Default bindings start in requested-enabled state but are effective only when all
required credentials are available. Fixed API bindings also require a verified
local credential. Credential generation happens through the credential service,
not inside a SQL migration. Failure leaves the binding visible but unavailable and
retryable; it never enables a route without its credential.

The initial mappings are exactly:

- ChatGPT Plus/Pro to Codex;
- Claude Pro/Max to Claude Code; and
- OpenRouter to OpenCode, OpenClaw, and Hermes.

OpenAI API and Anthropic API have no initial Agent bindings. Saving their Key must
not silently create a binding.

## Provider Page Experience

The Provider page shows the five system cards first and custom Providers after
them. System cards remain visible in disconnected and disabled states.

Each card shows:

- Provider name and subscription/metered classification;
- connected, disconnected, expired, missing, or unavailable authentication state;
- enabled/effective state;
- the authentication action appropriate to that card; and
- an Agent multi-select showing requested bindings.

### Subscription cards

- ChatGPT Plus/Pro offers “Sign in with ChatGPT,” reconnect, and disconnect.
- Claude Pro/Max offers “Sign in with Claude,” reconnect, and disconnect by running
  the official `claude auth login`, `claude auth status`, and
  `claude auth logout` commands. The login command runs interactively in a visible
  terminal; status is parsed only from the CLI's documented JSON output.
- Successful authentication makes an otherwise valid requested binding effective.
- Disconnecting keeps binding choices but makes every dependent binding
  ineffective immediately.

### API cards

- Show the locked canonical endpoint.
- Provide API Key save, replace, clear, and connection-test actions.
- Never redisplay the upstream API Key.
- Clearing the upstream Key preserves bindings and local credentials but makes all
  dependent bindings ineffective.
- Restoring a valid upstream Key reactivates previously requested-enabled bindings
  without recreating them.

### Agent bindings

Users may add, remove, enable, or disable any compatible Agent binding. Adding a
fixed API binding generates its local proxy credential. The UI shows the Agent's
local endpoint and provides an explicit copy/rotate action for the local Key.

Fixed API bindings configure LLM Usage Bar routing and attribution. The ChatGPT
binding uses the existing managed Codex path. The Claude Pro/Max binding is
observation-only: it associates official Claude Code subscription status and
session usage with the Claude Code Agent and is never accepted as a local proxy
route. The app presents the values an external Agent needs but does not silently
edit that Agent's live configuration.

## Proxy Data Flow

For a fixed API Provider request:

1. The Agent sends a request to its local proxy endpoint using its binding-local
   credential.
2. The proxy removes recognized inbound credential fields and hashes the local
   credential with the binding domain separator.
3. One atomic lookup resolves exactly one requested-enabled binding and verifies
   that the Agent and Provider are active and the Agent is not archived.
4. The credential service loads the stored local credential and verifies it in
   constant time against the inbound value.
5. The service loads and verifies the fixed Provider's upstream credential.
6. Request context freezes Agent ID, Provider ID, binding ID, and credential
   versions.
7. The proxy constructs the canonical upstream request and injects only the
   Provider credential in the protocol-appropriate location.
8. Usage ingestion records the frozen Agent and Provider identities and never
   re-resolves the current binding after the request starts.

The local credential must not survive in the upstream URL, headers, body, logs, or
error messages.

ChatGPT subscription requests continue to use the existing managed Codex path and
the same frozen Agent–Provider ownership rules. Claude Pro/Max is not a proxy
request path: any attempt to resolve its system Provider as an upstream proxy route
is rejected locally. Claude Code continues to communicate through Anthropic's own
official client and credential handling, while LLM Usage Bar observes only status,
quota, and trusted Claude Code session data.

## Failure Behavior

Unknown, missing, conflicting, disabled, rotated, store-missing, or malformed local
credentials produce one generic local authorization error before any upstream
network request. Missing or expired upstream Provider authentication also fails
closed and never falls back to another Provider.

Saving or replacing a Provider Key is not considered successful until protected
storage, database compare-and-swap, and verification all succeed. A partial failure
must preserve the previous usable generation or leave the Provider disconnected;
it must not expose a mixed state.

Connection tests use the same canonical route and protected credential resolution
as real traffic. Test failures update status without deleting a previously stored
Key or changing Agent bindings.

## Upgrade Policy

The schema migration is additive. It creates the system identity and
Provider-credential metadata needed by this design, then inserts the five canonical
Provider rows and default binding rows without deleting existing data.

Upgrade rules are conservative:

- existing custom and legacy Providers remain untouched;
- an existing managed ChatGPT/Codex OAuth account may be referenced by the new
  ChatGPT card because no secret is copied;
- existing Claude Code login state may be verified by the new Claude card through
  `claude auth status`, without reading its credential files or Keychain entries;
- legacy plaintext API Keys are not copied into the fixed cards;
- existing binding secrets are not reinterpreted as shared Provider credentials;
  and
- naming or endpoint similarity alone never causes a custom Provider to be merged
  into a system card.

If a fixed API card has no protected Provider credential after upgrade, it appears
disconnected and asks the user to save a Key. This is preferable to silently
adopting an ambiguous or exportable legacy secret.

## Security Invariants

- Raw upstream and local credentials never enter SQLite, logs, diagnostics,
  exports, backups, sync artifacts, analytics, or ordinary read DTOs.
- System card reconciliation cannot overwrite authentication or binding state.
- One local credential resolves to at most one active binding.
- Local credentials are never forwarded upstream.
- Provider credentials are never accepted as local binding credentials.
- Claude OAuth tokens are never read, copied, stored, returned, refreshed, injected,
  or used for proxy routing by LLM Usage Bar.
- The Claude Pro/Max system Provider is rejected by every local proxy route.
- Provider disconnect or disable invalidates every dependent route immediately.
- Binding edits affect future requests only and cannot reattribute history.
- Proxy resolution completes before network I/O and holds no database or
  credential-store lock during that I/O.
- There is no automatic Provider fallback after an authorization failure.

## Verification and Acceptance Criteria

### Persistence and migration

- A new database contains exactly five system Provider rows with the canonical
  immutable identities.
- The initial binding set is exactly ChatGPT–Codex, Claude–Claude Code, and
  OpenRouter–OpenCode/OpenClaw/Hermes.
- OpenAI API and Anthropic API start unbound.
- Repeated startup neither duplicates cards/bindings nor overwrites user state.
- Removing or disabling a default binding survives restart.
- Attempts to delete, rename, reclassify, or reroute a system card are rejected.
- Upgrade preserves all existing Provider, binding, event, link, and quota rows.

### Authentication and credential lifecycle

- ChatGPT login, expiry, reconnect, and disconnect states project onto its card and
  dependent binding.
- Claude login, status, reconnect, and logout use only the documented
  `claude auth` commands; tests inject a command runner and prove no credential
  file, Keychain item, or OAuth token is read.
- Provider API Key set, replace, clear, reconciliation, and connection test are
  covered with injected protected stores.
- Local binding Key generation, explicit copy, rotation, and invalidation are
  covered without exposing secrets through list APIs.
- Provider credential rotation leaves local binding credentials stable.
- Clearing a Provider credential preserves binding choices but produces zero
  effective routes until authentication is restored.

### Proxy and attribution

- One OpenRouter upstream Key can serve OpenCode, OpenClaw, and Hermes through
  three distinct local credentials.
- Each request is attributed to the correct frozen Agent and the shared OpenRouter
  Provider.
- Requests with missing, unknown, duplicated, stale, or protocol-mismatched local
  credentials produce zero upstream calls.
- Requests that select the Claude Pro/Max system Provider produce zero upstream
  calls and a local unsupported-route error.
- Upstream requests contain the Provider credential and no local credential.
- Concurrent credential rotation and binding disable tests prove the existing
  request boundary is linearizable and future requests fail closed.

### UI

- All five cards render before any user-created Provider.
- Subscription cards expose login actions; API cards expose Key actions and locked
  endpoints.
- Agent defaults render exactly as approved and remain user-editable.
- Disconnected, connected, expired, missing, disabled, and unavailable states are
  distinguishable.
- Restart persistence is covered for authentication projection, Provider enabled
  state, and modified bindings.

### Secret-leak regression checks

Tests inject unique sentinel secrets and assert they are absent from command
responses other than the explicit single-binding copy result, error strings, logs,
database dumps, SQL exports, backups, sync payloads, and serialized diagnostics.

## Implementation Boundary

This design is one feature with four implementation areas:

1. additive schema and system Provider reconciliation;
2. Provider-level protected API credential lifecycle;
3. binding-local credential resolution and proxy credential substitution; and
4. fixed Provider card UI, authentication actions, and editable default bindings.

The implementation plan must preserve existing custom Provider behavior and must
use focused migration, credential, proxy, and UI tests before broad repository
verification.
