export type OldIdentityOccurrenceKind =
  "ownedRename" | "legacyReadOnly" | "externalWireStable";

export interface OldIdentityOccurrence {
  file: string;
  lineNumber: number;
  context: string;
  kind: OldIdentityOccurrenceKind;
  reason: string;
}

const removedTask4OwnedOccurrences: readonly OldIdentityOccurrence[] = [
  {
    file: "src-tauri/src/services/skill.rs",
    lineNumber: 44,
    context: "CcSwitch,",
    kind: "ownedRename",
    reason: "prior writable current-product storage variant",
  },
  {
    file: "src-tauri/src/services/skill.rs",
    lineNumber: 483,
    context:
      'SkillStorageLocation::CcSwitch => get_app_config_dir().join("skills"),',
    kind: "ownedRename",
    reason: "prior writable current-product storage path arm",
  },
  {
    file: "src-tauri/src/services/skill.rs",
    lineNumber: 1168,
    context:
      'SkillStorageLocation::CcSwitch => get_app_config_dir().join("skills"),',
    kind: "ownedRename",
    reason: "prior storage migration target arm",
  },
  {
    file: "src-tauri/src/services/skill.rs",
    lineNumber: 1402,
    context: 'scan_sources.push((ssot_dir, "cc-switch".to_string()));',
    kind: "ownedRename",
    reason: "prior transient current-product scan label",
  },
  {
    file: "src-tauri/src/services/skill.rs",
    lineNumber: 1472,
    context:
      'search_sources.push((ssot_dir.clone(), "cc-switch".to_string()));',
    kind: "ownedRename",
    reason: "prior transient current-product import label",
  },
  {
    file: "src-tauri/src/settings.rs",
    lineNumber: 448,
    context:
      "/// Skill 存储位置：cc_switch（默认）或 unified（~/.agents/skills/）",
    kind: "ownedRename",
    reason: "prior current settings wire documentation",
  },
  {
    file: "src/types.ts",
    lineNumber: 235,
    context: 'export type SkillStorageLocation = "cc_switch" | "unified";',
    kind: "ownedRename",
    reason: "prior writable frontend storage wire value",
  },
  {
    file: "src/lib/api/skills.ts",
    lineNumber: 209,
    context: 'target: "cc_switch" | "unified",',
    kind: "ownedRename",
    reason: "prior writable Tauri command input",
  },
  {
    file: "src/lib/schemas/settings.ts",
    lineNumber: 38,
    context:
      'skillStorageLocation: z.enum(["cc_switch", "unified"]).optional(),',
    kind: "ownedRename",
    reason: "prior current settings write schema",
  },
  {
    file: "src/components/settings/SkillStorageLocationSettings.tsx",
    lineNumber: 82,
    context: 'active={value === "cc_switch"}',
    kind: "ownedRename",
    reason: "prior current storage UI state",
  },
  {
    file: "src/components/settings/SkillStorageLocationSettings.tsx",
    lineNumber: 84,
    context: 'onClick={() => handleSelect("cc_switch")}',
    kind: "ownedRename",
    reason: "prior current storage UI action",
  },
];

const serializedCodexProxyErrorOccurrences = [
  [1375, 'ProxyError::ForwardFailed(_) => "cc_switch_forward_failed",'],
  [
    1376,
    'ProxyError::Timeout(_) | ProxyError::StreamIdleTimeout(_) => "cc_switch_timeout",',
  ],
  [
    1377,
    'ProxyError::NoAvailableProvider => "cc_switch_no_available_provider",',
  ],
  [
    1378,
    'ProxyError::AllProvidersCircuitOpen => "cc_switch_all_providers_circuit_open",',
  ],
  [
    1379,
    'ProxyError::NoProvidersConfigured => "cc_switch_no_providers_configured",',
  ],
  [1380, 'ProxyError::MaxRetriesExceeded => "cc_switch_max_retries_exceeded",'],
  [1381, 'ProxyError::ProviderUnhealthy(_) => "cc_switch_provider_unhealthy",'],
  [1382, 'ProxyError::RouteNotBound(_) => "cc_switch_route_not_bound",'],
  [
    1383,
    'ProxyError::RouteProviderDisabled(_) => "cc_switch_route_provider_disabled",',
  ],
  [
    1384,
    'ProxyError::RouteProviderNotMetered(_) => "cc_switch_route_provider_not_metered",',
  ],
  [
    1385,
    'ProxyError::RouteConfigIncomplete(_) => "cc_switch_route_config_incomplete",',
  ],
  [1386, 'ProxyError::ConfigError(_) => "cc_switch_config_error",'],
  [1387, 'ProxyError::TransformError(_) => "cc_switch_transform_error",'],
  [1388, 'ProxyError::InvalidRequest(_) => "cc_switch_invalid_request",'],
  [1389, 'ProxyError::AuthError(_) => "cc_switch_auth_error",'],
  [1390, 'ProxyError::UpstreamError { .. } => "cc_switch_upstream_error",'],
  [1391, 'ProxyError::DatabaseError(_) => "cc_switch_database_error",'],
  [1392, 'ProxyError::Internal(_) => "cc_switch_internal_error",'],
  [1397, '| ProxyError::StopFailed(_) => "cc_switch_proxy_error",'],
] as const;

// Every preserved entry is an exact current file, one-based line, and complete
// trimmed source line. Task 5 expands this table before enabling the full
// runtime/current-doc scan; prefix, substring, file-wide, and regex exemptions
// are intentionally unsupported.
export const oldIdentityOccurrences: readonly OldIdentityOccurrence[] = [
  ...removedTask4OwnedOccurrences,
  {
    file: "src-tauri/src/services/skill.rs",
    lineNumber: 45,
    context: '#[serde(alias = "cc_switch")]',
    kind: "legacyReadOnly",
    reason: "read-only compatibility alias for existing settings",
  },
  {
    file: "src-tauri/src/settings.rs",
    lineNumber: 622,
    context: '== Some("cc_switch");',
    kind: "legacyReadOnly",
    reason: "exact detector for the read-only settings compatibility alias",
  },
  {
    file: "src-tauri/src/product_identity.rs",
    lineNumber: 9,
    context: 'pub const LEGACY_DISPLAY_NAME: &str = "CC Switch";',
    kind: "legacyReadOnly",
    reason: "typed prior-product display identity",
  },
  {
    file: "src-tauri/src/product_identity.rs",
    lineNumber: 10,
    context: 'pub const LEGACY_SLUG: &str = "cc-switch";',
    kind: "legacyReadOnly",
    reason: "typed prior-product slug",
  },
  {
    file: "src-tauri/src/product_identity.rs",
    lineNumber: 11,
    context: 'pub const LEGACY_DATA_DIR: &str = ".cc-switch";',
    kind: "legacyReadOnly",
    reason: "typed prior-product data directory",
  },
  {
    file: "src-tauri/src/product_identity.rs",
    lineNumber: 12,
    context: 'pub const LEGACY_DATABASE_FILE: &str = "cc-switch.db";',
    kind: "legacyReadOnly",
    reason: "typed prior current-app database filename",
  },
  {
    file: "src-tauri/src/product_identity.rs",
    lineNumber: 16,
    context:
      'pub const DATABASE_IDENTITY_ARCHIVE_FILE: &str = "cc-switch.db.pre-llm-usage-bar-v14";',
    kind: "legacyReadOnly",
    reason: "migration evidence filename",
  },
  ...[
    [201, 'let legacy_dir = home.join(".cc-switch");'],
    [387, 'let legacy = home.join(".cc-switch");'],
    [402, 'let legacy = temp.path().join(".cc-switch");'],
    [415, 'let legacy = temp.path().join(".cc-switch");'],
  ].map(([lineNumber, context]) => ({
    file: "src-tauri/src/config.rs",
    lineNumber: lineNumber as number,
    context: context as string,
    kind: "legacyReadOnly" as const,
    reason: "guard or test protecting the original application data directory",
  })),
  {
    file: "src-tauri/src/settings.rs",
    lineNumber: 99,
    context: '"cc-switch-sync".to_string()',
    kind: "externalWireStable",
    reason: "existing WebDAV and S3 remote root",
  },
  {
    file: "src-tauri/src/services/sync_protocol.rs",
    lineNumber: 25,
    context:
      'pub(crate) const PROTOCOL_FORMAT: &str = "cc-switch-webdav-sync";',
    kind: "externalWireStable",
    reason: "remote manifest protocol identifier",
  },
  {
    file: "src-tauri/src/services/sync_protocol.rs",
    lineNumber: 351,
    context:
      'let env_name = ["CC_SWITCH_DEVICE_NAME", "COMPUTERNAME", "HOSTNAME"]',
    kind: "externalWireStable",
    reason: "existing device-name environment alias",
  },
  {
    file: "src-tauri/src/services/codex_oauth_models.rs",
    lineNumber: 24,
    context: '.header("originator", "cc-switch")',
    kind: "externalWireStable",
    reason: "OAuth upstream compatibility header",
  },
  {
    file: "src-tauri/src/proxy/providers/claude.rs",
    lineNumber: 846,
    context: 'HeaderValue::from_static("cc-switch"),',
    kind: "externalWireStable",
    reason: "OAuth proxy compatibility header",
  },
  {
    file: "src-tauri/src/proxy/providers/codex_oauth_auth.rs",
    lineNumber: 58,
    context: 'const CODEX_USER_AGENT: &str = "cc-switch-codex-oauth";',
    kind: "externalWireStable",
    reason: "OAuth client compatibility User-Agent",
  },
  {
    file: "src-tauri/src/codex_config.rs",
    lineNumber: 15,
    context:
      'pub const CC_SWITCH_CODEX_MODEL_CATALOG_FILENAME: &str = "cc-switch-model-catalog.json";',
    kind: "externalWireStable",
    reason: "existing Codex catalog compatibility filename",
  },
  {
    file: "src-tauri/src/codex_config.rs",
    lineNumber: 3025,
    context: 'model_catalog_json = "cc-switch-model-catalog.json"',
    kind: "externalWireStable",
    reason: "Codex catalog compatibility fixture",
  },
  {
    file: "src-tauri/src/codex_config.rs",
    lineNumber: 3050,
    context:
      'let input = r#"model_catalog_json = "cc-switch-model-catalog.json"',
    kind: "externalWireStable",
    reason: "Codex catalog compatibility fixture",
  },
  {
    file: "src-tauri/src/database/backup.rs",
    lineNumber: 15,
    context:
      'const CC_SWITCH_SQL_EXPORT_HEADER: &str = "-- CC Switch SQLite 导出";',
    kind: "externalWireStable",
    reason: "accepted historical SQL interchange header",
  },
  {
    file: "src-tauri/src/config.rs",
    lineNumber: 23,
    context: 'if let Ok(home) = std::env::var("CC_SWITCH_TEST_HOME") {',
    kind: "externalWireStable",
    reason: "existing test and automation environment alias",
  },
  ...serializedCodexProxyErrorOccurrences.map(([lineNumber, context]) => ({
    file: "src-tauri/src/proxy/handlers.rs",
    lineNumber,
    context,
    kind: "externalWireStable" as const,
    reason: `serialized client-visible error contract at line ${lineNumber}`,
  })),
] as const;

export function classifyOldIdentityOccurrence(
  file: string,
  lineNumber: number,
  line: string,
): OldIdentityOccurrence | undefined {
  const normalizedFile = file.replace(/\\/g, "/").replace(/^\.\//, "");
  const normalizedLine = line.trim();
  return oldIdentityOccurrences.find(
    (entry) =>
      entry.file === normalizedFile &&
      entry.lineNumber === lineNumber &&
      entry.context === normalizedLine,
  );
}
