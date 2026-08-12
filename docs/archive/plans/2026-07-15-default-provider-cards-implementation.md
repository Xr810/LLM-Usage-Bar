# Fixed Default Provider Cards Implementation Plan

> **Execution:** Use `superpowers:executing-plans` and implement this plan directly,
> one task at a time. Do not delegate this work to Kimi: the repository rules
> explicitly prohibit delegating architecture, authentication/security, and
> database migrations.

**Goal:** Make five immutable system Provider cards always available, seed the
approved default Agent bindings once, use official subscription authentication,
and let the three fixed API Providers share one protected upstream Key while each
Agent binding uses its own protected local proxy Key.

**Architecture:** Schema v17 adds stable system Provider identity, binding-owned
local route protocol, and Provider-level credential metadata/journals. A static
catalog reconciles only canonical non-secret fields. The existing protected
credential service is extended instead of creating a second uncoordinated secret
lifecycle. Proxy resolution verifies a binding-local Key, then a Provider Key,
freezes both generations, and forwards only the Provider Key. ChatGPT reuses the
managed Codex OAuth path. Claude Pro/Max is status/session-only and invokes only
the official `claude auth` commands; it is never a proxy target.

**Tech stack:** Tauri 2, Rust, rusqlite, tokio, reqwest, React, TypeScript,
TanStack Query, Vitest, Testing Library, i18next.

**Approved design:**
[`docs/design/2026-07-15-default-provider-cards-design.md`](../../design/2026-07-15-default-provider-cards-design.md)

## Non-negotiable implementation decisions

- Stable Provider IDs are:
  `system-chatgpt-subscription`, `system-claude-subscription`,
  `system-openai-api`, `system-anthropic-api`, and `system-openrouter-api`.
- `usage_providers.system_preset_key` is nullable for custom Providers and unique
  for system Providers.
- `agent_provider_bindings.route_protocol` owns the local Agent namespace. The
  shared Provider owns the canonical upstream endpoint and wire format.
- Existing custom Provider bindings keep their current single binding-owned
  upstream secret semantics. Only the three fixed API Providers use split local
  and upstream secrets.
- Existing binding fingerprints keep their current fingerprint domain for upgrade
  compatibility. Provider fingerprints use a distinct domain.
- The fixed API route compatibility matrix for this release is:

  | Provider | Supported local Agent protocols |
  | --- | --- |
  | OpenAI API | `codex`, `opencode`, `openclaw`, `hermes` |
  | Anthropic API | `claude` |
  | OpenRouter | `claude`, `codex`, `opencode`, `openclaw`, `hermes` |

- The Claude subscription binding is observation-only. Its `route_protocol` is
  null and every proxy resolver rejects it before network I/O.
- OpenCode, OpenClaw, and Hermes use distinct local route prefixes even though
  their fixed OpenRouter Provider and upstream Key are shared.
- Do not add a Claude quota network path. If no official machine-readable CLI
  command exists, return `quotaAvailability: "unavailable"`.
- Preserve any unrelated dirty working-tree changes. Never reset or rewrite the
  user's existing Settings/dashboard edits while executing this plan.

## Task 1: Add schema v17 and the canonical system Provider catalog

**Files:**

- Create: `src-tauri/src/usage/system_providers.rs`
- Create: `src-tauri/src/usage/system_provider_migration.rs`
- Modify: `src-tauri/src/usage/mod.rs`
- Modify: `src-tauri/src/usage/domain.rs`
- Modify: `src-tauri/src/database/mod.rs`
- Modify: `src-tauri/src/database/schema.rs`
- Modify: `src-tauri/src/database/tests.rs`

### Step 1: Write failing migration tests

Add `migration_v16_to_v17` tests alongside `migration_v15_to_v16`. Reuse a v16
fixture created by running the existing v15→v16 migration. The primary test must
assert all of the following in one transactionally migrated database:

```rust
assert_eq!(Database::get_user_version(&conn).unwrap(), 17);

let system_rows = conn
    .prepare(
        "SELECT id, system_preset_key, name, billing_kind
         FROM usage_providers
         WHERE system_preset_key IS NOT NULL
         ORDER BY system_preset_key",
    )
    .unwrap()
    .query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();

assert_eq!(system_rows.len(), 5);
assert_eq!(
    conn.query_row(
        "SELECT COUNT(*) FROM agent_provider_bindings
         WHERE provider_id IN (
           'system-chatgpt-subscription',
           'system-claude-subscription',
           'system-openrouter-api'
         )",
        [],
        |row| row.get::<_, i64>(0),
    ).unwrap(),
    5,
);
assert_eq!(
    conn.query_row(
        "SELECT COUNT(*) FROM agent_provider_bindings
         WHERE provider_id IN ('system-openai-api','system-anthropic-api')",
        [],
        |row| row.get::<_, i64>(0),
    ).unwrap(),
    0,
);
```

Also assert:

- exact default pairs and `route_protocol` values:
  ChatGPT/Codex=`codex`, Claude/Claude Code=null,
  OpenRouter/OpenCode=`opencode`, OpenRouter/OpenClaw=`openclaw`, and
  OpenRouter/Hermes=`hermes`;
- three empty `provider_api_credentials` rows exist only for the API cards;
- custom Providers and existing bindings are byte-for-byte preserved except for
  the deterministic `route_protocol` backfill;
- a second catalog reconciliation creates no extra rows and does not restore a
  deleted default binding;
- duplicate system keys, changing a system key/ID, or deleting a system row fail;
- a forced migration failure rolls back columns, tables, rows, indexes, triggers,
  and `user_version`;
- a database claiming v17 but missing an index/trigger/table fails completeness
  validation.

Run:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml database::tests::migration_v16_to_v17 -- --nocapture
```

Expected: FAIL because schema v17 and its migration do not exist.

### Step 2: Define the catalog as the sole canonical metadata source

Create a typed catalog. Keep JSON construction in a function because
`serde_json::Value` is not const:

```rust
pub const CHATGPT_SUBSCRIPTION_ID: &str = "system-chatgpt-subscription";
pub const CLAUDE_SUBSCRIPTION_ID: &str = "system-claude-subscription";
pub const OPENAI_API_ID: &str = "system-openai-api";
pub const ANTHROPIC_API_ID: &str = "system-anthropic-api";
pub const OPENROUTER_API_ID: &str = "system-openrouter-api";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemProviderAuthKind {
    CodexOauth,
    ClaudeCli,
    ProviderApiKey,
}

pub struct SystemProviderDefinition {
    pub id: &'static str,
    pub preset_key: &'static str,
    pub name: &'static str,
    pub billing_kind: BillingKind,
    pub product_group_id: &'static str,
    pub token_sources: &'static [TokenSource],
    pub auth_kind: SystemProviderAuthKind,
    pub upstream_protocol: Option<&'static str>,
    pub route_config: Option<serde_json::Value>,
}

pub fn system_provider_definitions() -> Vec<SystemProviderDefinition> {
    vec![
        SystemProviderDefinition {
            id: CHATGPT_SUBSCRIPTION_ID,
            preset_key: "chatgpt-subscription",
            name: "ChatGPT Plus/Pro",
            billing_kind: BillingKind::Subscription,
            product_group_id: "chatgpt-subscription",
            token_sources: &[TokenSource::Proxy, TokenSource::SessionLog],
            auth_kind: SystemProviderAuthKind::CodexOauth,
            upstream_protocol: Some("codex"),
            route_config: None,
        },
        SystemProviderDefinition {
            id: CLAUDE_SUBSCRIPTION_ID,
            preset_key: "claude-subscription",
            name: "Claude Pro/Max",
            billing_kind: BillingKind::Subscription,
            product_group_id: "claude-subscription",
            token_sources: &[TokenSource::SessionLog],
            auth_kind: SystemProviderAuthKind::ClaudeCli,
            upstream_protocol: None,
            route_config: None,
        },
        SystemProviderDefinition {
            id: OPENAI_API_ID,
            preset_key: "openai-api",
            name: "OpenAI API",
            billing_kind: BillingKind::Metered,
            product_group_id: "openai-api",
            token_sources: &[TokenSource::Proxy],
            auth_kind: SystemProviderAuthKind::ProviderApiKey,
            upstream_protocol: Some("codex"),
            route_config: Some(serde_json::json!({
                "base_url": "https://api.openai.com/v1",
                "apiFormat": "openai_chat",
                "authMode": "bearer"
            })),
        },
        SystemProviderDefinition {
            id: ANTHROPIC_API_ID,
            preset_key: "anthropic-api",
            name: "Anthropic API",
            billing_kind: BillingKind::Metered,
            product_group_id: "anthropic-api",
            token_sources: &[TokenSource::Proxy],
            auth_kind: SystemProviderAuthKind::ProviderApiKey,
            upstream_protocol: Some("claude"),
            route_config: Some(serde_json::json!({
                "base_url": "https://api.anthropic.com",
                "apiFormat": "anthropic",
                "authMode": "x_api_key"
            })),
        },
        SystemProviderDefinition {
            id: OPENROUTER_API_ID,
            preset_key: "openrouter-api",
            name: "OpenRouter",
            billing_kind: BillingKind::Metered,
            product_group_id: "openrouter-api",
            token_sources: &[TokenSource::Proxy],
            auth_kind: SystemProviderAuthKind::ProviderApiKey,
            upstream_protocol: Some("codex"),
            route_config: Some(serde_json::json!({
                "base_url": "https://openrouter.ai/api/v1",
                "apiFormat": "openai_chat",
                "authMode": "bearer"
            })),
        },
    ]
}
```

The subscription definitions contain no raw authentication material. Claude has
no route config. ChatGPT records the existing managed Codex route marker, not an
OAuth token.

### Step 3: Implement the additive migration and exact validator

`migrate_v16_to_v17` must:

1. add `usage_providers.system_preset_key TEXT`;
2. add `agent_provider_bindings.route_protocol TEXT`;
3. backfill existing bindings from their Provider's current `route_app_type`;
4. create a unique partial index on non-null system keys;
5. create identity/delete protection triggers for system rows;
6. create `provider_api_credentials` and `provider_credential_operations`;
7. create unique partial indexes for Provider credential fingerprint and slot;
8. upsert the five canonical rows;
9. insert exactly five default bindings and set `route_protocol` as approved;
10. set `settings.system_provider_default_bindings_v1_seeded = true` in the same
    transaction;
11. set `codex` and `claude` usage-source bindings only when those sources have no
    existing owner; and
12. validate the complete v17 schema before returning.

The Provider credential table shape is:

```sql
CREATE TABLE provider_api_credentials (
    provider_id TEXT NOT NULL PRIMARY KEY
        REFERENCES usage_providers(id) ON DELETE RESTRICT,
    api_key_fingerprint BLOB,
    credential_slot TEXT,
    credential_version INTEGER NOT NULL DEFAULT 0 CHECK (credential_version >= 0),
    last_test_at INTEGER,
    last_test_status TEXT CHECK (
        last_test_status IS NULL OR last_test_status IN ('success','failed')
    ),
    last_test_error_code TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK (
        (api_key_fingerprint IS NULL AND credential_slot IS NULL)
        OR
        (api_key_fingerprint IS NOT NULL
         AND typeof(api_key_fingerprint) = 'blob'
         AND length(api_key_fingerprint) = 32
         AND credential_slot IS NOT NULL
         AND length(trim(credential_slot)) > 0
         AND credential_version > 0)
    )
);
```

Mirror the existing binding journal shape with `provider_id`, generations, and
`set|replace|clear` operations. Do not put connection-test response bodies in SQL.

### Step 4: Wire schema versioning and prove migration continuity

- Change `SCHEMA_VERSION` to 17.
- Add `16 => migrate_v16_to_v17` and v17 completeness validation.
- Register both new modules.
- Add a v12→v17 continuous migration test.

Run the focused test again; expected PASS.

### Step 5: Commit

```bash
git add src-tauri/src/usage/system_providers.rs \
  src-tauri/src/usage/system_provider_migration.rs \
  src-tauri/src/usage/mod.rs src-tauri/src/usage/domain.rs \
  src-tauri/src/database/mod.rs \
  src-tauri/src/database/schema.rs src-tauri/src/database/tests.rs
git commit -m "feat: add fixed system provider schema"
```

## Task 2: Enforce system Provider immutability and startup reconciliation

**Files:**

- Modify: `src-tauri/src/usage/system_providers.rs`
- Modify: `src-tauri/src/database/dao/usage_providers.rs`
- Modify: `src-tauri/src/usage/domain.rs`
- Modify: `src-tauri/src/database/backup.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/database/tests.rs`

### Step 1: Write failing DAO and reconciliation tests

Cover:

- system cards sort in catalog order before custom Providers;
- `save_usage_provider` rejects editing any system row with
  `system_provider_immutable`;
- `set_usage_provider_enabled` remains allowed;
- internal reconciliation repairs a missing card and stale endpoint/name without
  changing enabled state, bindings, credential metadata, timestamps owned by the
  user, or connection-test state;
- reconciliation never recreates a removed default binding;
- SQL import/sync cannot delete or replace protected system identity;
- custom Provider behavior remains unchanged.

Run:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml database::dao::usage_providers -- --nocapture
```

Expected: FAIL on missing system metadata and reconciliation APIs.

### Step 2: Add explicit public DTO fields

Extend the domain without exposing route JSON or credentials. Reuse the
`SystemProviderAuthKind` enum added in Task 1 and add these fields to
`UsageProviderView`:

```rust
pub system_preset_key: Option<String>,
pub system_auth_kind: Option<SystemProviderAuthKind>,
pub canonical_endpoint: Option<String>,
pub compatible_agent_module_ids: Vec<String>,
pub upstream_credential_status: BindingCredentialStatus,
pub upstream_credential_version: u64,
pub can_clear_upstream_credential: bool,
pub last_connection_test_at: Option<i64>,
pub last_connection_test_status: Option<String>,
```

`UsageProviderInput` gets no system-key field. That prevents ordinary callers from
creating or promoting a system Provider.

### Step 3: Separate public mutation from internal reconciliation

Implement:

```rust
impl Database {
    pub(crate) fn reconcile_system_providers(&self) -> Result<(), AppError>;
    pub(crate) fn is_system_provider(&self, provider_id: &str) -> Result<bool, AppError>;
}
```

The public save path rejects an existing system row before comparing input fields.
The internal reconciler uses the typed catalog and updates only canonical
non-secret columns. It must not call the public save API.

Run reconciliation after migrations and before proxy/session background work is
started in `lib.rs`. A reconciliation error aborts startup instead of launching
with a partial catalog.

### Step 4: Protect import and sync transactions

Extend `database/backup.rs` protected snapshots to include:

- all five system Provider identities and canonical fields;
- `provider_api_credentials` metadata; and
- the one-time default-binding seed marker.

Skip `provider_credential_operations` during sync, exactly as the existing binding
journal is skipped. Reconcile canonical cards inside the import transaction before
commit, then restore local credential slots/fingerprints/versions. Reject any
incoming payload that tries to turn a custom row into a system row.

Run:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml database::backup::tests -- --nocapture
```

Expected: PASS, including sentinel assertions proving SQL/export content has no raw
secret.

### Step 5: Commit

```bash
git add src-tauri/src/usage/system_providers.rs \
  src-tauri/src/database/dao/usage_providers.rs \
  src-tauri/src/usage/domain.rs src-tauri/src/database/backup.rs \
  src-tauri/src/lib.rs src-tauri/src/database/tests.rs
git commit -m "feat: reconcile immutable system providers"
```

## Task 3: Add Provider-level protected API Key lifecycle

**Files:**

- Create: `src-tauri/src/database/dao/provider_credentials.rs`
- Modify: `src-tauri/src/database/dao/mod.rs`
- Modify: `src-tauri/src/database/mod.rs`
- Modify: `src-tauri/src/credentials/service.rs`
- Modify: `src-tauri/src/credentials/tests.rs`
- Modify: `src-tauri/src/store.rs`

### Step 1: Write failing credential lifecycle tests

Use the existing injected in-memory `CredentialStore`. Add tests for:

- set, replace, clear, conflict, store failure, DB publish failure, and startup
  journal reconciliation;
- only the three fixed API Provider IDs are accepted;
- list/status DTOs never contain raw Keys;
- a Provider Key cannot authenticate a local binding because its fingerprint uses
  a separate domain;
- Provider rotation does not change any binding local credential generation;
- clearing preserves bindings but makes them ineffective;
- concurrent replace/clear has one winner and no mixed generation;
- backup/export/diagnostic serialization does not contain unique sentinel Keys.

Run:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml credentials::tests::provider_credential -- --nocapture
```

Expected: FAIL because Provider operations do not exist.

### Step 2: Add domain-separated fingerprints and slots

Do not modify the existing binding fingerprint algorithm. Add:

```rust
const PROVIDER_FINGERPRINT_DOMAIN: &[u8] =
    b"com.xr810.llm-usage-bar.provider-upstream.v1\0";

fn provider_credential_fingerprint(secret: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(PROVIDER_FINGERPRINT_DOMAIN);
    hasher.update(secret);
    hasher.finalize().into()
}

fn provider_staging_slot(provider_id: &str, generation: u64) -> String {
    format!("provider/{provider_id}/{generation}/{}", uuid::Uuid::new_v4())
}
```

Provider Keys remain `SecretString` inputs and non-serializable
`Zeroizing<Vec<u8>>` internally.

### Step 3: Mirror the proven staged publish protocol

Add DAO reservation/snapshot/journal types for Providers and extend the existing
`BindingCredentialService` with:

```rust
pub async fn set_provider_api_key(
    &self,
    provider_id: &str,
    expected_version: u64,
    api_key: SecretString,
) -> Result<UsageProviderView, AppError>;

pub async fn replace_provider_api_key(
    &self,
    provider_id: &str,
    expected_version: u64,
    api_key: SecretString,
) -> Result<UsageProviderView, AppError>;

pub async fn clear_provider_api_key(
    &self,
    provider_id: &str,
    expected_version: u64,
) -> Result<UsageProviderView, AppError>;
```

Keep the existing service name to avoid a repository-wide rename. All Provider and
binding operations must use the same `CredentialLifecycleLock`, so backup/import,
rotation, delete, and startup reconciliation cannot interleave unsafely.

### Step 4: Reconcile both journals on startup

`reconcile_startup()` must finish or clean both binding and Provider journal
entries while holding the exclusive lifecycle lock. Orphan detection must never
delete a slot referenced by either active table.

Run the focused tests again; expected PASS.

### Step 5: Commit

```bash
git add src-tauri/src/database/dao/provider_credentials.rs \
  src-tauri/src/database/dao/mod.rs src-tauri/src/database/mod.rs \
  src-tauri/src/credentials/service.rs src-tauri/src/credentials/tests.rs \
  src-tauri/src/store.rs
git commit -m "feat: protect shared provider api keys"
```

## Task 4: Generate, reveal, and rotate binding-local proxy Keys

**Files:**

- Modify: `src-tauri/src/database/dao/agent_provider_bindings.rs`
- Modify: `src-tauri/src/database/dao/binding_credentials.rs`
- Modify: `src-tauri/src/credentials/service.rs`
- Modify: `src-tauri/src/credentials/tests.rs`
- Modify: `src-tauri/src/usage/domain.rs`

### Step 1: Write failing split-secret binding tests

Test:

- adding a fixed API binding generates a distinct high-entropy local Key;
- two OpenRouter bindings have different local Keys but the same Provider
  credential version;
- only an explicit single-binding reveal returns a raw local Key;
- list/setup/status calls never return it;
- rotation invalidates the old local Key immediately;
- removing and re-adding creates a new binding ID and Key;
- custom Providers still accept the user-supplied binding API Key and forward that
  same secret under the old semantics;
- unsupported Provider/Agent pairs are rejected with `invalid_binding`;
- the Claude subscription binding cannot receive or reveal a local Key.

Run:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml credentials::tests::local_binding -- --nocapture
```

Expected: FAIL on missing generate/reveal/rotate APIs.

### Step 2: Derive binding route protocol server-side

Never accept `routeProtocol` from the frontend. Add a compatibility function:

```rust
pub(crate) fn system_binding_route_protocol(
    preset_key: &str,
    agent_module_id: &str,
) -> Option<Option<&'static str>> {
    match (preset_key, agent_module_id) {
        ("chatgpt-subscription", "codex") => Some(Some("codex")),
        ("claude-subscription", "claude-code") => Some(None),
        ("openai-api", "codex") => Some(Some("codex")),
        ("openai-api", "opencode") => Some(Some("opencode")),
        ("openai-api", "openclaw") => Some(Some("openclaw")),
        ("openai-api", "hermes") => Some(Some("hermes")),
        ("anthropic-api", "claude-code") => Some(Some("claude")),
        ("openrouter-api", "claude-code") => Some(Some("claude")),
        ("openrouter-api", "codex") => Some(Some("codex")),
        ("openrouter-api", "opencode") => Some(Some("opencode")),
        ("openrouter-api", "openclaw") => Some(Some("openclaw")),
        ("openrouter-api", "hermes") => Some(Some("hermes")),
        _ => None,
    }
}
```

For custom Providers, preserve current behavior by copying the Provider's
`route_app_type` into the binding at creation/update.

Extend `default_direct_placement` so `opencode`, `openclaw`, and `hermes` use
`AuthorizationBearer`. Route config `apiFormat` remains authoritative for the
upstream wire protocol and credential placement.

### Step 3: Generate local Keys only in the protected service

Use two UUID v4 values to provide 256 random bits without adding a dependency:

```rust
fn generate_local_binding_key() -> SecretString {
    SecretString::new(format!(
        "lub_{}_{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple(),
    ))
}
```

Add:

```rust
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBindingKeyReveal {
    pub binding_id: String,
    pub credential_version: u64,
    pub local_key: String,
}
```

Construct this DTO only at the explicit reveal boundary. Give it a redacted
`Debug` implementation, never clone it, and implement `Drop` with
`self.local_key.zeroize()`. Do not add any bulk reveal API.

Service methods:

```rust
pub async fn create_system_api_binding(
    &self,
    input: AgentProviderBindingInput,
) -> Result<AgentProviderBindingView, AppError>;
pub async fn reveal_local_binding_key(
    &self,
    binding_id: &str,
    expected_version: u64,
) -> Result<LocalBindingKeyReveal, AppError>;
pub async fn rotate_local_binding_key(
    &self,
    binding_id: &str,
    expected_version: u64,
) -> Result<LocalBindingKeyReveal, AppError>;
```

The create operation must be compensating/atomic across SQLite and protected
storage: if protected storage fails, leave a visible disabled binding with
`unavailable` status or delete the newly reserved binding before returning; never
return enabled without a verified Key.

After startup journal reconciliation and before starting the proxy, call
`ensure_fixed_api_binding_local_keys()`. It retries generation for existing fixed
API bindings that have no local credential, including the three migration-seeded
OpenRouter defaults. If protected storage is unavailable, it preserves each
requested-enabled binding and reports `unavailable`; it does not recreate deleted
bindings or change Agent selections.

### Step 4: Expose separate status fields

Extend `AgentProviderBindingView` with `route_protocol`,
`local_credential_status`, and `provider_credential_status`. Keep the legacy
`credential_status` field for custom Provider compatibility during this release.
`effective_enabled` for a fixed API binding is true only when requested-enabled,
Provider enabled, Agent active, local Key configured, Provider Key configured, and
the pair is compatible.

Run focused tests again; expected PASS.

### Step 5: Commit

```bash
git add src-tauri/src/database/dao/agent_provider_bindings.rs \
  src-tauri/src/database/dao/binding_credentials.rs \
  src-tauri/src/credentials/service.rs src-tauri/src/credentials/tests.rs \
  src-tauri/src/usage/domain.rs
git commit -m "feat: generate per-agent proxy keys"
```

## Task 5: Resolve split credentials atomically and add Agent route namespaces

**Files:**

- Modify: `src-tauri/src/credentials/service.rs`
- Modify: `src-tauri/src/credentials/tests.rs`
- Modify: `src-tauri/src/proxy/handler_context.rs`
- Modify: `src-tauri/src/proxy/forwarder.rs`
- Modify: `src-tauri/src/proxy/server.rs`
- Modify: `src-tauri/src/proxy/handlers.rs`
- Modify: `src-tauri/src/proxy/provider_router.rs`
- Modify: `src-tauri/src/proxy/response_processor.rs`
- Modify: `src-tauri/src/proxy/response_handler.rs`

### Step 1: Write failing resolver and no-upstream-call tests

Use unique local/upstream sentinel values. Add tests proving:

- OpenCode, OpenClaw, and Hermes each resolve through only their own namespace;
- the three local Keys select the same OpenRouter Provider but freeze different
  Agent/binding IDs;
- a local Key used on another Agent prefix fails before request-body parsing and
  before network I/O;
- the upstream request contains the Provider Key and contains no local Key in URL,
  headers, body, error, logs, usage identifiers, or streamed response state;
- Provider Key rotation and local Key rotation each invalidate only the intended
  generation;
- disabling Provider or binding before the second authoritative read yields zero
  upstream calls;
- a request that passed the second read keeps its frozen generations;
- the Claude subscription system Provider always yields an unsupported local route
  with zero upstream calls;
- custom Provider direct bindings retain their current request bytes and auth
  placement.

Run:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml proxy::handler_context::tests -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml proxy::handlers::tests -- --nocapture
```

Expected: FAIL because the resolver currently treats the binding Key as the
upstream Key and the three namespaces do not exist.

### Step 2: Refactor the resolved credential without making it serializable

Replace the ambiguous `secret` field with explicit local/upstream material:

```rust
pub struct ResolvedBindingCredential {
    ownership: FrozenBindingOwnership,
    route_protocol: String,
    product_group_id: String,
    runtime_provider: Provider,
    credential_placement: UpstreamCredentialPlacement,
    local_credential_version: u64,
    upstream_credential_version: u64,
    legacy_pricing_provider_id: Option<String>,
    pricing_override: BindingPricingOverride,
    local_secret: Zeroizing<Vec<u8>>,
    upstream_secret: Zeroizing<Vec<u8>>,
}

impl ResolvedBindingCredential {
    pub(crate) fn expose_upstream_secret(&self) -> &[u8] {
        self.upstream_secret.as_slice()
    }
}
```

For custom Providers, populate both fields from the verified legacy binding secret
so behavior is unchanged. For fixed API Providers, local and upstream secrets come
from different protected slots and use different fingerprints.

Resolution order is mandatory:

1. fingerprint inbound local Key;
2. read binding + Provider credential snapshot;
3. verify local protected item in constant time;
4. verify upstream protected item and its Provider-domain fingerprint;
5. re-read binding and Provider credential generations together;
6. reject any changed ID, slot, generation, enabled state, protocol, system kind,
   or Agent archive state;
7. construct the credential-free runtime Provider; and
8. return the frozen non-serializable object.

The second SQL read is the pre-send linearization point. Do not hold the database
mutex, lifecycle lock, or protected-store operation across network I/O.

### Step 3: Guard both secrets at every egress sink

The current `CredentialExposureGuard` protects one secret. Introduce a small set
wrapper and use it everywhere downstream:

```rust
#[derive(Clone)]
pub(crate) struct CredentialExposureGuardSet {
    guards: Vec<CredentialExposureGuard>,
}

impl CredentialExposureGuardSet {
    pub(crate) fn from_secrets(secrets: &[&[u8]]) -> Self {
        Self {
            guards: secrets
                .iter()
                .map(|secret| CredentialExposureGuard::from_secret(secret))
                .collect(),
        }
    }

    pub(crate) fn contains(&self, value: &str) -> bool {
        self.guards.iter().any(|guard| guard.contains(value))
    }
}
```

Add equivalent JSON, byte, normalization, and streaming scanner fan-out methods.
`RequestContext`, `Forwarder`, response processing, logging, session extraction,
and upstream error handling must all receive the set. Keep each underlying secret
prefix-safe across chunk boundaries.

### Step 4: Make local protocol binding-owned

Update snapshots and route projection to use `binding.route_protocol`. Do not
compare the requested namespace to `provider.route_app_type` for system Providers.
Keep Provider `route_app_type` as canonical upstream adapter metadata and preserve
the old equality behavior for custom Providers through their backfilled binding
protocol.

Rename internal accessors from `route_app_type()` to `route_protocol()` where they
refer to the inbound Agent namespace. Frozen usage context must record the binding
protocol.

### Step 5: Add prefixed OpenAI-compatible routes for three Agents

Refactor the existing Codex handlers into helpers that take `AppType`, tag,
protocol, and strip prefix. Register:

```rust
.route("/opencode/v1/chat/completions", post(handlers::handle_opencode_chat))
.route("/opencode/v1/models", get(handlers::handle_opencode_models))
.route("/opencode/v1/responses", post(handlers::handle_opencode_responses))
.route("/openclaw/v1/chat/completions", post(handlers::handle_openclaw_chat))
.route("/openclaw/v1/models", get(handlers::handle_openclaw_models))
.route("/openclaw/v1/responses", post(handlers::handle_openclaw_responses))
.route("/hermes/v1/chat/completions", post(handlers::handle_hermes_chat))
.route("/hermes/v1/models", get(handlers::handle_hermes_models))
.route("/hermes/v1/responses", post(handlers::handle_hermes_responses))
```

Also register each `/responses/compact` path. The helpers must pass the real
`AppType::{OpenCode,OpenClaw,Hermes}` and protocol string into request context;
the existing adapter fallback to `CodexAdapter` supplies the OpenAI-compatible
wire behavior.

`get_agent_proxy_setup_info` returns these exact bases:

```text
http://127.0.0.1:<port>/opencode/v1
http://127.0.0.1:<port>/openclaw/v1
http://127.0.0.1:<port>/hermes/v1
```

Run focused resolver, forwarder, handler, and response processor tests. Expected:
PASS.

### Step 6: Commit

```bash
git add src-tauri/src/credentials/service.rs src-tauri/src/credentials/tests.rs \
  src-tauri/src/proxy/handler_context.rs src-tauri/src/proxy/forwarder.rs \
  src-tauri/src/proxy/server.rs src-tauri/src/proxy/handlers.rs \
  src-tauri/src/proxy/provider_router.rs \
  src-tauri/src/proxy/response_processor.rs \
  src-tauri/src/proxy/response_handler.rs
git commit -m "feat: route agents with split credentials"
```

## Task 6: Add compliant Claude CLI authentication status/actions

**Files:**

- Create: `src-tauri/src/services/claude_cli_auth.rs`
- Create: `src-tauri/src/commands/claude_cli_auth.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/commands/misc.rs`
- Modify: `src-tauri/src/store.rs`
- Modify: `src-tauri/src/lib.rs`

### Step 1: Write failing tests with an injected command runner

Cover:

- CLI absent, authenticated, unauthenticated, timeout, malformed JSON, and nonzero
  failure states;
- exit 0 plus valid JSON maps to authenticated; exit 1 maps to disconnected;
- login launches exactly `claude auth login` in a visible terminal;
- logout launches exactly `claude auth logout` and refreshes status;
- no command contains `setup-token`, no output returns an OAuth token, and no code
  path opens `.credentials.json`, `~/.claude.json`, or Keychain;
- unknown JSON fields are ignored and never copied into the DTO;
- quota is always explicitly unavailable.

Run:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml services::claude_cli_auth::tests -- --nocapture
```

Expected: FAIL because the service does not exist.

### Step 2: Define a narrow command-runner boundary

```rust
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeSubscriptionType {
    Pro,
    Max,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCliAuthStatus {
    pub installed: bool,
    pub authenticated: bool,
    pub subscription_type: Option<ClaudeSubscriptionType>,
    pub quota_availability: &'static str,
    pub error_code: Option<String>,
}

pub struct ClaudeAuthCommandOutput {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
}

#[async_trait::async_trait]
pub trait ClaudeAuthCommandRunner: Send + Sync {
    async fn status(&self) -> Result<ClaudeAuthCommandOutput, AppError>;
    fn launch_login(&self) -> Result<(), AppError>;
    async fn logout(&self) -> Result<ClaudeAuthCommandOutput, AppError>;
}
```

Production status executes `claude auth status` with a short timeout. The official
CLI documents JSON output and exit 0/1 semantics. Treat exit code as authoritative,
require stdout to be a JSON object, and read only a boolean `loggedIn` plus an
optional allowlisted `subscriptionType` of `pro|max`. If fields are absent but exit
code is 0, authenticated remains true and subscription type remains null.

Login calls the existing trusted `launch_terminal_running("claude auth login",
"claude_auth_login")`. Logout runs the exact fixed argument vector
`["auth", "logout"]`; no user text reaches a shell.

### Step 3: Add Tauri commands and injectable state

Add:

```rust
#[tauri::command]
pub async fn get_claude_cli_auth_status(
    state: State<'_, AppState>,
) -> Result<ClaudeCliAuthStatus, AppError>;

#[tauri::command]
pub fn start_claude_cli_login(state: State<'_, AppState>) -> Result<(), AppError>;

#[tauri::command]
pub async fn logout_claude_cli(
    state: State<'_, AppState>,
) -> Result<ClaudeCliAuthStatus, AppError>;
```

Store `Arc<ClaudeCliAuthService>` in `AppState` and add a test constructor that
accepts an injected runner. Register all commands in `lib.rs`.

Run focused tests again; expected PASS.

### Step 4: Commit

```bash
git add src-tauri/src/services/claude_cli_auth.rs \
  src-tauri/src/commands/claude_cli_auth.rs src-tauri/src/services/mod.rs \
  src-tauri/src/commands/mod.rs src-tauri/src/commands/misc.rs \
  src-tauri/src/store.rs src-tauri/src/lib.rs
git commit -m "feat: expose official claude cli auth status"
```

## Task 7: Add system Provider commands, connection tests, and frontend API hooks

**Files:**

- Create: `src-tauri/src/services/system_provider_connection.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Modify: `src-tauri/src/commands/usage_dashboard.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/types/usageDashboard.ts`
- Modify: `src/lib/api/usageDashboard.ts`
- Modify: `src/lib/query/usageDashboard.ts`
- Create: `src/lib/api/claudeCliAuth.ts`

### Step 1: Write failing Rust command-contract tests

Add tests for:

- Provider Key set/replace/clear with optimistic versions;
- fixed API binding creation auto-generates a local Key;
- explicit reveal/rotate is single-binding and version-checked;
- all ordinary command JSON excludes both local and upstream sentinel values;
- connection test uses only canonical endpoint and correct header placement;
- OpenAI/OpenRouter call `GET /models` with bearer auth;
- Anthropic calls `GET /v1/models` with `x-api-key` and a fixed supported
  `anthropic-version` header;
- HTTP bodies and upstream error text are collapsed to public error codes before
  persistence or return;
- ChatGPT status comes from the existing `codex_oauth` account API;
- Claude status comes only from the new CLI service;
- requested/effective binding state is recomputed after auth disconnect.

Run:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml commands::usage_dashboard::tests -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml services::system_provider_connection::tests -- --nocapture
```

Expected: FAIL on missing commands and service.

### Step 2: Implement command boundaries

Add Tauri commands and corresponding test hooks:

```rust
set_system_provider_api_key(provider_id, expected_version, api_key)
replace_system_provider_api_key(provider_id, expected_version, api_key)
clear_system_provider_api_key(provider_id, expected_version)
test_system_provider_connection(provider_id, expected_version)
reveal_agent_provider_local_key(binding_id, expected_version)
rotate_agent_provider_local_key(binding_id, expected_version)
```

Keep the existing binding API Key commands for custom Providers. Those commands
must reject fixed API bindings so a caller cannot overwrite a generated local Key
with an upstream Key.

The reveal/rotate commands are the only commands allowed to serialize a local Key.
Do not log arguments/results and do not wrap them with generic diagnostics. The
connection tester receives a non-serializable resolved Provider credential and
drops it before returning its redacted status DTO.

For list commands, build a transient `SystemProviderAuthSnapshot` from the existing
`CodexOAuthState` account status and `AppState.claude_cli_auth_service`. Pass that
snapshot into the Provider/binding projection layer so subscription bindings report
effective only while their official authentication is currently available. Do not
persist this snapshot. Fixed API effective state comes from the verified Provider
and local protected-store statuses. The Tauri list commands may accept both managed
states; their test hooks accept an injected snapshot so command tests never need
real accounts.

### Step 3: Add TypeScript contracts

```ts
export type SystemProviderAuthKind =
  | "codex_oauth"
  | "claude_cli"
  | "provider_api_key";

export interface LocalBindingKeyReveal {
  bindingId: string;
  credentialVersion: number;
  localKey: string;
}

export interface ClaudeCliAuthStatus {
  installed: boolean;
  authenticated: boolean;
  subscriptionType: "pro" | "max" | null;
  quotaAvailability: "unavailable";
  errorCode: string | null;
}
```

Extend Provider/binding interfaces with the exact fields added in Tasks 2 and 4.
Add query keys for Provider credential actions and Claude auth status. All
mutations invalidate Provider, binding, proxy-setup, and dashboard roots.

The reveal/rotate API helper returns the value to its immediate caller but no
TanStack Query cache stores it.

### Step 4: Run contract checks

```bash
pnpm typecheck
pnpm test:unit src/lib/query/usageDashboard.test.tsx
```

Expected: PASS.

### Step 5: Commit

```bash
git add src-tauri/src/services/system_provider_connection.rs \
  src-tauri/src/services/mod.rs src-tauri/src/commands/usage_dashboard.rs \
  src-tauri/src/lib.rs src/types/usageDashboard.ts \
  src/lib/api/usageDashboard.ts src/lib/query/usageDashboard.ts \
  src/lib/api/claudeCliAuth.ts
git commit -m "feat: expose system provider actions"
```

## Task 8: Build the five fixed Provider cards and editable bindings UI

**Files:**

- Create: `src/components/settings/SystemProviderCard.tsx`
- Create: `src/components/settings/SystemProviderCard.test.tsx`
- Create: `src/components/settings/SystemProviderApiKeyDialog.tsx`
- Create: `src/components/settings/SystemProviderApiKeyDialog.test.tsx`
- Create: `src/components/settings/ClaudeCliAuthSection.tsx`
- Create: `src/components/settings/ClaudeCliAuthSection.test.tsx`
- Create: `src/components/settings/SystemProviderAgentBindings.tsx`
- Create: `src/components/settings/SystemProviderAgentBindings.test.tsx`
- Modify: `src/components/settings/UsageProvidersSettings.tsx`
- Modify: `src/components/settings/UsageProvidersSettings.test.tsx`
- Modify: `src/components/providers/forms/CodexOAuthSection.tsx`
- Modify: `src/i18n/locales/en.json`
- Modify: `src/i18n/locales/zh.json`
- Modify: `src/i18n/locales/zh-TW.json`
- Modify: `src/i18n/locales/ja.json`

### Step 1: Write failing component tests before components

The tests must assert:

- exactly five system cards render first in canonical order;
- custom Providers render after them and retain Edit/Enable actions;
- system cards have no Edit/Delete action;
- ChatGPT renders managed login/account/reconnect/logout controls;
- Claude renders official CLI login/status/logout and “quota unavailable”;
- API cards show locked canonical endpoints and Key save/replace/clear/test;
- upstream Keys are cleared from component state after submission and never
  re-rendered;
- default Agent selections are exactly Codex, Claude Code, and the three approved
  OpenRouter Agents;
- changing a selection creates/deletes only that binding and survives refetch;
- disconnected Provider retains selected bindings but shows them ineffective;
- local Key Copy calls reveal once, writes directly to clipboard, and keeps only a
  boolean copied flag in React state;
- Rotate requires confirmation, copies the new Key once, and invalidates the old
  version;
- missing/unavailable/expired/disabled states have different accessible labels;
- Add Provider still opens the custom dialog.

Run:

```bash
pnpm test:unit src/components/settings/UsageProvidersSettings.test.tsx
pnpm test:unit src/components/settings/SystemProviderCard.test.tsx
pnpm test:unit src/components/settings/SystemProviderApiKeyDialog.test.tsx
pnpm test:unit src/components/settings/ClaudeCliAuthSection.test.tsx
pnpm test:unit src/components/settings/SystemProviderAgentBindings.test.tsx
```

Expected: FAIL because the new components do not exist.

### Step 2: Implement card composition, not a second Provider page

`UsageProvidersSettings` partitions by `systemPresetKey` and renders system cards
first. Keep the existing custom Provider list and `UsageProviderDialog` below a
“Custom Providers” label.

`SystemProviderCard` receives a complete Provider DTO and composes one auth body:

```tsx
switch (provider.systemAuthKind) {
  case "codex_oauth":
    return <CodexOAuthSection variant="system-card" />;
  case "claude_cli":
    return <ClaudeCliAuthSection />;
  case "provider_api_key":
    return <SystemProviderApiKeyControls provider={provider} />;
}
```

Add the `system-card` presentation variant to the existing Codex component; do not
fork its OAuth logic or create another token store.

### Step 3: Implement secret-minimizing dialogs

The upstream Key dialog keeps the text only in a local input state. In a `finally`
block set it to the empty string before closing. It never accepts an initial Key
value.

The local Copy/Rotate actions follow this exact pattern:

```tsx
const copyLocalKey = async () => {
  const reveal = await usageDashboardApi.revealAgentProviderLocalKey(
    binding.id,
    binding.credentialVersion,
  );
  try {
    await copyText(reveal.localKey);
    setCopied(true);
  } finally {
    reveal.localKey = "";
  }
};
```

Do not put `reveal` in component state, mutation data, toast text, error objects, or
test snapshots.

### Step 4: Implement Agent multi-select semantics

Load the fixed Agent list and current bindings. Disable incompatible pairs based on
the server-provided compatibility result; do not duplicate the matrix as a mutable
frontend authority. Selecting an API Agent invokes the create path that generates
its local Key. Deselecting invokes versioned delete. Login/Key state changes never
silently alter selections.

### Step 5: Add complete translations and accessibility labels

Add the same key structure to English, Simplified Chinese, Traditional Chinese,
and Japanese. Required user-facing states include Connected, Disconnected,
Expired, Missing Key, Credential unavailable, Disabled, Effective, Requested,
Quota unavailable, Copy local Key, Rotate local Key, and Test connection.

Run all five focused component test files, then:

```bash
pnpm typecheck
pnpm format:check
```

Expected: PASS.

### Step 6: Commit

```bash
git add src/components/settings/SystemProviderCard.tsx \
  src/components/settings/SystemProviderCard.test.tsx \
  src/components/settings/SystemProviderApiKeyDialog.tsx \
  src/components/settings/SystemProviderApiKeyDialog.test.tsx \
  src/components/settings/ClaudeCliAuthSection.tsx \
  src/components/settings/ClaudeCliAuthSection.test.tsx \
  src/components/settings/SystemProviderAgentBindings.tsx \
  src/components/settings/SystemProviderAgentBindings.test.tsx \
  src/components/settings/UsageProvidersSettings.tsx \
  src/components/settings/UsageProvidersSettings.test.tsx \
  src/components/providers/forms/CodexOAuthSection.tsx \
  src/i18n/locales/en.json src/i18n/locales/zh.json \
  src/i18n/locales/zh-TW.json src/i18n/locales/ja.json
git commit -m "feat: render fixed provider cards"
```

## Task 9: Add end-to-end attribution and secret-leak regression coverage

**Files:**

- Modify: `src-tauri/src/commands/usage_dashboard.rs`
- Modify: `src-tauri/src/credentials/tests.rs`
- Modify: `src-tauri/src/proxy/handler_context.rs`
- Modify: `src-tauri/src/proxy/forwarder.rs`
- Modify: `src-tauri/src/proxy/response_processor.rs`
- Modify: `src-tauri/src/database/backup.rs`
- Modify: `src/components/settings/UsageProvidersSettings.test.tsx`
- Modify: `tests/integration/SettingsDialog.test.tsx`

### Step 1: Add one full shared-OpenRouter scenario

Build a test fixture with one protected OpenRouter Key and three generated local
Keys. Send one request through each Agent namespace. Assert:

```rust
assert_eq!(events.len(), 3);
assert_eq!(events[0].provider_id, OPENROUTER_API_ID);
assert_eq!(events[1].provider_id, OPENROUTER_API_ID);
assert_eq!(events[2].provider_id, OPENROUTER_API_ID);
assert_eq!(
    events.iter().map(|event| event.agent_module_id.as_deref()).collect::<Vec<_>>(),
    vec![Some("opencode"), Some("openclaw"), Some("hermes")],
);
```

Capture all upstream requests and prove each uses the shared Provider Key and no
request contains any of the three local Keys.

### Step 2: Add a cross-surface sentinel leak test

Inject recognizable local and upstream sentinels, exercise:

- Provider/binding list commands;
- proxy setup and status;
- connection-test success/failure;
- request success/failure and streamed response processing;
- logs/errors/usage identifiers;
- SQL dump, backup, and sync payloads; and
- frontend rendered text and snapshots.

Serialize every collected ordinary surface and assert neither sentinel is present.
The only exception is the one explicit single-binding reveal value, which is
asserted separately and then zeroed.

### Step 3: Add restart and user-edit persistence coverage

Test that after restarting from the same database:

- all five cards remain;
- changed Provider enabled state remains;
- removed default bindings remain removed;
- manually added compatible bindings remain;
- Provider Key and local Key statuses reconcile from protected storage;
- raw Keys remain absent from the database file/dump; and
- Claude status is re-read from CLI rather than persisted as an OAuth credential.

Run:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml shared_openrouter -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml secret_leak -- --nocapture
pnpm test:unit tests/integration/SettingsDialog.test.tsx
```

Expected: PASS.

### Step 4: Commit

```bash
git add src-tauri/src/commands/usage_dashboard.rs \
  src-tauri/src/credentials/tests.rs src-tauri/src/proxy/handler_context.rs \
  src-tauri/src/proxy/forwarder.rs \
  src-tauri/src/proxy/response_processor.rs \
  src-tauri/src/database/backup.rs \
  src/components/settings/UsageProvidersSettings.test.tsx \
  tests/integration/SettingsDialog.test.tsx
git commit -m "test: cover fixed provider security invariants"
```

## Task 10: Full verification and documentation closeout

**Files:**

- Modify if behavior changed during implementation:
  `docs/superpowers/specs/2026-07-15-default-provider-cards-design.md`
- Modify: `docs/superpowers/plans/2026-07-15-default-provider-cards-implementation.md`

### Step 1: Run formatting and static checks

```bash
pnpm rust -- fmt --manifest-path src-tauri/Cargo.toml -- --check
pnpm rust -- clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
pnpm typecheck
pnpm format:check
```

Expected: all commands exit 0.

### Step 2: Run focused feature suites

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml database::tests::migration_v16_to_v17 -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml credentials::tests -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml commands::usage_dashboard::tests -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml proxy::handler_context::tests -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml proxy::handlers::tests -- --nocapture
pnpm test:unit src/components/settings/UsageProvidersSettings.test.tsx
pnpm test:unit src/components/settings/SystemProviderCard.test.tsx
pnpm test:unit src/components/settings/SystemProviderApiKeyDialog.test.tsx
pnpm test:unit src/components/settings/ClaudeCliAuthSection.test.tsx
pnpm test:unit src/components/settings/SystemProviderAgentBindings.test.tsx
```

Expected: all focused suites pass.

### Step 3: Run broad repository verification

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml
pnpm test:unit --exclude '.worktrees/**'
pnpm build:renderer
```

Expected: all commands exit 0. If an unrelated pre-existing test fails, record the
exact test and prove the focused feature suites still pass; do not silently weaken
or delete that test.

### Step 4: Self-review against every acceptance criterion

Before claiming completion, inspect the complete diff in order and verify:

- the catalog contains exactly five cards and the approved default bindings;
- system card repair cannot overwrite credentials/enabled/bindings;
- default binding seed never re-runs after user removal;
- Claude code contains no credential-file/Keychain/token access;
- each fixed API request verifies two credential generations and forwards one;
- route namespace is binding-owned;
- custom Providers retain legacy semantics;
- no ordinary DTO/log/backup/export contains a raw secret;
- no frontend cache/state retains a revealed local Key; and
- all commands were run through the repository's `pnpm rust`/`pnpm tauri`
  wrappers.

Update this plan's status notes with actual verification commands and results. If
the implementation required a design deviation, amend the approved design first
and explain why.

### Step 5: Commit documentation closeout

```bash
git add docs/superpowers/specs/2026-07-15-default-provider-cards-design.md \
  docs/superpowers/plans/2026-07-15-default-provider-cards-implementation.md
git commit -m "docs: close fixed provider implementation plan"
```

## Implementation status (2026-07-15)

Completed on `codex/default-provider-cards` with no design deviation. The final
implementation contains exactly the five approved fixed Provider cards and the
approved one-time defaults: ChatGPT Plus/Pro → Codex, Claude Pro/Max → Claude
Code, and OpenRouter → OpenCode/OpenClaw/Hermes. OpenAI API and Anthropic API
remain initially unbound, and every binding remains user-editable.

Final verification results:

- `pnpm rust -- fmt --manifest-path src-tauri/Cargo.toml -- --check`: passed.
- `pnpm rust -- clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`: passed.
- `pnpm typecheck`: passed.
- `pnpm format:check`: passed.
- Focused Rust feature suites passed: migration v16→v17 (6), credential service
  (66), usage-dashboard commands (17), proxy handler context (11), and proxy
  handlers (50).
- Focused Provider UI/settings/i18n suites passed: 9 files, 34 tests.
- `pnpm rust -- test --manifest-path src-tauri/Cargo.toml -- --test-threads=1`:
  passed with 2,249 library tests and 142 integration/end-to-end tests; 2
  explicitly ignored tests remained ignored. The serial runner and local socket
  permission are required by existing process-wide HOME and loopback-listener
  tests.
- `pnpm test:unit --exclude '.worktrees/**'`: passed, 97 files and 522 tests.
- `pnpm build:renderer`: passed.

The final self-review confirmed that catalog reconciliation preserves user
enablement, credentials, and binding edits; the one-time seed never restores a
removed default; Claude auth invokes only the official CLI auth commands; fixed
API routing verifies both local and upstream credential generations; binding
namespaces are server-owned; custom Provider behavior remains compatible; raw
keys are absent from normal DTO/log/backup/export surfaces; and revealed local
keys are copied transiently without entering React Query state.

## Official references to re-check during implementation

- Claude CLI commands and exit behavior:
  <https://code.claude.com/docs/en/cli-reference>
- Claude authentication/credential restrictions:
  <https://code.claude.com/docs/en/legal-and-compliance>

If either official contract changes before implementation, update the design and
tests before changing behavior. Do not substitute an unofficial Claude OAuth flow.
