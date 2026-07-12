export type OldIdentityOccurrenceKind =
  "ownedRename" | "legacyReadOnly" | "externalWireStable";

export interface OldIdentityOccurrence {
  file: string;
  context: string;
  kind: OldIdentityOccurrenceKind;
  reason: string;
}

const serializedCodexProxyErrorCodes = [
  "cc_switch_forward_failed",
  "cc_switch_timeout",
  "cc_switch_no_available_provider",
  "cc_switch_all_providers_circuit_open",
  "cc_switch_no_providers_configured",
  "cc_switch_max_retries_exceeded",
  "cc_switch_provider_unhealthy",
  "cc_switch_route_not_bound",
  "cc_switch_route_provider_disabled",
  "cc_switch_route_provider_not_metered",
  "cc_switch_route_config_incomplete",
  "cc_switch_config_error",
  "cc_switch_transform_error",
  "cc_switch_invalid_request",
  "cc_switch_auth_error",
  "cc_switch_upstream_error",
  "cc_switch_database_error",
  "cc_switch_internal_error",
  "cc_switch_proxy_error",
] as const;

// Exact file + stable source context only. Task 5 expands this inventory before
// scanning every runtime/current-document occurrence; blanket path or regex
// exemptions are intentionally unsupported.
export const oldIdentityOccurrences: readonly OldIdentityOccurrence[] = [
  {
    file: "src-tauri/src/services/skill.rs",
    context: "SkillStorageLocation::CcSwitch",
    kind: "ownedRename",
    reason: "writable current-product storage variant",
  },
  {
    file: "src-tauri/src/services/skill.rs",
    context: '(ssot_dir, "cc-switch".to_string())',
    kind: "ownedRename",
    reason: "transient current-product SSOT source label",
  },
  {
    file: "src/types.ts",
    context: 'SkillStorageLocation = "cc_switch"',
    kind: "ownedRename",
    reason: "writable frontend storage wire value",
  },
  {
    file: "src/lib/api/skills.ts",
    context: 'target: "cc_switch" | "unified"',
    kind: "ownedRename",
    reason: "writable Tauri command input",
  },
  {
    file: "src/lib/schemas/settings.ts",
    context: 'z.enum(["cc_switch", "unified"])',
    kind: "ownedRename",
    reason: "current settings write schema",
  },
  {
    file: "src/components/settings/SkillStorageLocationSettings.tsx",
    context: 'handleSelect("cc_switch")',
    kind: "ownedRename",
    reason: "current storage UI action",
  },
  {
    file: "src-tauri/src/product_identity.rs",
    context: "pub const LEGACY_",
    kind: "legacyReadOnly",
    reason: "typed prior-product compatibility contract",
  },
  {
    file: "src-tauri/src/config.rs",
    context: 'join(".cc-switch")',
    kind: "legacyReadOnly",
    reason: "guard protecting the original application data directory",
  },
  {
    file: "src-tauri/src/settings.rs",
    context: '"cc-switch-sync".to_string()',
    kind: "externalWireStable",
    reason: "existing WebDAV and S3 remote root",
  },
  {
    file: "src-tauri/src/services/sync_protocol.rs",
    context: '"cc-switch-webdav-sync"',
    kind: "externalWireStable",
    reason: "remote manifest protocol identifier",
  },
  {
    file: "src-tauri/src/services/codex_oauth_models.rs",
    context: '.header("originator", "cc-switch")',
    kind: "externalWireStable",
    reason: "OAuth upstream compatibility header",
  },
  {
    file: "src-tauri/src/proxy/providers/claude.rs",
    context: 'HeaderValue::from_static("cc-switch")',
    kind: "externalWireStable",
    reason: "OAuth proxy compatibility header",
  },
  {
    file: "src-tauri/src/proxy/providers/codex_oauth_auth.rs",
    context: 'CODEX_USER_AGENT: &str = "cc-switch-codex-oauth"',
    kind: "externalWireStable",
    reason: "OAuth client compatibility User-Agent",
  },
  {
    file: "src-tauri/src/codex_config.rs",
    context: '"cc-switch-model-catalog.json"',
    kind: "externalWireStable",
    reason: "existing Codex catalog compatibility filename",
  },
  {
    file: "src-tauri/src/database/backup.rs",
    context: 'CC_SWITCH_SQL_EXPORT_HEADER: &str = "-- CC Switch SQLite',
    kind: "externalWireStable",
    reason: "accepted historical SQL interchange header",
  },
  {
    file: "src-tauri/src/config.rs",
    context: 'std::env::var("CC_SWITCH_TEST_HOME")',
    kind: "externalWireStable",
    reason: "existing test and automation environment alias",
  },
  {
    file: "src-tauri/src/services/sync_protocol.rs",
    context: '"CC_SWITCH_DEVICE_NAME"',
    kind: "externalWireStable",
    reason: "existing device-name environment alias",
  },
  ...serializedCodexProxyErrorCodes.map((code) => ({
    file: "src-tauri/src/proxy/handlers.rs",
    context: `"${code}"`,
    kind: "externalWireStable" as const,
    reason: `serialized client-visible error code ${code}`,
  })),
] as const;

export function classifyOldIdentityOccurrence(
  file: string,
  _lineNumber: number,
  line: string,
): OldIdentityOccurrence | undefined {
  const normalizedFile = file.replace(/\\/g, "/").replace(/^\.\//, "");
  return oldIdentityOccurrences.find(
    (entry) => entry.file === normalizedFile && line.includes(entry.context),
  );
}
