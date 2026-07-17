# Agent Provider Switching and Custom Agent Adapter Design

> **Superseded 2026-07-17:** This design is retained only as history. The approved
> replacement is
> [`2026-07-17-provider-only-monitoring-design.md`](2026-07-17-provider-only-monitoring-design.md).

**Date:** 2026-07-16
**Status:** Approved

## Summary

LLM Usage Bar will add explicit, CC Switch-style Provider switching to the
Agent-centric product. An Agent may have several available Provider bindings,
but a managed Agent has at most one current binding. Choosing another binding
from the application performs a real configuration change: the backend writes
the Agent's live configuration, verifies the result, and only then records the
new current binding.

This first release includes both the five built-in Agents and Custom Agents.
Custom Agent support is not limited to naming, usage attribution, or binding
management. A Custom Agent can define a safe declarative configuration adapter,
bind compatible API Providers, switch between them, send requests through the
selected route, detect external configuration drift, and recover from failed or
interrupted writes.

Provider availability and current selection remain separate concepts:

- several bindings may be available to an Agent at the same time; and
- exactly one binding is current after a managed Agent has completed a
  successful switch or an exact existing configuration has been adopted.

Selecting an Agent tab, changing dashboard navigation, enabling a binding, or
reordering Agents never changes live configuration. Switching always requires a
specific user action.

## Context

The repository currently contains two Provider domains:

1. The Agent-centric usage domain stores `usage_providers` and many-to-many
   `agent_provider_bindings`. Its `enabled` and `effective_enabled` states mean
   that a route is requested and usable; they do not identify one current
   Provider or write an external Agent's configuration.
2. The legacy CC Switch domain stores app-scoped `providers`, tracks
   `is_current` for some applications, and contains live writers for Claude,
   Codex, OpenCode, OpenClaw, and Hermes. Its Provider IDs and switching
   semantics are not interchangeable with usage Provider or binding IDs.

The Agent-centric proxy already gives fixed API bindings separate credentials:
an Agent receives a binding-local `lub_*` key, while the upstream API key stays
in protected storage and is injected only by the proxy. Requests freeze Agent,
Provider, account instance, account claim, binding, and credential identity
before upstream I/O, which preserves usage attribution.

The current UI allows users to create and edit Custom Agents, manage Provider
bindings, enable or disable routes, configure credentials, and view usage. It
does not expose a current Provider or a real switch action. The old Provider
components still contain useful interaction patterns, but they use legacy
`Provider[] + AppId` identities and cannot be mounted directly into the new UI.

Earlier specifications deliberately excluded automatic modification of Agent
configuration. This specification supersedes that exclusion only for explicit,
user-initiated switching and its required reconciliation and recovery actions.
It does not turn dashboard navigation or background monitoring into a switch
trigger. All other credential, attribution, and fail-closed guarantees remain in
force.

## Goals

- Let a user switch the real Provider used by a built-in Agent from inside LLM
  Usage Bar.
- Make Custom Agents fully switchable in the first release through declarative
  configuration adapters.
- Preserve the distinction between multiple available bindings and one current
  binding.
- For an API binding, apply the Provider base URL, binding-local key, and binding
  default model as one verified configuration transaction.
- Keep upstream credentials out of Agent configuration, ordinary DTOs, logs,
  diagnostics, SQLite, exports, and recovery metadata.
- Detect external changes instead of silently overwriting them.
- Provide deterministic rollback and startup recovery for interrupted
  multi-file switches.
- Preserve existing Provider, Profile, proxy, usage, dashboard, settings, and
  menu-bar behavior.
- Preserve immutable historical Agent and Provider attribution when current
  configuration changes.
- Keep legacy switching usable under explicit linkage, ownership, locking, and
  backfill rules.

## Non-goals

- Switching when a user selects an Agent tab or dashboard module.
- Automatic Provider failover, load balancing, or background optimization.
- Running user-provided shell commands, scripts, plugins, or arbitrary template
  expressions as a Custom Agent adapter.
- Guessing undocumented configuration fields or modifying an unvalidated file.
- Copying, exporting, or proxying with subscription OAuth tokens that the
  official client owns.
- Making every subscription Provider compatible with every Custom Agent.
- Automatically merging legacy Provider identities into usage Providers.
- Automatically upgrading ambiguous legacy combined credentials.
- Editing configuration on a remote machine in this release.
- Adding mutation controls to the left-click usage popover.
- Rewriting historical usage events after a switch.

## Considered Approaches

### 1. Binding-centric switch coordinator with Agent adapters

Add one switch coordinator whose input is an Agent-Provider binding. It resolves
the binding into a safe configuration projection and delegates file-specific
work to a built-in or declarative Agent adapter.

This is the selected approach. It preserves the Agent-centric identity model,
uses the binding-local proxy credential for attribution, gives built-in and
Custom Agents the same transaction and status model, and avoids treating legacy
Provider IDs as current truth.

### 2. Make the legacy CC Switch Provider model authoritative

The application could create or link one legacy Provider for every usage
binding, then call the existing `switch_provider` path. This would reuse more
existing writers, but it would duplicate identity, make fixed system Providers
awkward, inherit inconsistent exclusive versus additive semantics, and create a
long-lived synchronization problem between two Provider databases.

### 3. Keep Agent configuration permanently pointed at one proxy route

Every Agent could receive one stable proxy URL and key while the application
changes an internal active route. This gives fast hot-switching but does not
perform the true configuration switch requested by the user. It also weakens
binding-local credential identity and would make the live Agent configuration
unable to prove which Provider it selects.

## Product Invariants

### Available is not current

The following states remain independent:

- Agent visibility controls whether the Agent appears in the product.
- Provider enabled state controls whether the Provider may be used globally.
- Binding existence means the Provider is bound to the Agent.
- Binding enabled state means “available to this Agent.”
- Binding effective state means all compatibility and credential requirements
  currently pass.
- Binding switchability is a separate state: `ready`,
  `confirmation_required`, or `blocked(reason)`. It additionally accounts for a
  valid adapter, explicit model/mapping, live ownership, recovery, target safety,
  and required protected preflight.
- Current binding means the Agent's live configuration was verified to select
  that binding.

The implementation must not rename or reinterpret an existing `enabled` field as
current state.

### One current binding

An Agent may temporarily have no current binding when it is new, unmanaged,
missing its configuration, awaiting Custom Agent adapter setup, or in unresolved
recovery. Once a managed Agent has an exact verified configuration, it has one
current binding. It never has two.

An exact external configuration match may be adopted as current during
reconciliation. An ambiguous or partial match is drift, not a guessed current
binding.

### Current binding protection

The application rejects operations that would knowingly break the current
route, including:

- disabling, unbinding, or deleting the current binding;
- globally disabling or deleting its Provider;
- clearing a credential required by the current route; or
- changing the active adapter mapping or default model without applying the
  corresponding live configuration change.

If a Provider is current for several Agents, a global disable, delete, or
credential-clear operation is blocked until every affected Agent has switched.
The error lists those Agents without exposing their credentials.

The user must first switch successfully to another binding. Operations designed
to keep the route valid, such as “rotate local key and reapply” or “save model and
reapply,” may run as one coordinated transaction.

Deleting a managed Custom Agent is also blocked while its live configuration
selects one of its bindings. An explicit `Disconnect` action uses the same
journal, preview, write, verification, and rollback machinery to remove only the
adapter-owned mapped values. It does not guess or restore unknown values that
predated management. Once the detector proves that no binding-local credential
for that Agent remains selected, the existing Custom Agent deletion flow may
continue. Drifted Agents require reconciliation or a confirmed Disconnect before
deletion.

External changes can still make a route invalid. They produce an explicit
drift, missing, or partial state and never cause silent fallback.

When live state is drifted, missing, or partial, `current_binding_id` is null but
the switch-state row retains a non-authoritative last-verified reference for
recovery and explanation. Backend guards still block deletion, disabling,
credential clearing, adapter replacement, or Agent deletion when the uncertain
live state may reference an affected binding or target. The user must first
reconcile, disconnect, or prove that the binding-local credential is absent.

## Architecture

### AgentSwitchService

`AgentSwitchService` is the only new service allowed to commit a binding as the
current Provider for an Agent. It owns:

- binding, Provider, Agent, protocol, and credential validation;
- per-Agent and per-target-path serialization;
- proxy readiness checks for API routes;
- configuration preview and drift detection;
- switch journaling, protected snapshots, apply, verification, and rollback;
- current-binding compare-and-swap updates;
- startup recovery; and
- redacted audit events and actionable errors.

Legacy commands may call this service after resolving a linked binding, but no
UI or legacy service may write the per-Agent switch-state row directly.

### AgentConfigAdapter

An `AgentConfigAdapter` provides a narrow interface:

1. locate and validate its target files;
2. inspect live files and return a normalized, secret-safe selection state;
3. build a write plan from a resolved Provider projection;
4. generate a redacted preview;
5. write through the coordinator's transaction facilities; and
6. re-read and verify the expected selection.

The first release includes built-in adapters for:

- Codex;
- Claude Code;
- OpenCode;
- OpenClaw; and
- Hermes.

Built-in adapters reuse proven parsing and projection logic from the existing
live writers where its semantics are correct. They do not call the legacy
Provider service with a usage Provider ID. OpenCode, OpenClaw, and Hermes must
update the field that actually selects the default Provider or model; merely
adding a Provider entry to an additive configuration is not a successful
switch.

### Built-in adapter contracts

Every generated Provider alias is derived from the immutable binding ID, not a
display name. Existing unrelated Provider entries and MCP configuration remain
untouched. Additive clients may retain non-current entries, but the selector
must point to exactly the intended binding alias and model.

| Agent | API-mode owned fields | Exact verification |
| --- | --- | --- |
| Codex | `~/.codex/config.toml`: one LLM Usage Bar-owned `model_providers` entry with local base URL, wire API, and binding-local `experimental_bearer_token`; top-level `model_provider` and `model` | Owned entry, selector, model, local-key fingerprint, and preserved non-owned TOML/MCP content all match. API switching never overwrites `auth.json`. |
| Claude Code | `~/.claude/settings.json`: LLM Usage Bar-owned `env.ANTHROPIC_BASE_URL`, `ANTHROPIC_AUTH_TOKEN`, and `ANTHROPIC_MODEL` | Those three fields select the intended route and fingerprint; unrelated env, hooks, permissions, and role-specific model variables remain unchanged. |
| OpenCode | `~/.config/opencode/opencode.json`: one `provider.<binding-alias>` entry containing local URL/key plus top-level `model = "<binding-alias>/<model>"` | Provider entry, top-level model selector, model, and local-key fingerprint match while unrelated providers/plugins/MCP remain. |
| OpenClaw | `~/.openclaw/openclaw.json`: one `models.providers.<binding-alias>` entry plus `agents.defaults.model.primary = "<binding-alias>/<model>"` | Provider entry and primary model selector both match; unrelated defaults, fallbacks, Agents, tools, and Provider entries remain. |
| Hermes | `~/.hermes/config.yaml`: one LLM Usage Bar-owned `custom_providers` entry plus `model.provider` and `model.default` | Provider alias, default model, endpoint, and local-key fingerprint match while unrelated YAML sections remain. |

Codex official mode removes only an LLM Usage Bar-owned API selector, owned
entry, and owned bearer token, writes the binding's explicit top-level model,
then uses the existing managed Codex account bridge. Claude official mode
removes only LLM Usage Bar-owned `ANTHROPIC_BASE_URL` and authentication-token
overrides, writes the binding's explicit `ANTHROPIC_MODEL`, and verifies official
CLI login. Neither mode deletes unrelated user overrides or reads a client-owned
token store. Exact official detection requires both the owned route/model shape
and current official authentication evidence.

If an existing value at an owned path was not written by LLM Usage Bar, it is
drift and requires preview confirmation. An adapter never labels a field as
owned merely because its name is one the App normally uses.

### ProviderProjectionResolver

The resolver converts one effective binding into the values an adapter may
write. For an API binding the projection contains:

- a namespaced local proxy base URL for the Agent;
- the binding-local `lub_*` credential;
- the binding's explicit default model;
- stable non-secret Provider identity and display metadata; and
- protocol-specific field values.

The raw local key is read inside the backend only for the duration of the
transaction. It is never placed in an ordinary switch-state DTO or frontend
store. The upstream key is never part of the projection.

For a supported official subscription binding, the projection contains a
built-in “official mode” instruction rather than an OAuth token. Codex uses
the managed Codex OAuth path. Claude Code removes only adapter-owned API
overrides and defers to the official `claude auth` state. A Custom Agent cannot use a subscription
binding unless a future built-in bridge explicitly supports that exact Agent and
official authentication flow.

Subscription adapters use dedicated credential-safe bridges. The generic
configuration transaction never parses, copies, previews, journals, or snapshots
a client-owned OAuth token file. An official bridge exposes only opaque
prepare/apply/verify/rollback operations to the coordinator. The Claude bridge
never reads the Claude token store. The Codex bridge may use only the existing
managed-account credential path that already owns its protected account state.

### Agent route registry and protocol roles

Custom Agent support requires a dynamic local proxy audience; it cannot reuse a
hard-coded list of five Agent IDs. Every Agent has an immutable route audience
derived from its stable Agent ID. A Custom Agent receives a route such as
`/agents/{opaque-agent-id}/{protocol}/...`, never a path derived from its mutable
display name.

The route model separates:

- the Agent-facing protocol and route audience used to authenticate and parse
  the local request; from
- the Provider-facing wire format and upstream destination used after the
  binding is resolved.

A binding is switchable only when that exact protocol path is supported; the
first release does not infer cross-protocol translation. Fixed Agent routes keep
their existing compatibility aliases, while new binding creation and Provider
compatibility checks accept a Custom Agent according to its declared protocol
rather than a fixed Agent-name allowlist.

The proxy resolves a local credential to one binding and then requires the
request's route audience to match that binding's Agent ID. A valid key presented
under another fixed or dynamic Agent namespace fails before upstream I/O.

### AgentConfigStateDetector

The detector compares normalized live configuration with known effective
bindings without returning raw configured secrets. It may classify an exact
match by comparing protected credential fingerprints inside the backend.

On startup, refresh, or a relevant legacy action:

- an exact match to the recorded binding is `in_sync`;
- an exact match to another known binding updates current state with provenance
  `detected_external` and records an audit event;
- a partial or unknown match is `drifted`;
- an absent required target is `missing`; and
- an unsupported or intentionally unmanaged configuration is `unmanaged`.

Passive detection may update database metadata to reflect an exact live match.
It never writes Agent configuration and never guesses from Provider name alone.

### Session-source attribution epochs

Proxy events keep their existing strongest path: the local credential freezes
Agent, Provider, account-instance, and account-claim identity before upstream
I/O. A version-18 UsageEvent persists those immutable values; this design does
not require adding binding ID, and it never fabricates account provenance for an
older event.

Claude and Codex session import currently use one mutable
`usage_source_bindings` Provider pointer. Version 18 adds machine-local,
append-only source epochs containing source, fixed Agent, Provider, valid-from,
valid-to, account instance, account claim, provenance, and switch-state version.
Before the first externally
observable file replacement or official-bridge mutation, an App-managed switch
durably closes the old epoch and opens an `unproven transition gap`. A successful
final database commit opens the new Provider epoch at its verified commit time
and updates the compatibility pointer in the same transaction that changes
current state. Verified rollback opens a new epoch for the old Provider at
rollback completion; it never stretches the old epoch across the gap.

A late session record selects an epoch by its own occurrence time. No epoch or
overlapping/ambiguous evidence produces a warning and no guessed Provider event.
Records whose occurrence time falls inside a transaction gap remain unassigned,
even if the compatibility pointer still names the old Provider.
An exact external switch detected between two observations creates an unproven
gap: the old epoch ends at the last verified observation and the new one begins
at detection. Records inside the gap remain unimported unless stronger session
metadata proves Provider identity. Detection never rewrites already persisted
history.

Custom Agent adapters in this first release obtain usage through their proxy
route. They do not automatically create a new session-log parser.

### Live ownership and shared writer locks

Every built-in target has installation-local ownership and generation metadata.
Coordinator, legacy Provider, Profile, proxy takeover, adapter edit, recovery,
and proxy-origin operations acquire the same target locks and inspect that
ownership. No older writer may bypass a `partial` state or coordinator ownership.

A legacy Provider maps to a usage binding only when its explicit
`legacy_app_type` and `legacy_provider_id` link, the fixed Agent-to-App mapping,
and the binding all agree uniquely. Names, endpoints, display order, or matching
secrets are never linkage evidence.

After a linked coordinator switch, an exclusive legacy `is_current` marker is
updated only as a compatibility projection. When the current binding has no
legacy counterpart, the target is marked coordinator-owned and legacy current
is suppressed. Legacy backfill may not copy coordinator-owned proxy fields into
an old Provider record. A managed target with an unlinked legacy selection must
be linked or disconnected before that legacy writer can run directly.

Unmanaged targets keep existing legacy behavior. A linked legacy action enters
the coordinator. An unlinked action affecting a managed target opens the main
link/switch flow instead of writing around ownership. All successful legacy-only
actions rescan before their menu state is refreshed.

### Proxy lifecycle, Profiles, and takeover

App-managed API routes always use a loopback origin. An explicit switch may
start the proxy under a global lifecycle lock. If that transaction fails and it
was the sole reason for starting the proxy, with no other current or recovering
API Agent, rollback stops it. On application startup, current API routes cause a
start attempt; failure produces `in_sync + proxy_down`.

Stopping the proxy is blocked while any current API route or switch/recovery
journal depends on it. Changing its loopback origin is a batch move: start the
new listener, preview and rewrite every affected Agent under one parent journal,
verify all, commit all switch states, then stop the old listener. Failure leaves
the old listener and every old configuration authoritative.

Legacy takeover and coordinator ownership are mutually exclusive for one Agent.
Existing takeover and hot-switch remain available for unmanaged legacy targets.
Enabling takeover on a managed target is blocked until explicit Disconnect.
Switching a takeover-owned target offers `Disable takeover and switch`, which
restores and verifies the takeover backup before the coordinator claims the
target. A takeover hot-switch is never reported as a true configuration switch.

A Profile that affects one managed Agent follows the normal coordinator path. A
Profile affecting several managed Agents uses sorted global/Agent/target locks,
one batch preview, a parent journal with child transactions, and all-or-rollback
semantics. Current states commit only after every child verifies. If any child
needs drift, target-creation, or destructive confirmation, the native menu opens
the main batch preview instead of attempting a partial quick switch.

## Persistence Model

Schema version 18 is additive and does not rewrite existing historical usage,
quota, Provider, or binding identity. It adds nullable account-instance and
account-claim provenance fields to usage events, quota observations, and source
epochs. Pre-version-18 rows remain null and are explicitly `legacy_unscoped`;
they are never guessed or backfilled from a Provider's current credential.

### Per-Agent switch-state record

Add one `agent_switch_states` row for every built-in or Custom Agent when that
Agent is created or migrated. The row exists even before the Agent has a current
binding, so compare-and-swap versions cannot return to a previous “no row” state
and suffer an ABA race. It contains:

- `agent_module_id` as the primary key;
- nullable `current_binding_id`;
- nullable, non-authoritative `last_verified_binding_id` for recovery and UI
  explanation;
- durable configuration-state classification;
- orthogonal route-health classification and redacted reason;
- monotonic state version;
- adapter kind and adapter schema version;
- last observed and last verified normalized configuration digests;
- selection provenance (`applied` or `detected_external`);
- last observation, successful configuration verification, and route-health
  check times; and
- created and updated timestamps.

`current_binding_id` is non-null only while the durable state is `in_sync`. A
drifted, missing, setup-required, unmanaged, or partial transition clears it and
increments the state version; `last_verified_binding_id` does not confer current
status or permit the UI to show `In use`.

The binding table exposes a unique `(agent_module_id, id)` pair. Current binding
uses a restrictive composite foreign key from
`(agent_module_id, current_binding_id)` to that pair, so persistence—not only the
UI—prevents a binding from becoming current for another Agent. The
non-authoritative last-verified reference is cleared by the service before a
non-current binding is deleted.

Only a successful apply, completed recovery, or exact detector match may set
`current_binding_id`. Transient `switching` and `recovering` flags come from the
switch journal, not from prematurely changing current state.

All persisted configuration digests are installation-secret HMACs. Raw file
hashes and secret-bearing normalized material are neither returned to the UI nor
usable after export to another installation.

### Binding switch metadata

Each binding gains an explicit optional Agent-facing default model, an optional
upstream model mapping, and a monotonic switch-configuration version. The
default model is the value written into the Agent's selector. The optional
upstream model is the value selected after the proxy resolves that binding; when
it is absent, the Agent-facing value passes through. Both values and the mapping
version are frozen into switch preview and recovery state.

A binding without a valid default model for an adapter is visible but not
switchable. Existing seeded bindings are not assigned a guessed model during
migration. The UI requires the user to choose one or explicitly adopt a detected
model before the first switch. Changing the model on a non-current binding is a
normal metadata update. Changing it on the current binding requires a
coordinated reapply.

### Provider account-instance identity

Each `usage_providers` row gains one non-null, globally unique
`account_instance_id`. New Providers receive a random UUID. Migration derives an
existing Provider's value deterministically as UUIDv5 from a fixed application
namespace and that immutable Provider ID, so two offline installations upgrading
the same synchronized Provider converge on the same value. The field syncs with
that Provider and has a one-to-one uniqueness constraint; it is neither a
credential fingerprint nor proof that two independently configured accounts
match. It is immutable after creation; changing accounts always creates a new
Provider rather than repurposing historical Provider identity.

Every installation-local credential or official login also has a random,
immutable `account_claim_id`, claim generation, and proof status bound to its
Provider and account instance. Version-18 proxy events, quota observations, and
session-source epochs freeze Provider ID, account-instance ID, and account-claim
ID before external I/O or import attribution. Sync envelopes carry those stored
values and origin installation—not a later join to the mutable Provider row.

The existing nullable unique system-preset key remains exclusive to the five
seeded system Provider rows. Version 18 neither duplicates that key nor allows an
ordinary row to become a system Provider. The dedicated `Create separate
account` service instead creates an ordinary Provider with a null system-preset
key and an immutable, service-owned `catalog_origin_provider_id` referencing the
seeded row. That link copies the fixed Provider family, canonical route, billing,
and authentication kind under backend validation; generic save/import commands
cannot create or edit it. An official bridge accepts an additional account row
only through this validated catalog-origin link and its local account proof, so a
user-authored custom Provider cannot claim subscription-bridge privileges.

When an upstream exposes a stable non-secret account identifier, the App may
record an opaque `account_proof_token`: a Provider-namespaced digest under a
protected account-comparison key. Without sync, the token is installation-local.
Joining a sync profile performs a protected account recheck and derives a token
under that profile's shared comparison key; only that opaque token may sync. An
official bridge may supply an equivalent opaque stable handle. Raw identifiers,
email addresses, tokens, and credential fingerprints are never synchronized. A
remote installation may claim an existing account instance only by reproducing
its sync-profile proof token. If no comparable proof exists, attaching a new
local credential to that synchronized instance is forbidden and the user creates
a separate Provider/account instance.

Migration gives every existing local credential or official login its own claim
against the Provider's deterministic account instance. Equal proof tokens allow
those claims to aggregate under that instance. Different proof tokens, or
claims from different installations without comparable proof, are a conflict:
neither claim nor its usage/quota envelopes are merged. One side must create a
new Provider/account instance and explicitly rebind before future
account-sensitive sync resumes. Already persisted events keep their original
Provider, account-instance, and claim provenance; unresolved or pre-resolution
conflict envelopes remain quarantined rather than being relabeled. This may keep
legacy ambiguous history installation-local, but it never silently merges two
billing accounts.

### Machine-local route namespaces

Version 18 adds installation-scoped Agent route records containing the Agent ID,
opaque server-generated route namespace, Agent-facing protocol family, and
monotonic generation. The namespace is unique within an installation, contains
only a fixed URL-safe alphabet, and never changes when an Agent is renamed.

Fixed compatibility paths resolve to the same audience check as their route
record. Custom paths use only the opaque namespace. URL decoding must yield one
canonical segment; encoded separators, dot segments, mixed-normalization aliases,
and unknown protocols are rejected before credential lookup.

### Custom Agent adapters

Custom adapter records contain only non-secret configuration:

- Agent ID;
- protocol family;
- adapter schema version;
- ordered target files;
- file format for each target;
- allowlisted logical-field mappings;
- creation policy for an absent target;
- normalized path ownership metadata; and
- timestamps and monotonic version.

The supported protocol families are OpenAI-compatible, Anthropic-compatible,
and Gemini-compatible. Supported file formats are JSON, TOML, YAML, and
dotenv-style files.

Every target file mapping may reference only these logical values:

- `local_base_url`;
- `local_api_key`;
- `default_model`;
- `provider_id`; and
- `provider_name`.

JSON, TOML, and YAML use literal path segments. Dotenv uses exact variable
names. No JSONPath execution, expression language, string interpolation,
command substitution, includes, hooks, or executable templates are accepted.

An adapter must map `local_base_url`, `local_api_key`, and `default_model` at
least once. It may deliberately write one logical value to several declared
targets, but every occurrence receives the same resolved value and participates
in verification and rollback. Two mappings may not assign different logical
values to the same target field.

Incomplete wizard state lives in a separate installation-local
`custom_agent_adapter_drafts` record with Agent ID, partial non-secret form data,
wizard step, validation messages, `create` or `edit` kind, nullable base-adapter
version, and monotonic draft version. There is at most one draft per Agent. A
draft claims no target ownership, creates no proxy-usable route namespace, and
cannot be read as a validated adapter.

Initial `Save draft` creates the Custom Agent, permanent no-current switch-state
row, and create draft in one database transaction. Opening a `Setup required`
Agent resumes that draft at its saved step. Discarding a create draft removes
only the draft and leaves the Agent in `setup_required`; deleting that Agent is a
separate guarded action.

Editing an Agent with a validated adapter creates an edit draft bound to the
exact base-adapter version. The validated adapter, ownership, current route, and
detector remain authoritative while the draft exists. Discarding the edit draft
changes none of them. Promotion revalidates every field and target and requires
the base-adapter and switch-state compare-and-swap versions. An initial promotion
writes the validated adapter and route record, then removes the draft in one
database transaction. An edit promotion that could affect an active target uses
the previewed adapter-migration transaction; otherwise the Agent must first be
successfully disconnected. For an active migration, verified target bytes,
adapter/ownership publication, switch-state update, and draft removal join the
same `db_committed` transaction and recovery pre/post state machine as a switch.
No draft is silently promoted by sync or startup.

### Switch journal and recovery snapshots

The durable journal stores non-secret transaction metadata:

- expected Agent state, binding switch configuration, Provider route, adapter
  target-set, proxy origin, takeover ownership, and live ownership generations;
- expected local and upstream credential generations;
- canonical target identities;
- keyed before and intended byte digests plus normalized selection digests;
- completed phase and file commit markers;
- creation and stale-diagnostic times; and
- recovery status.

Byte-exact before-images can contain the local key and unrelated third-party
secrets from the same file. They are therefore secret-bearing recovery material,
not ordinary backups. Each transaction uses a random 256-bit key held in
protected credential storage. Every blob is encrypted independently with
XChaCha20-Poly1305, a unique random nonce, and associated data binding the
installation ID, journal ID, Agent, canonical target identity, metadata version,
and keyed before digest. Digest values use an installation-secret HMAC and never
leave the backend.

Encrypted blobs use user-only permissions, live in a dedicated no-backup and
no-sync application-data directory, and are excluded from diagnostics, export,
and normal backup manifests. Snapshot ordering is exact: reserve journal → stage
protected key → write/fsync/rename blobs → fsync directory → decrypt and verify
all blobs → mark `snapshot_ready`. No live target may change earlier.

The recovery directory also contains one independently durable, encrypted and
authenticated sidecar manifest for every non-terminal journal. It contains no
raw credential or file bytes, but binds the installation, journal ID, monotonic
phase, recorded database pre/post versions, target identities, and keyed blob
digests. It is written and fsynced with every phase transition and does not
depend on the main database surviving. Startup scans these manifests before
enabling writers; a missing or replaced database therefore cannot silently erase
evidence of a `snapshot_ready` or later transaction.

For `db_committed`, the SQLite transaction containing the post-state and journal
phase commits first; the matching sidecar phase is fsynced immediately after. A
crash between them may leave an earlier sidecar beside a proven post-state, in
which case recovery validates the database journal and advances the sidecar.
A sidecar alone never reconstructs or asserts a missing database post-state; it
blocks writers and requires recovery or manual diagnosis instead.

Successful completion or recovery removes blobs and their protected key; cleanup
failure remains a retryable terminal-journal cleanup state. An age threshold
raises a stale-recovery diagnostic but never deletes material for a non-terminal
`snapshot_ready` journal. Orphan cleanup may remove only material that can be
proven never to have reached `snapshot_ready`. A missing key, blob, or protected
store during recovery produces `partial` rather than an unverifiable restore.

## Custom Agent Adapter Experience

### Creation and setup

The existing name-only Create action is replaced by an adapter wizard rather
than creating an attribution-only record immediately. Custom Agents continue to
use the product's generic Custom icon; this feature does not add a new icon
persistence model. The wizard collects:

1. display name;
2. protocol family;
3. one or more configuration file paths;
4. file format for each path;
5. mappings for base URL, API key, and model, with optional Provider identity
   mappings;
6. whether a missing target may be created;
7. a read-only validation result; and
8. a redacted preview of a representative switch.

Cancelling before save creates no Agent. `Save draft` creates the Agent and its
installation-local incomplete adapter state as `setup_required`. Completing the
wizard validates and saves both together. A failed final save does not leave a
partly configured switchable Agent.

Agents settings shows `Resume setup` at the saved step and `Discard draft` for an
initial draft. Discard keeps the Agent as `setup_required` and offers its normal
separate Delete action. For an existing validated adapter, `Edit adapter` opens
or resumes the version-bound edit draft and `Discard edits` returns to the still
active adapter with current state and configuration untouched.

The user may save an incomplete Custom Agent, but it remains `setup_required`
and cannot be presented as switch-capable. A newly created Custom Agent is
release-complete only after the adapter validates and a compatible binding can
be applied.

Existing Custom Agents upgrade without configuration writes. They display
`Setup required` until the user completes the wizard.

### Path and file safety

Only a leading literal `~` is expanded; environment variables, command
substitution, and relative traversal are never expanded. Existing files are
chosen through a native file picker. Creating a missing target requires the user
to select its existing parent directory and then confirm the exact resulting
path in the preview.

The backend restricts Custom Agent targets to the current user's home directory
and denies sensitive or unrelated locations including SSH/GnuPG material, shell
startup files, Keychains, LaunchAgents, and LLM Usage Bar's database, backup,
sync, credential, journal, snapshot, and cache roots. Only user-owned regular
files are accepted. Symlinks, hard links, devices, sockets, FIFOs, files with
unsupported ACL/flag behavior, and targets over 4 MiB are rejected. Total
before-image size for one transaction is limited to 16 MiB.

Path ownership and locking use canonical Unicode/case-normalized identity plus
an opened parent-directory handle and file ID, not a second unchecked string
open. Commit revalidates the parent, target identity, link count, and ownership
with descriptor/handle-based no-follow primitives. Custom adapters cannot claim
a target already owned by another Agent adapter, including a built-in target.

For every accepted format subset, target editors preserve unrelated keys,
comments, and supported metadata and modify only mapped fields. A target that
cannot be parsed or preserved by the selected editor is rejected before any
write. Creating an absent target requires
the explicit creation policy recorded by the wizard and appears in the preview.
Replacement preserves the original owner and permission mode. A new file uses a
restrictive user-only mode, and the coordinator fsyncs both file contents and the
containing directory before claiming durability.

The first release accepts these explicit format subsets:

| Format | Accepted subset | Rejected input |
| --- | --- | --- |
| JSON | One standard JSON object; mapped paths traverse objects and end at string values | Comments, duplicate keys, arrays in mapped paths, non-object root |
| TOML | `toml_edit`-round-trippable document; mapped literal keys/tables end at strings | Duplicate definitions, array traversal, or a construct the editor cannot preserve byte-semantically outside owned fields |
| YAML | One mapping-only document along mapped paths; plain or quoted string leaves | Multiple documents, duplicate keys, anchors, aliases, tags, merge keys, mapped sequence traversal |
| dotenv | One assignment per mapped name; optional `export`; plain, single-quoted, or double-quoted single-line value | Duplicate mapped names, multiline values, interpolation, command substitution, or malformed quoting |

Unsupported syntax is a validation error, not a reason to rewrite the whole
file. The macOS release gate exercises descriptor-based no-follow and file-ID
checks. Another platform exposes Custom adapter switching only after equivalent
safe primitives and the same test matrix pass there.

### Compatibility

A Custom Agent can bind and switch among all effective API Providers whose
protocol family is compatible. This includes fixed API Providers and split-mode
custom Providers. It receives the same transaction, drift, recovery, status, and
current-binding protections as a built-in Agent.

Custom Agent support does not imply that an arbitrary subscription login can be
reused. Subscription Providers appear unavailable unless a dedicated official
authentication bridge exists.

## Credential Modes and Migration

### Fixed API Providers

Fixed API Providers retain their existing split model:

- one protected upstream credential belongs to the Provider; and
- one independently generated local credential belongs to every binding.

Switching writes only the binding-local credential.

### Custom Providers

One usage Provider continues to represent one upstream account and billing
identity. New custom API Providers therefore use one Provider-scoped upstream
credential, while every Agent binding receives an independent local credential.
Different upstream accounts require different Provider records; this design does
not hide several accounts behind one Provider ID.

Version 18 generalizes the existing protected Provider credential metadata from
fixed API cards to split-mode custom API Providers. Provider and binding records
expose a credential mode such as `provider_split` or `legacy_combined`, while raw
values remain in protected storage. Provider quota, connection tests, billing,
and aggregation continue to use that Provider-scoped account identity.

Provider ID alone is not accepted as cross-installation account proof. Every
logical Provider has a globally unique, syncable, non-secret
`account_instance_id`; it labels one upstream billing account and is never
derived from a credential. Local protected credential or official-login metadata
records which account instance it claims. Attaching a credential on another
installation to an already claimed instance requires the API or official bridge
to reproduce its synchronized `account_proof_token` from a provider-issued
stable account identifier or opaque stable account handle. Raw account
identifiers and credential fingerprints are not synchronized.

When stable proof is unavailable or differs, the App must not attach that local
credential to the synchronized Provider. It offers `Create separate account`
instead, cloning the Provider kind/preset into a new Provider ID and new account
instance, followed by explicit rebinding. Fixed system-card definitions serve as
immutable catalog presets for this operation; the migrated first account
Provider row keeps its existing Provider ID. Because one Provider ID remains one
account instance and version-18 evidence also freezes its claim, new UsageEvents
and quota observations cannot silently merge different accounts. Pre-version-18
evidence remains visibly `legacy_unscoped` and is never treated as account proof.
Conservative duplicates may remain separate even when the user knows they are
the same account but the upstream cannot prove it.

Existing custom or legacy bindings that use one secret both locally and
upstream are marked `legacy_combined`. They remain usable through their existing
paths, but App-managed switching is unavailable until the user runs an explicit
upgrade:

1. compare protected fingerprints for every legacy binding of that Provider;
2. if they represent one upstream account, stage that secret as the Provider's
   upstream credential; if they differ, require the user to create or duplicate
   one Provider per account and rebind explicitly;
3. generate a new independent `lub_*` local credential for every upgraded
   binding;
4. verify the proxy route using the two separate roles;
5. publish the new metadata and clean up obsolete slots through a durable
   credential-upgrade journal; and
6. leave the old combined route authoritative if any step fails.

Upgrade is blocked while any such binding is verified current, is the
non-authoritative last-verified binding under drift/missing/partial state, or
cannot be proven absent from every managed live target. The user must first
switch away or complete a guarded `Disconnect`, then rescan. Only a proven
non-selected legacy binding may enter the upgrade journal; after a successful
upgrade the user explicitly switches back. This prevents revoking the selected
combined secret without rewriting and verifying the Agent configuration.

SQLite and protected storage do not share one atomic transaction. The upgrade
therefore follows reserve → stage new slots → verify → publish metadata → clean
old slots. Startup reconciliation completes or rolls back each phase from its
journal, and orphan cleanup never invalidates the old combined path before the
publish marker is durable.

The upgrade does not write an Agent's configuration automatically. After it
succeeds, the user explicitly switches or reapplies the binding.

### Credential lifecycle while current

Clearing an upstream or local credential required by the current binding is
blocked until another binding is current. Local-key rotation for the current
binding uses a staged generation: the old key remains valid, the proxy accepts
the staged key only for the same binding and transaction, live configuration is
written and verified, and only then does compare-and-swap promotion revoke the
old key. Crash recovery promotes the staged key only when every intended file
and generation still matches; otherwise it restores the file and discards the
staged key.

Replacing an upstream key used by a current binding follows stage → protected
connection test → account-continuity proof → publish. The new key or official
login must reproduce the Provider's existing `account_proof_token`; a mismatch
or unavailable proof cannot publish in place and instead offers `Create separate
account` followed by explicit rebinding. Failure preserves the old key. A
successful same-account publish does not rewrite Agent configuration, but it
increments route generation and refreshes route health. An official logout or
disconnect command is blocked while that subscription binding is current;
authentication that expires or is removed outside the App produces unhealthy
route state without silent fallback, and a later different-account login is not
adopted under the old Provider identity.

## Switch Transaction

An apply request includes the binding ID, expected switch-state version,
expected adapter version, and an opaque, short-lived preview token. The backend
binds that token to the normalized live digest and intended selection. The UI
never receives the raw digest or a hash derived directly from secret-bearing
configuration.

The coordinator performs these phases:

1. Acquire the global lifecycle/source locks when needed, then Agent and
   canonical target-path locks in stable order.
2. Re-read the Agent, binding, Provider, adapter, live ownership, takeover,
   proxy, source-binding, and credential metadata.
3. Verify ownership, compatibility, effective enabled state, default model,
   switch-state version, and every generation bound to the preview token.
4. For an API projection, ensure the App-managed origin is loopback and start or
   health-check the local proxy through its lifecycle lock, then run the
   Provider's protected connection test. For an official projection, perform a
   read-only authentication preflight and reserve the dedicated opaque bridge;
   `prepare` does not mutate official account state. A failed or unsupported
   required preflight performs no configuration write and leaves the target
   unswitchable in this release.
5. Read every target byte-for-byte through its validated handle, parse it, and
   compare its normalized state with the server-side preview state.
6. If drift exists, return a conflict with a redacted difference. A user may
   preview again and approve replacing that exact previewed state; a stale
   approval never authorizes a newer file.
7. Build one explicit projection variant entirely in the backend:
   `ApiProxyProjection` resolves the binding-local credential, while
   `OfficialModeProjection` contains no binding-local or OAuth token.
8. Reparse every proposed temporary output and verify its exact intended bytes
   and normalized selection before touching a live target.
9. Reserve the journal with the exact pre-state and intended post-state versions
   and generations, stage any credentials or reversible official-bridge state,
   persist and verify encrypted byte-exact before-images, then durably mark
   `snapshot_ready`. Before the next step, close the old source epoch and durably
   mark its unproven transition gap when that Agent has session import.
10. For an official projection, call bridge `apply` only now, record its durable
    transaction marker, and retain the exact opaque rollback state. Then write
    and fsync temporary files, replace targets in deterministic order, fsync
    their containing directories, and record commit markers.
11. Re-read all targets and require both exact intended bytes and an exact
    expected binding selection. An official projection must also pass bridge
    `verify` against the intended account proof before database commit.
12. In one database transaction, compare-and-swap from the recorded pre-version
    to the intended post-state, update live ownership, open the new
    session-source epoch, update its compatibility pointer, project linked
    legacy current metadata, and mark the journal `db_committed` with the exact
    post-version and generations. No current state changes until every target
    verifies.
13. Idempotently promote the staged credential or finalize the already applied
    and verified official bridge by discarding only its rollback reservation;
    bridge finalization does not change the selected official account. Retain the
    old protected generation/bridge before-state until this is durable. Mark
    `external_finalized`, publish a redacted state event, then mark terminal and
    remove recovery material. Old protected state is not revoked before the
    terminal path can be recovered.

In the normal `in_sync` case, choosing `Switch` is itself the explicit action and
does not require a second confirmation dialog. A drift conflict, target
creation, or destructive reconciliation requires a redacted preview and explicit
confirmation.

### Failure and crash recovery

Before any live replacement, failure leaves existing files and switch state
unchanged. After one or more replacements, synchronous rollback first proves
that each affected target still has the file identity and exact byte digest
written by this transaction. Only those owned outputs may be restored
byte-for-byte. A newly created target may be deleted only when its identity and
digest still match the transaction-created file. Any unknown external change
stops automatic rollback and enters `partial`; the App never overwrites the
newer content. Live-file rollback is allowed only before `db_committed`; a
failure after database commit follows the post-state finalization or compensating
state transition below and never restores the old file set.

Before `db_committed`, any transaction-owned official bridge apply is part of
the same synchronous rollback: bridge `rollback` must prove and restore its
opaque before-state before the old route is reported restored. An unprovable
bridge rollback enters `partial` and does not claim either account as current.

At startup, recovery acquires the same locks and inspects every non-terminal
journal before enabling any writer:

- Every journal carries distinct recorded pre-state and intended post-state
  versions/generations. `snapshot_ready`, file markers, `db_committed`,
  `external_finalized`, and terminal cleanup are monotonic phases; recovery never
  infers a phase merely from file contents.
- If the database still matches the recorded pre-state, forward completion is
  allowed only when installation identity, binding/model, Provider route,
  adapter/target set, credentials, proxy origin, takeover/ownership, exact
  intended bytes, normalized selection, all pre-generations, and any recorded
  official bridge apply marker and fresh verification match. Recovery then
  performs the original compare-and-swap once. Otherwise it may roll back only
  proven transaction outputs and an applied bridge, verify exact before-images,
  and open a new old Provider source epoch at rollback completion.
- If the database matches the recorded post-state and its journal phase is
  `db_committed` or later, recovery never reruns the pre-state
  compare-and-swap or restores old files. It verifies the post-state and finishes
  staged credential or bridge finalization idempotently. If the post-state
  cannot be proven or finalization cannot be made safe, a compensating
  compare-and-swap from the recorded post-version clears current state, enters
  `partial`, closes any new source epoch, and opens an unproven gap; it never
  overwrites an unknown target.
- If the database matches neither the recorded pre-state nor recorded
  post-state, recovery enters `partial` without mutating live files or protected
  credentials.
- Restoration before database commit is allowed only when all recorded
  generations still match and every target is either an unchanged transaction
  output or an untouched before-image. Recovery restores only proven transaction
  outputs, invokes and verifies official bridge rollback when it had applied,
  and verifies exact original bytes and metadata.
- A compare-and-swap conflict, changed inode/file ID, unknown byte digest,
  missing protected key/blob, unsupported metadata restoration, or external
  modification becomes `partial`. Recovery does not “try the old snapshot
  anyway.”
- `partial` blocks coordinator, legacy, Profile, takeover, adapter, credential,
  and proxy-origin mutations for the affected targets. Diagnostics provide a
  redacted manual-recovery path and never expose snapshot contents.

The service never reports success for a partially written multi-file adapter.

## State Model

Configuration synchronization and route usability are orthogonal. The durable
`config_state` classifications are:

| State | Meaning | Allowed next action |
| --- | --- | --- |
| `in_sync` | Live configuration exactly selects the current binding | Switch or reapply |
| `drifted` | Files are valid but do not exactly match one known selection | Preview and repair, or explicitly overwrite the previewed state |
| `missing` | A required target does not exist | Create through approved adapter policy or repair path |
| `setup_required` | A Custom Agent has no valid adapter | Complete adapter wizard |
| `unmanaged` | Adapter intentionally cannot manage current configuration | Configure a supported adapter or leave observation-only |
| `partial` | Interrupted mutation cannot yet be proven complete or restored | Resolve recovery before any new mutation |

Only `in_sync` has a non-null verified current binding. `switching` and
`recovering` are transient activity flags layered on these states. While an
operation is pending, the frontend keeps the previous verified frame instead of
showing a loading placeholder. Once a completed observation reports drifted,
missing, unmanaged, setup-required, or partial, it removes `In use` and displays
`Current unknown` plus the non-authoritative last-verified Provider when one
exists.

The independent `route_health` values are:

| Health | Meaning |
| --- | --- |
| `usable` | Required proxy/authentication/credentials and the last protected health check pass |
| `unknown` | Configuration is known but route health has not yet been proven |
| `proxy_down` | An API route selects the local proxy but its loopback listener is unavailable |
| `auth_missing` | The official client login required by the current binding is unavailable |
| `credential_unavailable` | A required protected local or upstream generation cannot be resolved |
| `upstream_unhealthy` | Protected connection testing or a real request proves an upstream failure |

An `in_sync` binding remains the current Provider when route health becomes
unhealthy, because the live selector has not changed. The App shows both states,
does not fall back, and offers the relevant restart, login, credential, or test
action. User-initiated operations that would knowingly degrade a usable current
route use staged validation or are blocked; external expiry and outages are
reported rather than reclassified as configuration drift.
An expired health observation becomes `unknown`, not `usable`; it does not claim
that an untested route is healthy.
When configuration state has no verified current binding, route health is
`unknown` and cannot make an unavailable binding appear current.

## Commands and Secret-Safe DTOs

The frontend receives focused operations equivalent to:

- list or get Agent configuration states, route health, verified current, and
  non-authoritative last-verified state;
- preview a binding switch;
- apply a binding switch with expected versions and an opaque preview token;
- preview and apply a Profile or proxy-origin batch;
- refresh or reconcile one Agent's live state;
- inspect guarded mutations and their affected Agents;
- list and preview switch-sensitive sync mutations and account-claim conflicts;
- resolve a pending sync item through guarded apply, durable keep-local, or
  create-separate-account as allowed by its backend reason;
- get, save, discard, and validate/promote a Custom Agent adapter draft;
- validate, save, and remove a Custom Agent adapter;
- preview and disconnect a managed Custom Agent;
- preview and perform a legacy combined-credential upgrade; and
- rotate and reapply a current local key.

Every binding-row DTO must contain one backend-authoritative `switchability`
object with `state`, ordered `reason_codes`, `primary_repair_action`, and all
evaluated state/generation versions. The frontend never derives switchability
from `effective`, model, credential, adapter, or health fields. `state` is exactly
`ready`, `confirmation_required`, or `blocked`; a ready non-current row has no
reasons and action `switch`, while a ready current row has action `reapply` and
still renders `In use`. Confirmation requires action `preview_switch`.

Stable reason codes are: `drift_preview`, `target_creation`,
`takeover_transition`, `batch_preview`, `recovery_partial`,
`ownership_conflict`, `adapter_setup_required`, `adapter_invalid`,
`protocol_incompatible`, `account_claim_conflict`, `legacy_upgrade_required`,
`provider_disabled`, `binding_disabled`, `credential_missing`, `login_missing`,
`model_missing`, `official_bridge_unsupported`, `proxy_unavailable`, and
`upstream_unhealthy`. Stable repair actions are `switch`, `reapply`,
`preview_switch`, `view_diagnostics`, `resolve_ownership`, `setup_adapter`,
`edit_adapter`, `create_separate_account`, `upgrade_legacy`, `enable_provider`,
`enable_binding`, `set_credential`, `login`, `edit_model`, `restart_proxy`, and
`test_connection`.
When several reasons apply, the backend orders them by recovery/safety block,
ownership, adapter/protocol/account migration, enabled/credential/login/model,
then runtime readiness; the first actionable reason determines the primary
repair action.

Other read fields may contain Provider identity, model, protocol, state
versions, opaque preview tokens, redacted diffs, and timestamps. They never
contain raw local keys, upstream keys, OAuth tokens, recovery bytes,
credential-store slots, or full configured secret values. Preview tokens are
single-use, expire quickly, and are invalidated by any relevant Agent, binding,
adapter, credential, or file state change.

Existing user-initiated local-key copy remains a separate sensitive command. The
switch UI does not call it; the backend reads the key internally.

## User Experience

### Agent dashboard

The selected Agent page gains a compact current-Provider controller near its
header. It shows:

- current Provider name;
- current default model;
- configuration state and route health; and
- a menu of enabled bindings available to that Agent.

The current row reads `In use`. A `ready` row offers `Switch`; a
`confirmation_required` row opens its preview. A `blocked` row remains visible
with a precise reason and the appropriate
credential, login, model, or adapter setup action.

The controller is independent of usage-history projection and renders before
the current `no historical Provider` early-return/empty state. A newly created
Agent can therefore be configured and switched before it has usage. Switching
does not hide or rewrite historical Provider cards below it.

Changing the dashboard Agent tab never invokes a switch. Pending state is scoped
to the affected Agent/binding/targets; switching Agent A does not freeze Agent B
or prevent navigation.

### Agents settings

The existing binding editor exposes three explicit concepts:

- `Bound to this Agent` for relationship existence and Unbind;
- `Available to this Agent` for the binding enabled toggle; and
- `Current Provider` / `In use` for live selection.

The current binding cannot be disabled or removed. Other `ready` or
`confirmation_required` bindings expose the corresponding switch action. Custom
Agent details include the adapter setup or edit entry,
validation status, target summary, a redacted test preview, and a guarded
`Disconnect` action. Editing the targets of an active adapter requires either a
previewed adapter-migration transaction or a successful Disconnect first.

Every binding row contains a `Default model` editor and optional upstream model
mapping. A non-current binding uses `Save`. A current binding uses
`Save & reapply` and does not publish metadata unless live reapply succeeds.
Missing or invalid models keep the row visible with a precise not-switchable
reason and focus the editor from its repair action.

### Providers settings

The Provider page remains the global place for Provider identity, enabled state,
authentication, credentials, connection tests, and Agent bindings. Existing
Provider-side checkboxes are relabeled as the `Bound` relationship instead of
pretending that binding existence means enabled. Each bound Agent row exposes a
separate availability switch, effective/switchable status, and Agent-specific
`In use` state.

Before global disable, delete, credential clear, logout, or another guarded
operation, the page must list every affected current or uncertain Agent. The
backend DTO supplies the guard reason and affected Agent IDs; this is mandatory,
not an optional informational projection. There is still no one global
`is_current` flag because current selection is Agent-specific.

Legacy combined custom bindings show `Upgrade required for App-managed
switching` and expose the explicit split-credential upgrade flow.

Providers and Agents settings share a `Sync changes need review` banner and
queue. Each item shows its source installation, redacted change, affected Agents,
and backend-supplied blockers, then offers only the allowed `Review & apply`,
`Keep local`, or `Create separate account` action. Applying opens the same
preview/reapply or Disconnect flow used locally; account conflicts never offer a
merge action without matching proof. Resolved items disappear only after their
causal resolution marker is durable.

### Menu bar and existing actions

The left-click popover remains usage-only. This avoids accidental configuration
writes from the compact status surface.

The existing native right-click Provider/Profile menu remains available. A menu
checkmark comes only from a verified current binding or an unmanaged legacy
current marker; drifted, missing, or partial managed state has no checked
Provider and shows `Current unknown`.

A linked item first performs server-side preview against the explicit legacy
link. It may quick-apply only when the Agent is `in_sync`, the target is
`ready`, no target creation or destructive reconciliation is required, and
the freshly issued preview token remains valid. Drift, missing targets,
takeover transition, Profile batch confirmation, stale preview, or another
confirmation requirement reveals the Dock/main window and opens the switch
controller. A stale token is discarded and the main controller performs a new
preview before rendering any confirmation; a still-valid prepared preview may
be reused. An unlinked action for an unmanaged target retains legacy
behavior and rescans. An unlinked action for a managed target opens the link or
Disconnect flow and never writes around coordinator ownership.

During native quick-apply, the old verified checkmark remains in place and the
selected target shows a per-Agent `Switching…` pending marker; duplicate or
conflicting actions for that Agent are disabled, while unrelated Agents remain
usable. The checkmark moves only after the backend returns a verified commit and
the menu refreshes from a fresh detector result. A rolled-back failure retains
the old checkmark; `partial` removes every managed checkmark. Success produces a
non-blocking operation result. Any non-stale failure, rollback, or partial result
reveals the main controller with its stable reason and repair action; if native
notification permission is unavailable, a status-item error badge persists
until that result is opened. A quick switch never fails silently merely because
the menu closed.

## Compatibility and Migration

The schema migration itself performs no Agent configuration writes.

On first startup after upgrade:

1. built-in adapters scan their live files;
2. an exact known effective binding is adopted with provenance
   `detected_external`;
3. unknown or partial configurations become `drifted`, `missing`, or
   `unmanaged` without being overwritten;
4. existing Custom Agents become `setup_required` until configured; and
5. legacy combined credentials remain unchanged until explicit upgrade.

The new binding-centric current state is authoritative for new switch UI. Legacy
`providers.is_current` remains compatibility data for legacy screens and actions;
it is not joined to a usage binding without the existing explicit legacy link.

### Machine-local persistence, sync, and restore

Current selection is a property of one machine's live files. The following are
installation-local and excluded from WebDAV/S3/application sync, logical export,
diagnostics bundles, and normal backup manifests:

- per-Agent switch states and live-ownership generations;
- Custom adapter paths, target identities, and validation state;
- route namespace records and proxy-origin generations;
- source-attribution epochs and their compatibility current pointer;
- preview tokens, journals, encrypted snapshots, staged credentials, and
  recovery keys.

Logical Agent, Provider, binding, and default-model metadata continues under its
existing sync/export rules, including the non-secret Provider account-instance
identity, but an imported Custom Agent is `setup_required` until its adapter is
configured and validated on that installation. Incoming disable, delete,
Unbind, Provider-account replacement, or default-model changes that could affect
a locally current or uncertain route are stored as an installation-local
`switch_sensitive_pending` mutation instead of being applied to the referenced
logical row. Safe display metadata may merge immediately. A pending mutation is
published locally only after the same locks, detector, affected-Agent guards,
preview, and coordinated reapply or Disconnect used by an interactive change;
remote sync never bypasses current-binding protection.

Every pending item has an opaque mutation ID, causal sync version, source
installation label, redacted field-level summary, affected Agent IDs, blocker
codes, and allowed resolution actions. `Apply remote` reruns detection and a
fresh preview, then publishes only through the normal guarded transaction.
`Keep local` records a durable causal rejection so the same remote mutation is
not requeued; later genuinely newer remote edits may still conflict. An account
claim with matching proof resolves automatically. A different or unprovable
claim permits only `Create separate account` plus explicit rebind, or `Keep
local` while its incoming account-sensitive envelopes remain quarantined. No
resolution action relabels an existing usage event.

Every machine-local row and recovery blob is bound to a protected installation
identity. A row from another installation is quarantined and never executed. An
App-supported binary or SQL restore first scans local sidecar manifests and
refuses to replace the database while any local journal is non-terminal; the
user must complete recovery or an explicitly diagnosed manual recovery first.
Recovery material arriving inside an imported backup is foreign and is
quarantined, never executed, merged, or allowed to overwrite local manifests,
blobs, or protected keys. After an accepted restore, the App clears verified
current state, regenerates route namespaces, and forces read-only detection
before any write. A restored exact live match may be adopted only by the local
detector, and database restore never triggers a configuration write.

### Superseded boundaries

This specification replaces only the listed portions of earlier approved work:

| Earlier document | Replaced boundary | Boundary that remains |
| --- | --- | --- |
| `2026-07-10-usage-dashboard-design.md` | No Provider switching UI or quick switch | Usage remains evidence-based; there is no automatic failover or history rewrite |
| `2026-07-12-subscription-dashboard-navigation-design.md` | Its prohibition on Provider quick switching and changing an external Agent's selected route | Module/Agent navigation itself still changes view only; account contexts never auto-merge and there is no automatic failover |
| `2026-07-14-agent-centric-usage-modules.md` | App never edits Agent config; every custom binding key serves both local and upstream roles; Custom Agent deletion may immediately invalidate bindings | Explicit switch/disconnect may edit validated config; historical events remain immutable; archive identity remains resolvable |
| `2026-07-15-default-provider-cards-design.md` | No explicit external switch; custom credentials stay combined; Claude subscription is observation-only for every action | Fixed cards and protected credentials remain; Claude OAuth token is never read/copied/proxied; quota observation limits remain |
| `2026-07-15-menu-bar-usage-popover-design.md` | Management actions may not change how an external Agent selects its Provider | Left-click popover remains usage-only; existing Provider/Profile management and menu-bar-first lifecycle remain |

Claude subscription switching means selecting or restoring official Claude CLI
mode after `claude auth` succeeds. It does not create a Claude OAuth client,
extract a token, or add quota access that the official CLI does not expose.

The word “automatically” remains important: navigation, monitoring, migration,
quota state, and background refresh never initiate a switch.

## Security Requirements

- Raw upstream credentials never enter Agent configuration or frontend state.
- Binding-local credentials cannot be replayed across Agent namespaces.
- Configuration, preview, error, diagnostic, telemetry, backup, and journal
  outputs are redacted before logging or serialization.
- Target files are opened and replaced with canonical path, file-ID, ownership,
  mode, link-count, hardlink, symlink/reparse-point, special-file, size, and
  race checks through descriptor/handle-based primitives.
- Generic configuration snapshots never include a client-owned subscription
  token file; official authentication remains inside its dedicated bridge.
- Recovery material is encrypted at rest, independently keyed from database
  metadata, installation-bound, and retained until its journal is terminal.
- Switch requests use compare-and-swap versions and backend-bound opaque preview
  tokens to prevent a stale UI from overwriting a newer external edit.
- Adapter validation and switch preview never send an upstream request. A
  connection test, credential upgrade verification, or applied switch may do so
  only after local ownership and credentials are already proven.
- There is no automatic fallback after authentication, network, or write
  failure.
- App-managed API endpoints are loopback-only, and proxy stop/origin changes use
  the same backend current-route guards as Provider and binding mutations.
- Disable, delete, unbind, archive, adapter edit, credential clear/logout,
  Profile, legacy, takeover, and proxy commands all invoke backend guards; UI
  disabling is never the only enforcement.
- Switching does not mutate historical usage events, quota snapshots, or prior
  request attribution.
- Custom adapter configuration is data, never executable code.
- Official subscription modes use supported official flows and never copy a
  client-owned OAuth token into another Agent.
- Sensitive backend buffers are scoped to the transaction, excluded from debug
  formatting, and zeroized on a best-effort basis after use.

## Testing Strategy

### Unit tests

- current-binding ownership and one-current-per-Agent invariants;
- permanent no-current rows, monotonic versions, and ABA rejection;
- switch-state, projection-generation, opaque-token, and keyed-digest
  compare-and-swap behavior;
- built-in and dynamic Custom Provider/protocol compatibility matrices;
- immutable opaque namespace generation, rename stability, URL normalization,
  traversal rejection, and cross-audience replay rejection;
- projection rules proving that local, not upstream, credentials are selected;
- Agent-facing versus upstream model mapping;
- JSON, TOML, YAML, and dotenv accepted/rejected subset and round-trip behavior;
- literal path-segment and allowlist validation;
- path ownership, denylist, duplicate/casefold target, file ID, hardlink, symlink,
  special file, size, ACL/flag, and permission rejection;
- configuration-state versus route-health transitions;
- authoritative switchability state, blocker priority, stable reason codes, and
  primary repair actions;
- session-source epoch selection, late records, external-detection gaps, and no
  historical rewrite;
- deterministic offline Provider-account migration, immutable account/claim
  provenance, proof-token matching, separate-account fallback, and conflicted
  sync-envelope quarantine;
- redaction for previews, errors, journals, diagnostics, and events; and
- legacy combined-to-split credential stage/publish/reconcile transitions.

### Service integration tests

- all five built-in adapters select the actual Provider and default model;
- every built-in adapter preserves unrelated config, MCP, comments, Provider
  entries, and client-owned authentication state defined by its contract;
- Custom Agent single-file and multi-file switches across all three protocol
  families and all four supported format subsets;
- exact external selection detection and adoption;
- drift conflicts and stale-preview rejection;
- failpoints before, during, and after every file replacement, database commit,
  credential promotion, and official-bridge finalization;
- official bridge read-only prepare, post-snapshot apply, pre-commit verify,
  pre-commit rollback, and post-commit cleanup ordering;
- byte-exact rollback after a later target fails;
- refusal to overwrite an external edit made after a transaction output;
- pre-commit startup completion only when exact bytes, every generation, and CAS
  match, with restoration limited to unchanged transaction-owned outputs;
- post-commit startup finalization that never restores old files, plus safe
  compensating `partial` transitions when finalization cannot be proven;
- unresolved recovery blocking subsequent writes;
- XChaCha20-Poly1305 key/blob/journal ordering, protected-store loss, orphan
  cleanup, sidecar lag and database replacement, terminal cleanup retry, and no
  sync/export of recovery material;
- staged local-key rotation, staged current upstream-key replacement, failure
  preservation, and crash recovery;
- same-account proof for upstream replacement and separate-account fallback on
  mismatch or unavailable proof;
- transaction source gaps before live mutation, verified new/old epoch opening,
  and zero attribution through an unproven gap;
- refusal to upgrade a current or uncertain `legacy_combined` binding, plus
  non-current upgrade publish/rollback recovery;
- concurrent switch, linked/unlinked legacy, Profile batch, takeover, credential
  rotation, proxy-origin move, and adapter edit attempts;
- proxy auto-start/compensation, guarded stop, batch origin move, and route-health
  failure;
- offline migration convergence, equal-proof claim aggregation,
  missing/mismatched-proof and stored-event quarantine, sync staging of
  switch-sensitive remote mutations, restore refusal with local non-terminal
  sidecars, foreign recovery quarantine, and read-only detection after accepted
  restore;
- local-key invalidation, cross-Agent namespace rejection, and zero upstream I/O
  on identity failure; and
- unchanged historical attribution across switches.

### Frontend tests

- controller rendering above the no-history empty state;
- verified current, last-verified, configuration state, route health, model, and
  precise unavailable reasons;
- Bound versus Available versus Effective versus Switchability versus Current
  controls;
- backend-authored switchability reasons/actions with no frontend inference;
- default-model Save and current-binding Save & reapply;
- blocking disable, unbind, delete, and credential-clear actions on current
  bindings;
- affected-Agent lists for global Provider guards and uncertain state;
- switch success, unavailable reason, drift preview, failure, rollback, and
  recovery states;
- Custom Agent wizard cancel, create-draft save/resume/discard,
  setup-required migration, version-bound edit-draft resume/discard/promotion,
  validation, missing-target confirmation, and redacted previews;
- guarded Custom Agent disconnect and deletion;
- legacy credential upgrade prompts; and
- linked native quick switch, confirmation fallback to the main window, Profile
  batch preview, pending/checkmark rules, success/failure feedback, and unmanaged
  legacy fallback;
- switch-sensitive sync queue, guarded apply, durable keep-local, and
  create-separate-account resolution;
- per-Agent activity that does not freeze unrelated Agents; and
- no switch from dashboard navigation, sorting, hiding, or menu-bar popover use.

### Environment isolation

All automated writer and recovery tests use a temporary HOME, temporary
database, isolated protected-credential test store, and mock upstream. They must
not start the desktop application against the developer's real configuration or
credentials.

Local Rust commands use `pnpm rust -- ...`. Repository-wide frontend verification
excludes sibling `.worktrees/**` as required by the repository instructions.

## Delivery Decomposition

The implementation plan may use dependent milestones so each safety boundary is
reviewable without redefining first-release completion:

1. Version 18 state, installation-local persistence, dynamic route audiences,
   credential split/upgrade, ownership locks, preview tokens, journal, and
   recovery primitives.
2. Five built-in adapters, official bridges, route health, proxy lifecycle,
   session-source epochs, and linked legacy/takeover behavior.
3. Custom Agent wizard, protocol compatibility, four declarative format
   adapters, single/multi-file transactions, and Custom Provider upgrade UI.
4. Dashboard/Settings controller, model editor, mutation guards, native menu,
   Profile batch, Disconnect, and diagnostics.
5. Security review, failure injection, isolated end-to-end matrix, existing
   regression gates, and completion audit.

These are implementation milestones, not smaller product releases. The first
release is incomplete until milestone 5 proves the Custom Agent gate below.

## First-Release Acceptance Gate

The feature is not complete unless an automated end-to-end test proves this
Custom Agent workflow:

1. Cancel an unsaved Custom Agent wizard and prove no record or file is created;
   then save a draft and prove it is `setup_required` with zero configuration
   writes, close the flow, and resume at the exact saved step.
2. Complete and validate the declarative adapter, including one explicitly
   confirmed missing target, and prove cancelling that target preview performs
   zero writes.
3. Bind two protocol-compatible API Providers with Provider-scoped upstream
   credentials and independent local credentials.
4. Assign an explicit Agent-facing default model and upstream mapping to each
   binding.
5. Switch from Provider A to Provider B and back to Provider A.
6. Verify after every switch that base URL, binding-local key fingerprint,
   default model, model mapping, switch-state record, `in_sync` state, and
   `usable` route health agree. The fingerprint assertion occurs inside the
   isolated backend test harness, not in a frontend DTO.
7. Verify unrelated fields, comments, Provider entries, owner/mode, and approved
   file metadata remain unchanged.
8. Send a request after every switch and prove the dynamic route resolves the
   intended binding, the mock upstream receives the correct upstream credential,
   and the immutable usage event retains the Custom Agent and Provider identity.
9. Rename the Custom Agent and prove its opaque route namespace remains stable;
   present its local key under another Agent namespace and prove zero upstream
   I/O.
10. Prove the current binding cannot be disabled, unbound, deleted, cleared, or
    globally disabled, and that every affected-Agent guard is reported.
11. Inject a later-file write failure and prove byte-exact rollback with the
    original current binding unchanged.
12. Simulate application crashes before and after `db_committed`. Before commit,
    prove startup completes only with exact bytes/generations/CAS or restores
    only unchanged transaction-owned output. After commit, prove recovery never
    restores old files and either finalizes idempotently or enters a guarded
    compensating `partial` state. In both cases, prove records inside the source
    transition gap receive no guessed Provider.
13. Modify a transaction output externally before recovery and prove the App
    enters `partial` without overwriting that edit.
14. Detect an exact external Provider B selection as `detected_external`, then
    make an unknown edit and prove it becomes `Current unknown` drift without an
    implicit overwrite.
15. Disconnect the Custom Agent through a confirmed transaction, prove no local
    binding credential remains selected, and complete its existing delete flow.
16. Prove successful completion and recovery remove terminal encrypted blobs,
    journals, sidecars, and protected snapshot keys after cleanup succeeds, then
    scan UI payloads, logs, database rows, diagnostics, export/sync data, and
    recovery metadata for upstream and local secret sentinels.

The Custom Agent gate is parameterized so the suite directly covers all three
protocol families, JSON/TOML/YAML/dotenv, and both single-file and multi-file
adapters. Separate acceptance cases prove existing Custom Agents migrate to
`setup_required`, unsupported subscriptions and missing/invalid models remain
unswitchable, a current or uncertain `legacy_combined` binding cannot upgrade,
and a permitted upgrade's success or failure preserves one unambiguous Provider
account and the old route until publish. Cross-installation cases must also prove
that an unverified local credential creates a separate account instance rather
than merging Provider-keyed usage.

Draft acceptance cases also prove that discarding an initial draft leaves its
Agent in `setup_required` with no configuration write, while discarding an edit
draft leaves the validated adapter, ownership, current route, and live bytes
unchanged. Stale base-adapter versions must reject promotion. Sync acceptance
cases prove the review queue can durably keep local state, guarded-apply a safe
remote edit, and resolve an unprovable account claim only through a separate
Provider and explicit rebind.

The release also requires equivalent switch coverage for the five built-in
adapters, preservation of existing Provider/Profile and usage behavior, and the
full existing repository verification gates. It also requires linked native
quick-switch and main-window confirmation fallback, Profile batch rollback,
takeover ownership, proxy lifecycle, source-attribution epochs, and
machine-local restore tests. Passing only the built-in Agent paths does not
satisfy this design.

## Completion Definition

The implementation is complete only when:

- built-in and Custom Agents both perform verified real configuration switches;
- Bound, Available, Effective, Switchability, and Current are represented
  separately;
- every `in_sync` managed Agent has exactly one verified current binding while a
  degraded state has none and retains only non-authoritative last-verified data;
- normal, drifted, missing, unmanaged, setup-required, and partial states are
  surfaced accurately;
- route health remains distinct from configuration synchronization;
- all current-binding mutation guards are enforced;
- multi-file rollback and startup recovery are proven;
- dynamic Custom Agent namespaces and cross-audience key rejection are proven;
- machine-local state cannot be imported as another machine's current state;
- Provider account instances and frozen claim provenance prevent cross-account
  usage, quota, or credential replacement from merging silently;
- custom and fixed API routes preserve split credentials and attribution;
- existing product features remain functional; and
- the first-release Custom Agent acceptance gate passes in an isolated
  environment.
