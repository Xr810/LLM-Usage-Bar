use super::lifecycle_lock::{CredentialLifecycleLock, SharedLifecycleGuard};
use super::{CredentialStore, SecretString, KEYCHAIN_SERVICE};
use crate::database::{
    BindingAuthMode, CredentialBindingSnapshot, CredentialJournalEntry, CredentialMutationKind,
    CredentialOperationReservation, Database, ProviderCredentialJournalEntry,
    ProviderCredentialOperationReservation, ProviderCredentialSnapshot,
};
use crate::error::AppError;
use crate::provider::Provider;
use crate::proxy::provider_router::{
    build_binding_route_projection, BindingPricingOverride, UpstreamCredentialPlacement,
};
use crate::usage::domain::{
    AgentProviderBindingInput, AgentProviderBindingView, BindingCredentialStatus,
    LocalBindingKeyReveal, SystemProviderAuthKind, UsageProviderView,
};
use crate::usage::system_providers::{is_fixed_api_preset, system_binding_route_protocol};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, Zeroizing};

const FINGERPRINT_SEPARATOR: &[u8] = b"\0";
const PROVIDER_FINGERPRINT_DOMAIN: &[u8] = b"com.xr810.llm-usage-bar.provider-upstream.v1\0";
const MIN_BINDING_CREDENTIAL_BYTES: usize = 16;
const MAX_BINDING_CREDENTIAL_BYTES: usize = 4096;
const MIN_BINDING_CREDENTIAL_DISTINCT_BYTES: usize = 4;
const RESPONSE_NORMALIZATION_ROUNDS: usize = 16;
const RESPONSE_NORMALIZATION_MAX_STATES: usize = 64;
const RESPONSE_NORMALIZATION_MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy)]
struct CredentialNormalizationScan {
    matched: bool,
    partial_len: usize,
    incomplete_encoding: bool,
}

impl CredentialNormalizationScan {
    fn matched() -> Self {
        Self {
            matched: true,
            partial_len: 0,
            incomplete_encoding: false,
        }
    }

    fn has_pending_suffix(self) -> bool {
        self.partial_len > 0 || self.incomplete_encoding
    }
}

fn public_error(code: &'static str) -> AppError {
    AppError::Message(code.to_string())
}

fn normalize_db_error(error: AppError) -> AppError {
    if let AppError::Message(code) = error {
        if matches!(
            code.as_str(),
            "binding_not_found"
                | "invalid_binding"
                | "credential_required"
                | "credential_conflict"
                | "credential_unavailable"
                | "unsupported_auth"
        ) {
            return AppError::Message(code);
        }
    }
    log::error!("credential database operation failed");
    public_error("credential_unavailable")
}

fn credential_fingerprint(secret: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(KEYCHAIN_SERVICE.as_bytes());
    hasher.update(FINGERPRINT_SEPARATOR);
    hasher.update(secret);
    hasher.finalize().into()
}

fn provider_credential_fingerprint(secret: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(PROVIDER_FINGERPRINT_DOMAIN);
    hasher.update(secret);
    hasher.finalize().into()
}

fn provider_credential_is_acceptable(secret: &[u8]) -> bool {
    (1..=MAX_BINDING_CREDENTIAL_BYTES).contains(&secret.len())
        && secret.iter().all(u8::is_ascii_graphic)
}

fn generate_local_binding_key() -> SecretString {
    SecretString::new(format!(
        "lub_{}_{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple(),
    ))
}

/// Binding credentials are forwarded through HTTP authentication headers and
/// also select a local route. Reject trivially enumerable or ambiguous values
/// whose ordinary runtime rendering (for example `200`) could collide with
/// status, latency, or token diagnostics despite credential-aware sink guards.
fn binding_credential_is_acceptable(secret: &[u8]) -> bool {
    if !(MIN_BINDING_CREDENTIAL_BYTES..=MAX_BINDING_CREDENTIAL_BYTES).contains(&secret.len())
        || !secret.iter().all(u8::is_ascii_graphic)
    {
        return false;
    }
    let mut seen = [false; 256];
    let mut distinct = 0;
    for byte in secret {
        let slot = &mut seen[usize::from(*byte)];
        if !*slot {
            *slot = true;
            distinct += 1;
            if distinct >= MIN_BINDING_CREDENTIAL_DISTINCT_BYTES {
                return true;
            }
        }
    }
    false
}

/// A verified, frozen binding projection and its protected upstream credential.
/// It is intentionally non-Clone and non-serializable.
#[allow(dead_code)] // Its route/key accessors are consumed by Task 4 proxy routing.
pub struct ResolvedBindingCredential {
    ownership: FrozenBindingOwnership,
    route_app_type: String,
    product_group_id: String,
    runtime_provider: Provider,
    credential_placement: UpstreamCredentialPlacement,
    legacy_pricing_provider_id: Option<String>,
    pricing_override: BindingPricingOverride,
    secret: Zeroizing<Vec<u8>>,
}

/// Transient, non-serializable detector used to keep the resolved binding key
/// out of response-derived identifiers and compatibility logs. Prefix-safe
/// streaming requires the request credential itself; it is shared only for the
/// request lifetime and zeroized when the last guard clone is dropped.
#[derive(Clone)]
pub(crate) struct CredentialExposureGuard {
    fingerprint: [u8; 32],
    rolling_hash: u64,
    secret_len: usize,
    secret: Arc<Zeroizing<Vec<u8>>>,
    prefix_function: Arc<Zeroizing<Vec<usize>>>,
}

impl CredentialExposureGuard {
    pub(crate) fn from_secret(secret: &[u8]) -> Self {
        let mut prefix_function = vec![0_usize; secret.len()];
        for index in 1..secret.len() {
            let mut matched = prefix_function[index - 1];
            while matched > 0 && secret[index] != secret[matched] {
                matched = prefix_function[matched - 1];
            }
            if secret[index] == secret[matched] {
                matched += 1;
            }
            prefix_function[index] = matched;
        }

        Self {
            fingerprint: credential_fingerprint(secret),
            rolling_hash: credential_rolling_hash(secret),
            secret_len: secret.len(),
            secret: Arc::new(Zeroizing::new(secret.to_vec())),
            prefix_function: Arc::new(Zeroizing::new(prefix_function)),
        }
    }

    pub(crate) fn contains_bytes(&self, value: &[u8]) -> bool {
        self.scan_normalized_bytes(value).matched
    }

    fn scan_normalized_bytes(&self, value: &[u8]) -> CredentialNormalizationScan {
        if self.contains_raw_bytes(value) {
            return CredentialNormalizationScan::matched();
        }
        if value.len() > RESPONSE_NORMALIZATION_MAX_BYTES {
            return CredentialNormalizationScan::matched();
        }

        let mut partial_len = self.target_prefix_suffix_len(value);
        let mut incomplete_encoding = has_incomplete_normalization_suffix(value);
        let mut frontier = vec![Zeroizing::new(value.to_vec())];
        let mut seen: HashSet<[u8; 32]> = HashSet::new();
        seen.insert(Sha256::digest(value).into());
        let mut normalized_bytes = value.len();

        for _ in 0..RESPONSE_NORMALIZATION_ROUNDS {
            let mut next = Vec::new();
            for candidate in &frontier {
                for decoded in [
                    decode_url_component(candidate, false).map(Zeroizing::new),
                    decode_url_component(candidate, true).map(Zeroizing::new),
                    decode_json_escapes(candidate).map(Zeroizing::new),
                ]
                .into_iter()
                .flatten()
                {
                    if decoded.as_slice() == candidate.as_slice() {
                        continue;
                    }
                    if self.contains_raw_bytes(&decoded) {
                        return CredentialNormalizationScan::matched();
                    }
                    partial_len = partial_len.max(self.target_prefix_suffix_len(&decoded));
                    incomplete_encoding |= has_incomplete_normalization_suffix(&decoded);
                    let digest: [u8; 32] = Sha256::digest(decoded.as_slice()).into();
                    if !seen.insert(digest) {
                        continue;
                    }
                    normalized_bytes = normalized_bytes.saturating_add(decoded.len());
                    if seen.len() > RESPONSE_NORMALIZATION_MAX_STATES
                        || normalized_bytes > RESPONSE_NORMALIZATION_MAX_BYTES
                    {
                        return CredentialNormalizationScan::matched();
                    }
                    next.push(decoded);
                }
            }
            if next.is_empty() {
                return CredentialNormalizationScan {
                    matched: false,
                    partial_len,
                    incomplete_encoding,
                };
            }
            frontier = next;
        }

        // If the input still changes after the supported normalization depth,
        // fail closed rather than release an attacker-controlled deeper chain.
        let still_changing = frontier.iter().any(|candidate| {
            [
                decode_url_component(candidate, false).map(Zeroizing::new),
                decode_url_component(candidate, true).map(Zeroizing::new),
                decode_json_escapes(candidate).map(Zeroizing::new),
            ]
            .into_iter()
            .flatten()
            .any(|decoded| decoded.as_slice() != candidate.as_slice())
        });
        if still_changing {
            CredentialNormalizationScan::matched()
        } else {
            CredentialNormalizationScan {
                matched: false,
                partial_len,
                incomplete_encoding,
            }
        }
    }

    fn contains_raw_bytes(&self, value: &[u8]) -> bool {
        if self.secret_len == 0 || value.len() < self.secret_len {
            return false;
        }
        let mut candidate_hash = credential_rolling_hash(&value[..self.secret_len]);
        if candidate_hash == self.rolling_hash
            && bool::from(
                credential_fingerprint(&value[..self.secret_len]).ct_eq(&self.fingerprint),
            )
        {
            return true;
        }

        let highest_power = (1..self.secret_len).fold(1_u64, |power, _| {
            power.wrapping_mul(CREDENTIAL_ROLLING_HASH_BASE)
        });
        for end in self.secret_len..value.len() {
            let outgoing = u64::from(value[end - self.secret_len]) + 1;
            let incoming = u64::from(value[end]) + 1;
            candidate_hash = candidate_hash
                .wrapping_sub(outgoing.wrapping_mul(highest_power))
                .wrapping_mul(CREDENTIAL_ROLLING_HASH_BASE)
                .wrapping_add(incoming);
            let start = end + 1 - self.secret_len;
            if candidate_hash == self.rolling_hash
                && bool::from(credential_fingerprint(&value[start..=end]).ct_eq(&self.fingerprint))
            {
                return true;
            }
        }
        false
    }

    fn target_prefix_suffix_len(&self, value: &[u8]) -> usize {
        if value.is_empty() || self.secret_len == 0 {
            return 0;
        }
        value.iter().fold(0_usize, |matched, byte| {
            self.advance_prefix_match(matched, *byte)
        })
    }

    fn advance_prefix_match(&self, mut matched: usize, byte: u8) -> usize {
        let pattern = self.secret.as_slice();
        while matched > 0 && pattern[matched] != byte {
            matched = self.prefix_function[matched - 1];
        }
        if pattern[matched] == byte {
            matched += 1;
        }
        if matched == pattern.len() {
            self.prefix_function[matched - 1]
        } else {
            matched
        }
    }

    pub(crate) fn contains(&self, value: &str) -> bool {
        self.contains_bytes(value.as_bytes())
    }

    pub(crate) fn contains_json_value(&self, value: &serde_json::Value) -> bool {
        fn contains_individual(guard: &CredentialExposureGuard, value: &serde_json::Value) -> bool {
            match value {
                serde_json::Value::String(value) => guard.contains(value),
                serde_json::Value::Array(values) => {
                    values.iter().any(|value| contains_individual(guard, value))
                }
                serde_json::Value::Object(values) => values
                    .iter()
                    .any(|(key, value)| guard.contains(key) || contains_individual(guard, value)),
                serde_json::Value::Number(value) => guard.contains(&value.to_string()),
                serde_json::Value::Bool(value) => {
                    guard.contains(if *value { "true" } else { "false" })
                }
                serde_json::Value::Null => guard.contains("null"),
            }
        }

        contains_individual(self, value) || self.semantic_stream_scanner().push_json_value(value)
    }

    pub(crate) fn stream_scanner(&self) -> CredentialStreamScanner {
        CredentialStreamScanner::new(self.clone())
    }

    pub(crate) fn semantic_stream_scanner(&self) -> CredentialSemanticStreamScanner {
        CredentialSemanticStreamScanner::new(self.clone())
    }

    pub(crate) fn redact_option(&self, value: Option<String>) -> Option<String> {
        value.filter(|value| !self.contains(value))
    }

    pub(crate) fn redact_or<'a>(&self, value: &'a str, replacement: &'a str) -> &'a str {
        if self.contains(value) {
            replacement
        } else {
            value
        }
    }
}

const SEMANTIC_RESPONSE_FIELDS: [&str; 10] = [
    "text",
    "delta",
    "content",
    "arguments",
    "partial_json",
    "output_text",
    "reasoning_content",
    "reasoning",
    "thought",
    "thinking",
];
const MAX_SEMANTIC_CHANNEL_BYTES: usize = 256 * 1024;
const MAX_SEMANTIC_CHANNEL_UPDATES: usize = 4096;
const MAX_SEMANTIC_CHANNELS: usize = 64;
const MAX_SEMANTIC_IDENTITIES: usize = 8;

const NUMERIC_SEMANTIC_IDENTITIES: [(&str, u8); 3] =
    [("index", 0), ("output_index", 1), ("content_index", 2)];
const TEXT_SEMANTIC_IDENTITIES: [(&str, u8); 2] = [("item_id", 3), ("call_id", 4)];

#[derive(Clone, PartialEq, Eq, Hash)]
struct CredentialSemanticIdentity {
    kind: u8,
    digest: [u8; 32],
}

impl CredentialSemanticIdentity {
    fn new(kind: u8, value: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update([kind]);
        hasher.update(FINGERPRINT_SEPARATOR);
        hasher.update(value);
        Self {
            kind,
            digest: hasher.finalize().into(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct CredentialSemanticChannelKey {
    field: usize,
    identities: Vec<CredentialSemanticIdentity>,
}

struct CredentialSemanticChannel {
    bytes: Zeroizing<Vec<u8>>,
    updates: usize,
    partial_len: usize,
    incomplete_encoding: bool,
}

/// Stateful JSON/SSE semantic scanner. Only the fixed protocol payload fields
/// that clients concatenate are retained. Stable protocol indices/IDs isolate
/// parallel choices, content blocks, and tool calls without retaining upstream
/// identifier text. All channel counts, identity depth, bytes, and updates are
/// bounded and fail closed.
pub(crate) struct CredentialSemanticStreamScanner {
    guard: CredentialExposureGuard,
    channels: HashMap<CredentialSemanticChannelKey, CredentialSemanticChannel>,
}

impl CredentialSemanticStreamScanner {
    fn new(guard: CredentialExposureGuard) -> Self {
        Self {
            guard,
            channels: HashMap::new(),
        }
    }

    fn semantic_channel(key: &str) -> Option<usize> {
        SEMANTIC_RESPONSE_FIELDS
            .iter()
            .position(|candidate| *candidate == key)
    }

    fn object_identities(
        values: &serde_json::Map<String, serde_json::Value>,
        inherited: &[CredentialSemanticIdentity],
    ) -> Option<Vec<CredentialSemanticIdentity>> {
        let mut identities = inherited.to_vec();
        let mut found_numeric = false;
        for (field, kind) in NUMERIC_SEMANTIC_IDENTITIES {
            let Some(serde_json::Value::Number(value)) = values.get(field) else {
                continue;
            };
            found_numeric = true;
            identities.push(CredentialSemanticIdentity::new(
                kind,
                value.to_string().as_bytes(),
            ));
        }
        if !found_numeric {
            for (field, kind) in TEXT_SEMANTIC_IDENTITIES {
                let Some(serde_json::Value::String(value)) = values.get(field) else {
                    continue;
                };
                identities.push(CredentialSemanticIdentity::new(kind, value.as_bytes()));
            }
        }
        (identities.len() <= MAX_SEMANTIC_IDENTITIES).then_some(identities)
    }

    fn push_semantic_bytes(&mut self, key: CredentialSemanticChannelKey, bytes: &[u8]) -> bool {
        let previous = self.channels.get(&key);
        let previous_len = previous.map_or(0, |channel| channel.bytes.len());
        let combined_len = previous_len.saturating_add(bytes.len());
        if combined_len > RESPONSE_NORMALIZATION_MAX_BYTES {
            return true;
        }

        let updates = previous
            .map_or(0, |channel| channel.updates)
            .saturating_add(1);
        let mut combined = Zeroizing::new(Vec::with_capacity(combined_len));
        if let Some(previous) = previous {
            combined.extend_from_slice(&previous.bytes);
        }
        combined.extend_from_slice(bytes);

        let mut scan = self.guard.scan_normalized_bytes(&combined);
        if scan.matched {
            return true;
        }
        if !scan.has_pending_suffix() {
            self.channels.remove(&key);
            return false;
        }

        if updates > MAX_SEMANTIC_CHANNEL_UPDATES {
            return true;
        }

        // Retain only a suffix that independently preserves all pending
        // evidence. Try tiny suffixes first for the common raw-prefix and
        // split-escape cases. Deeply encoded candidates may need the full
        // bounded tail; if that tail loses any prefix/decoder state, reject
        // rather than release evidence that a future update could complete.
        if combined.len() > MAX_SEMANTIC_CHANNEL_BYTES {
            let preserves = |candidate_scan: CredentialNormalizationScan| {
                !candidate_scan.matched
                    && candidate_scan.partial_len >= scan.partial_len
                    && (!scan.incomplete_encoding || candidate_scan.incomplete_encoding)
            };
            let mut retained = None;
            for suffix_len in 1..=64.min(combined.len()) {
                let start = combined.len() - suffix_len;
                let candidate_scan = self.guard.scan_normalized_bytes(&combined[start..]);
                if preserves(candidate_scan) {
                    retained = Some((Zeroizing::new(combined[start..].to_vec()), candidate_scan));
                    break;
                }
            }
            if retained.is_none() {
                let start = combined.len() - MAX_SEMANTIC_CHANNEL_BYTES;
                let candidate_scan = self.guard.scan_normalized_bytes(&combined[start..]);
                if preserves(candidate_scan) {
                    retained = Some((Zeroizing::new(combined[start..].to_vec()), candidate_scan));
                }
            }
            let Some((retained_bytes, retained_scan)) = retained else {
                return true;
            };
            combined = retained_bytes;
            scan = retained_scan;
        }
        if !self.channels.contains_key(&key) && self.channels.len() >= MAX_SEMANTIC_CHANNELS {
            return true;
        }

        self.channels.insert(
            key,
            CredentialSemanticChannel {
                bytes: combined,
                updates,
                partial_len: scan.partial_len,
                incomplete_encoding: scan.incomplete_encoding,
            },
        );
        false
    }

    pub(crate) fn push_json_value(&mut self, value: &serde_json::Value) -> bool {
        fn walk(
            scanner: &mut CredentialSemanticStreamScanner,
            value: &serde_json::Value,
            inherited_channel: Option<usize>,
            inherited_identities: &[CredentialSemanticIdentity],
        ) -> bool {
            match value {
                serde_json::Value::Object(values) => {
                    let Some(identities) = CredentialSemanticStreamScanner::object_identities(
                        values,
                        inherited_identities,
                    ) else {
                        return true;
                    };
                    values.iter().any(|(key, value)| {
                        if scanner.guard.contains(key) {
                            return true;
                        }
                        let channel = CredentialSemanticStreamScanner::semantic_channel(key)
                            .or(inherited_channel);
                        walk(scanner, value, channel, &identities)
                    })
                }
                serde_json::Value::Array(values) => values
                    .iter()
                    .any(|value| walk(scanner, value, inherited_channel, inherited_identities)),
                scalar => {
                    let encoded;
                    let bytes = match scalar {
                        serde_json::Value::String(value) => value.as_bytes(),
                        serde_json::Value::Number(value) => {
                            encoded = value.to_string();
                            encoded.as_bytes()
                        }
                        serde_json::Value::Bool(value) => {
                            if *value {
                                &b"true"[..]
                            } else {
                                &b"false"[..]
                            }
                        }
                        serde_json::Value::Null => b"null",
                        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                            unreachable!()
                        }
                    };
                    match inherited_channel {
                        Some(field) => scanner.push_semantic_bytes(
                            CredentialSemanticChannelKey {
                                field,
                                identities: inherited_identities.to_vec(),
                            },
                            bytes,
                        ),
                        None => scanner.guard.contains_bytes(bytes),
                    }
                }
            }
        }

        walk(self, value, None, &[])
    }

    pub(crate) fn has_partial_match(&self) -> bool {
        self.channels
            .values()
            .any(|channel| channel.partial_len > 0 || channel.incomplete_encoding)
    }
}

/// Stateful response scanner. It detects a protected credential even when raw
/// or repeatedly percent/form-encoded bytes are split across arbitrary stream
/// chunk boundaries. The guard owns one request-lifetime zeroized key copy;
/// matcher windows and decoder state are bounded and zeroized on drop.
pub(crate) struct CredentialStreamScanner {
    guard: CredentialExposureGuard,
    raw: CredentialWindowMatcher,
    percent: StreamingDecodePipeline,
    form: StreamingDecodePipeline,
    json: StreamingJsonDecodePipeline,
    json_strings: StreamingJsonStringMatcher,
}

impl CredentialStreamScanner {
    fn new(guard: CredentialExposureGuard) -> Self {
        Self {
            raw: CredentialWindowMatcher::new(&guard),
            percent: StreamingDecodePipeline::new(&guard, false),
            form: StreamingDecodePipeline::new(&guard, true),
            json: StreamingJsonDecodePipeline::new(&guard),
            json_strings: StreamingJsonStringMatcher::new(&guard),
            guard,
        }
    }

    /// Returns true as soon as the newly supplied bytes complete any protected
    /// raw or normalized credential representation.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> bool {
        for byte in bytes {
            if self.raw.push(*byte, &self.guard)
                || self.percent.push(*byte, &self.guard)
                || self.form.push(*byte, &self.guard)
                || self.json.push(*byte, &self.guard)
                || self.json_strings.push(*byte, &self.guard)
            {
                return true;
            }
        }
        false
    }

    pub(crate) fn has_partial_match(&self) -> bool {
        self.raw.target_prefix_suffix_len() > 0
            || self.percent.has_partial_match()
            || self.form.has_partial_match()
            || self.json.has_partial_match()
            || self.json_strings.has_partial_match()
    }
}

#[derive(Clone, Copy)]
enum JsonStringState {
    Outside,
    Inside,
    Escape,
    Unicode { value: u16, digits: u8 },
}

/// Feeds the decoded contents of every JSON string into one continuous set of
/// matchers. Keeping the matcher across string/SSE-event boundaries prevents a
/// malicious upstream from splitting a key across successive text deltas that
/// a client or shadow cache will later concatenate.
struct StreamingJsonStringMatcher {
    state: JsonStringState,
    raw: CredentialWindowMatcher,
    percent: StreamingDecodePipeline,
    form: StreamingDecodePipeline,
}

impl StreamingJsonStringMatcher {
    fn new(guard: &CredentialExposureGuard) -> Self {
        Self {
            state: JsonStringState::Outside,
            raw: CredentialWindowMatcher::new(guard),
            percent: StreamingDecodePipeline::new(guard, false),
            form: StreamingDecodePipeline::new(guard, true),
        }
    }

    fn feed_decoded(&mut self, byte: u8, guard: &CredentialExposureGuard) -> bool {
        self.raw.push(byte, guard) || self.percent.push(byte, guard) || self.form.push(byte, guard)
    }

    fn push(&mut self, byte: u8, guard: &CredentialExposureGuard) -> bool {
        match self.state {
            JsonStringState::Outside => {
                if byte == b'"' {
                    self.state = JsonStringState::Inside;
                }
                false
            }
            JsonStringState::Inside if byte == b'"' => {
                self.state = JsonStringState::Outside;
                false
            }
            JsonStringState::Inside if byte == b'\\' => {
                self.state = JsonStringState::Escape;
                false
            }
            JsonStringState::Inside => self.feed_decoded(byte, guard),
            JsonStringState::Escape => match byte {
                b'"' | b'\\' | b'/' => {
                    self.state = JsonStringState::Inside;
                    self.feed_decoded(byte, guard)
                }
                b'b' | b'f' | b'n' | b'r' | b't' => {
                    self.state = JsonStringState::Inside;
                    let decoded = match byte {
                        b'b' => 0x08,
                        b'f' => 0x0c,
                        b'n' => b'\n',
                        b'r' => b'\r',
                        _ => b'\t',
                    };
                    self.feed_decoded(decoded, guard)
                }
                b'u' => {
                    self.state = JsonStringState::Unicode {
                        value: 0,
                        digits: 0,
                    };
                    false
                }
                _ => {
                    self.state = JsonStringState::Inside;
                    self.feed_decoded(byte, guard)
                }
            },
            JsonStringState::Unicode { value, digits } => {
                let Some(nibble) = hex_nibble(byte) else {
                    self.state = JsonStringState::Inside;
                    return false;
                };
                let value = (value << 4) | u16::from(nibble);
                let digits = digits + 1;
                if digits == 4 {
                    self.state = JsonStringState::Inside;
                    self.feed_decoded(u8::try_from(value).unwrap_or(b'?'), guard)
                } else {
                    self.state = JsonStringState::Unicode { value, digits };
                    false
                }
            }
        }
    }

    fn has_partial_match(&self) -> bool {
        self.raw.target_prefix_suffix_len() > 0
            || self.percent.has_partial_match()
            || self.form.has_partial_match()
    }
}

struct StreamingJsonDecodePipeline {
    decoder: StreamingJsonDecoder,
    matcher: CredentialWindowMatcher,
}

impl StreamingJsonDecodePipeline {
    fn new(guard: &CredentialExposureGuard) -> Self {
        Self {
            decoder: StreamingJsonDecoder::new(),
            matcher: CredentialWindowMatcher::new(guard),
        }
    }

    fn push(&mut self, byte: u8, guard: &CredentialExposureGuard) -> bool {
        let (output, output_len) = self.decoder.push(byte);
        output
            .into_iter()
            .take(output_len)
            .any(|decoded| self.matcher.push(decoded, guard))
    }

    fn has_partial_match(&self) -> bool {
        self.matcher.target_prefix_suffix_len() > 0 || self.decoder.has_pending_escape()
    }
}

#[derive(Clone, Copy)]
enum JsonDecodeState {
    Plain,
    Escape,
    Unicode { value: u16, digits: u8 },
}

struct StreamingJsonDecoder {
    state: JsonDecodeState,
}

impl StreamingJsonDecoder {
    fn new() -> Self {
        Self {
            state: JsonDecodeState::Plain,
        }
    }

    fn push(&mut self, byte: u8) -> ([u8; 3], usize) {
        let mut output = [0; 3];
        let mut output_len = 0;
        let mut current = Some(byte);
        while let Some(byte) = current.take() {
            match self.state {
                JsonDecodeState::Plain if byte == b'\\' => {
                    self.state = JsonDecodeState::Escape;
                }
                JsonDecodeState::Plain => {
                    output[output_len] = byte;
                    output_len += 1;
                }
                JsonDecodeState::Escape => {
                    let decoded = match byte {
                        b'"' | b'\\' | b'/' => Some(byte),
                        b'b' => Some(0x08),
                        b'f' => Some(0x0c),
                        b'n' => Some(b'\n'),
                        b'r' => Some(b'\r'),
                        b't' => Some(b'\t'),
                        b'u' => {
                            self.state = JsonDecodeState::Unicode {
                                value: 0,
                                digits: 0,
                            };
                            None
                        }
                        _ => {
                            output[output_len] = b'\\';
                            output_len += 1;
                            self.state = JsonDecodeState::Plain;
                            current = Some(byte);
                            None
                        }
                    };
                    if let Some(decoded) = decoded {
                        output[output_len] = decoded;
                        output_len += 1;
                        self.state = JsonDecodeState::Plain;
                    }
                }
                JsonDecodeState::Unicode { value, digits } => {
                    if let Some(nibble) = hex_nibble(byte) {
                        let value = (value << 4) | u16::from(nibble);
                        let digits = digits + 1;
                        if digits == 4 {
                            // Binding credentials are printable ASCII. A
                            // non-ASCII scalar cannot complete a valid key.
                            output[output_len] = u8::try_from(value).unwrap_or(b'?');
                            output_len += 1;
                            self.state = JsonDecodeState::Plain;
                        } else {
                            self.state = JsonDecodeState::Unicode { value, digits };
                        }
                    } else {
                        // Invalid JSON escape: preserve the introducer and let
                        // the raw scanner cover the original bytes.
                        output[output_len] = b'\\';
                        output[output_len + 1] = b'u';
                        output_len += 2;
                        self.state = JsonDecodeState::Plain;
                        current = Some(byte);
                    }
                }
            }
        }
        (output, output_len)
    }

    fn has_pending_escape(&self) -> bool {
        !matches!(self.state, JsonDecodeState::Plain)
    }
}

const STREAM_DECODE_ROUNDS: usize = 16;

struct StreamingDecodePipeline {
    stages: Vec<StreamingUrlDecoder>,
    matchers: Vec<CredentialWindowMatcher>,
    json_matchers: Vec<StreamingJsonDecodePipeline>,
    overflow: StreamingPercentDepthGuard,
}

impl StreamingDecodePipeline {
    fn new(guard: &CredentialExposureGuard, plus_as_space: bool) -> Self {
        Self {
            stages: (0..STREAM_DECODE_ROUNDS)
                .map(|_| StreamingUrlDecoder::new(plus_as_space))
                .collect(),
            matchers: (0..STREAM_DECODE_ROUNDS)
                .map(|_| CredentialWindowMatcher::new(guard))
                .collect(),
            json_matchers: (0..STREAM_DECODE_ROUNDS)
                .map(|_| StreamingJsonDecodePipeline::new(guard))
                .collect(),
            overflow: StreamingPercentDepthGuard::new(),
        }
    }

    fn push(&mut self, byte: u8, guard: &CredentialExposureGuard) -> bool {
        self.push_at(0, byte, guard)
    }

    fn push_at(&mut self, stage: usize, byte: u8, guard: &CredentialExposureGuard) -> bool {
        let (output, output_len) = self.stages[stage].push(byte);
        for decoded in output.into_iter().take(output_len) {
            if self.matchers[stage].push(decoded, guard) {
                return true;
            }
            if self.json_matchers[stage].push(decoded, guard) {
                return true;
            }
            if stage + 1 < self.stages.len() {
                if self.push_at(stage + 1, decoded, guard) {
                    return true;
                }
            } else if self.overflow.push(decoded) {
                // More than the documented normalization depth is suspicious
                // even before a full credential window can be reconstructed.
                return true;
            }
        }
        false
    }

    fn has_partial_match(&self) -> bool {
        self.matchers
            .iter()
            .any(|matcher| matcher.target_prefix_suffix_len() > 0)
            || self
                .json_matchers
                .iter()
                .any(StreamingJsonDecodePipeline::has_partial_match)
            || self
                .stages
                .iter()
                .any(StreamingUrlDecoder::has_pending_escape)
            || self.overflow.has_pending_escape()
    }
}

struct StreamingPercentDepthGuard {
    state: PercentDecodeState,
}

impl StreamingPercentDepthGuard {
    fn new() -> Self {
        Self {
            state: PercentDecodeState::Plain,
        }
    }

    /// Returns true once a complete `%XX` escape survives all supported
    /// decoding rounds, proving the response is still changing at depth 17.
    fn push(&mut self, byte: u8) -> bool {
        match self.state {
            PercentDecodeState::Plain if byte == b'%' => {
                self.state = PercentDecodeState::Percent;
                false
            }
            PercentDecodeState::Plain => false,
            PercentDecodeState::Percent if hex_nibble(byte).is_some() => {
                self.state = PercentDecodeState::High(byte);
                false
            }
            PercentDecodeState::Percent => {
                self.state = if byte == b'%' {
                    PercentDecodeState::Percent
                } else {
                    PercentDecodeState::Plain
                };
                false
            }
            PercentDecodeState::High(_) if hex_nibble(byte).is_some() => {
                self.state = PercentDecodeState::Plain;
                true
            }
            PercentDecodeState::High(_) => {
                self.state = if byte == b'%' {
                    PercentDecodeState::Percent
                } else {
                    PercentDecodeState::Plain
                };
                false
            }
        }
    }

    fn has_pending_escape(&self) -> bool {
        !matches!(self.state, PercentDecodeState::Plain)
    }
}

#[derive(Clone, Copy)]
enum PercentDecodeState {
    Plain,
    Percent,
    High(u8),
}

struct StreamingUrlDecoder {
    plus_as_space: bool,
    state: PercentDecodeState,
}

impl StreamingUrlDecoder {
    fn new(plus_as_space: bool) -> Self {
        Self {
            plus_as_space,
            state: PercentDecodeState::Plain,
        }
    }

    fn push(&mut self, byte: u8) -> ([u8; 3], usize) {
        let mut output = [0; 3];
        let mut output_len = 0;
        let mut current = Some(byte);
        while let Some(byte) = current.take() {
            match self.state {
                PercentDecodeState::Plain if byte == b'%' => {
                    self.state = PercentDecodeState::Percent;
                }
                PercentDecodeState::Plain => {
                    output[output_len] = if self.plus_as_space && byte == b'+' {
                        b' '
                    } else {
                        byte
                    };
                    output_len += 1;
                }
                PercentDecodeState::Percent => {
                    if hex_nibble(byte).is_some() {
                        self.state = PercentDecodeState::High(byte);
                    } else {
                        output[output_len] = b'%';
                        output_len += 1;
                        self.state = PercentDecodeState::Plain;
                        current = Some(byte);
                    }
                }
                PercentDecodeState::High(high) => {
                    if let (Some(high), Some(low)) = (hex_nibble(high), hex_nibble(byte)) {
                        output[output_len] = (high << 4) | low;
                        output_len += 1;
                        self.state = PercentDecodeState::Plain;
                    } else {
                        output[output_len] = b'%';
                        output[output_len + 1] = high;
                        output_len += 2;
                        self.state = PercentDecodeState::Plain;
                        current = Some(byte);
                    }
                }
            }
        }
        (output, output_len)
    }

    fn has_pending_escape(&self) -> bool {
        !matches!(self.state, PercentDecodeState::Plain)
    }
}

struct CredentialWindowMatcher {
    ring: Vec<u8>,
    start: usize,
    rolling_hash: u64,
    highest_power: u64,
    target_len: usize,
    prefix_matched: usize,
}

impl CredentialWindowMatcher {
    fn new(guard: &CredentialExposureGuard) -> Self {
        let highest_power = (1..guard.secret_len).fold(1_u64, |power, _| {
            power.wrapping_mul(CREDENTIAL_ROLLING_HASH_BASE)
        });
        Self {
            ring: Vec::with_capacity(guard.secret_len),
            start: 0,
            rolling_hash: 0,
            highest_power,
            target_len: guard.secret_len,
            prefix_matched: 0,
        }
    }

    fn push(&mut self, byte: u8, guard: &CredentialExposureGuard) -> bool {
        if self.target_len == 0 {
            return false;
        }
        self.prefix_matched = guard.advance_prefix_match(self.prefix_matched, byte);
        if self.ring.len() < self.target_len {
            self.ring.push(byte);
            self.rolling_hash = self
                .rolling_hash
                .wrapping_mul(CREDENTIAL_ROLLING_HASH_BASE)
                .wrapping_add(u64::from(byte) + 1);
            if self.ring.len() < self.target_len {
                return false;
            }
        } else {
            let outgoing = u64::from(self.ring[self.start]) + 1;
            self.ring[self.start] = byte;
            self.start = (self.start + 1) % self.target_len;
            self.rolling_hash = self
                .rolling_hash
                .wrapping_sub(outgoing.wrapping_mul(self.highest_power))
                .wrapping_mul(CREDENTIAL_ROLLING_HASH_BASE)
                .wrapping_add(u64::from(byte) + 1);
        }
        if self.rolling_hash != guard.rolling_hash {
            return false;
        }
        let mut candidate = Zeroizing::new(Vec::with_capacity(self.target_len));
        candidate.extend_from_slice(&self.ring[self.start..]);
        candidate.extend_from_slice(&self.ring[..self.start]);
        bool::from(credential_fingerprint(candidate.as_slice()).ct_eq(&guard.fingerprint))
    }

    fn target_prefix_suffix_len(&self) -> usize {
        self.prefix_matched
    }
}

impl Drop for CredentialWindowMatcher {
    fn drop(&mut self) {
        self.ring.zeroize();
    }
}

const CREDENTIAL_ROLLING_HASH_BASE: u64 = 257;

fn credential_rolling_hash(value: &[u8]) -> u64 {
    value.iter().fold(0_u64, |hash, byte| {
        hash.wrapping_mul(CREDENTIAL_ROLLING_HASH_BASE)
            .wrapping_add(u64::from(*byte) + 1)
    })
}

fn decode_url_component(value: &[u8], plus_as_space: bool) -> Option<Vec<u8>> {
    if !(value.contains(&b'%') || plus_as_space && value.contains(&b'+')) {
        return None;
    }

    let mut decoded = Vec::with_capacity(value.len());
    let mut index = 0;
    while index < value.len() {
        if value[index] == b'%' && index + 2 < value.len() {
            if let (Some(high), Some(low)) =
                (hex_nibble(value[index + 1]), hex_nibble(value[index + 2]))
            {
                decoded.push((high << 4) | low);
                index += 3;
            } else {
                decoded.push(value[index]);
                index += 1;
            }
        } else if plus_as_space && value[index] == b'+' {
            decoded.push(b' ');
            index += 1;
        } else {
            decoded.push(value[index]);
            index += 1;
        }
    }
    Some(decoded)
}

fn decode_json_escapes(value: &[u8]) -> Option<Vec<u8>> {
    if !value.contains(&b'\\') {
        return None;
    }
    let mut decoder = StreamingJsonDecoder::new();
    let mut decoded = Vec::with_capacity(value.len());
    for byte in value {
        let (output, output_len) = decoder.push(*byte);
        decoded.extend(output.into_iter().take(output_len));
    }
    Some(decoded)
}

fn has_incomplete_normalization_suffix(value: &[u8]) -> bool {
    if value.ends_with(b"%")
        || value.len() >= 2
            && value[value.len() - 2] == b'%'
            && hex_nibble(value[value.len() - 1]).is_some()
    {
        return true;
    }

    let start = value.len().saturating_sub(5);
    for index in start..value.len() {
        if value[index] != b'\\' {
            continue;
        }
        let preceding_backslashes = value[..index]
            .iter()
            .rev()
            .take_while(|byte| **byte == b'\\')
            .count();
        if preceding_backslashes % 2 == 1 {
            continue;
        }
        let tail = &value[index + 1..];
        if tail.is_empty()
            || tail.len() <= 4
                && tail[0] == b'u'
                && tail[1..].iter().all(|byte| hex_nibble(*byte).is_some())
        {
            return true;
        }
    }
    false
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

impl fmt::Debug for CredentialExposureGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CredentialExposureGuard([REDACTED])")
    }
}

/// Credential-free ownership frozen at the same lookup linearization point as
/// the upstream route and protected key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenBindingOwnership {
    pub binding_id: String,
    pub agent_module_id: String,
    pub provider_id: String,
}

#[allow(dead_code)] // Its route/key accessors are consumed by Task 4 proxy routing.
impl ResolvedBindingCredential {
    pub fn binding_id(&self) -> &str {
        &self.ownership.binding_id
    }

    pub fn agent_module_id(&self) -> &str {
        &self.ownership.agent_module_id
    }

    pub fn provider_id(&self) -> &str {
        &self.ownership.provider_id
    }

    pub(crate) fn route_app_type(&self) -> &str {
        &self.route_app_type
    }

    pub(crate) fn product_group_id(&self) -> &str {
        &self.product_group_id
    }

    pub(crate) fn runtime_provider(&self) -> &Provider {
        &self.runtime_provider
    }

    pub(crate) fn credential_placement(&self) -> UpstreamCredentialPlacement {
        self.credential_placement
    }

    pub(crate) fn legacy_pricing_provider_id(&self) -> Option<&str> {
        self.legacy_pricing_provider_id.as_deref()
    }

    pub(crate) fn pricing_override(&self) -> &BindingPricingOverride {
        &self.pricing_override
    }

    pub(crate) fn frozen_ownership(&self) -> FrozenBindingOwnership {
        self.ownership.clone()
    }

    pub(crate) fn exposure_guard(&self) -> CredentialExposureGuard {
        CredentialExposureGuard::from_secret(self.secret.as_slice())
    }

    pub(crate) fn expose_secret(&self) -> &[u8] {
        self.secret.as_slice()
    }
}

impl fmt::Debug for ResolvedBindingCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ResolvedBindingCredential([REDACTED])")
    }
}

pub struct BindingCredentialService {
    db: Arc<Database>,
    store: Arc<dyn CredentialStore>,
    lifecycle_lock: CredentialLifecycleLock,
}

impl BindingCredentialService {
    pub fn new(db: Arc<Database>, store: Arc<dyn CredentialStore>) -> Self {
        let lifecycle_lock = CredentialLifecycleLock::new(&db);
        Self {
            db,
            store,
            lifecycle_lock,
        }
    }

    async fn store_put(
        &self,
        slot: String,
        secret: Zeroizing<Vec<u8>>,
        lifecycle_guard: Arc<SharedLifecycleGuard>,
    ) -> Result<(), ()> {
        let store = self.store.clone();
        match tokio::task::spawn_blocking(move || {
            let _lifecycle_guard = lifecycle_guard;
            store.put(&slot, secret.as_slice())
        })
        .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) | Err(_) => {
                log::error!("credential store put failed");
                Err(())
            }
        }
    }

    async fn store_get(&self, slot: String) -> Result<Option<Zeroizing<Vec<u8>>>, ()> {
        let store = self.store.clone();
        match tokio::task::spawn_blocking(move || {
            store.get(&slot).map(|secret| secret.map(Zeroizing::new))
        })
        .await
        {
            Ok(Ok(secret)) => Ok(secret),
            Ok(Err(_)) | Err(_) => {
                log::error!("credential store get failed");
                Err(())
            }
        }
    }

    async fn store_delete(&self, slot: String) -> Result<(), ()> {
        let store = self.store.clone();
        match tokio::task::spawn_blocking(move || store.delete(&slot)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) | Err(_) => {
                log::error!("credential store delete failed");
                Err(())
            }
        }
    }

    async fn cleanup_staging_after_failure(&self, reservation: &CredentialOperationReservation) {
        let Some(staging_slot) = reservation.staging_slot.as_deref() else {
            if self
                .db
                .delete_pending_journal_entry(
                    &reservation.operation_id,
                    &reservation.binding_id,
                    reservation.generation,
                )
                .is_err()
            {
                log::error!("credential pending journal cleanup failed");
            }
            return;
        };
        match self.db.claim_pending_staging_cleanup(reservation) {
            Ok(false) => return,
            Err(_) => {
                log::error!("credential staging cleanup claim failed");
                return;
            }
            Ok(true) => {}
        }
        match self.db.credential_slot_is_active(staging_slot) {
            Ok(false) => {}
            Ok(true) | Err(_) => {
                log::error!("credential staging cleanup safety check failed");
                return;
            }
        }
        if self.store_delete(staging_slot.to_string()).await.is_ok()
            && self.db.finish_claimed_staging_cleanup(reservation).is_err()
        {
            log::error!("credential pending journal cleanup failed");
        }
    }

    async fn finish_published_operation(
        &self,
        reservation: &CredentialOperationReservation,
    ) -> Result<(), ()> {
        if let Some(previous_slot) = reservation.previous_slot.as_deref() {
            match self.db.credential_slot_is_active(previous_slot) {
                Ok(true) => return Err(()),
                Err(_) => {
                    log::error!("credential cleanup safety check failed");
                    return Err(());
                }
                Ok(false) => {}
            }
            self.store_delete(previous_slot.to_string()).await?;
        }
        self.db
            .finish_credential_operation(reservation)
            .map_err(|_| {
                log::error!("credential journal finalization failed");
            })
    }

    async fn cleanup_provider_staging_after_failure(
        &self,
        reservation: &ProviderCredentialOperationReservation,
    ) {
        let Some(staging_slot) = reservation.staging_slot.as_deref() else {
            if self
                .db
                .delete_pending_provider_journal_entry(
                    &reservation.operation_id,
                    &reservation.provider_id,
                    reservation.generation,
                )
                .is_err()
            {
                log::error!("provider credential pending journal cleanup failed");
            }
            return;
        };
        match self.db.claim_pending_provider_staging_cleanup(reservation) {
            Ok(false) => return,
            Err(_) => {
                log::error!("provider credential staging cleanup claim failed");
                return;
            }
            Ok(true) => {}
        }
        match self.db.provider_credential_slot_is_active(staging_slot) {
            Ok(false) => {}
            Ok(true) | Err(_) => {
                log::error!("provider credential staging cleanup safety check failed");
                return;
            }
        }
        if self.store_delete(staging_slot.to_string()).await.is_ok()
            && self
                .db
                .finish_claimed_provider_staging_cleanup(reservation)
                .is_err()
        {
            log::error!("provider credential pending journal cleanup failed");
        }
    }

    async fn finish_published_provider_operation(
        &self,
        reservation: &ProviderCredentialOperationReservation,
    ) -> Result<(), ()> {
        if let Some(previous_slot) = reservation.previous_slot.as_deref() {
            match self.db.provider_credential_slot_is_active(previous_slot) {
                Ok(true) => return Err(()),
                Err(_) => {
                    log::error!("provider credential cleanup safety check failed");
                    return Err(());
                }
                Ok(false) => {}
            }
            self.store_delete(previous_slot.to_string()).await?;
        }
        self.db
            .finish_provider_credential_operation(reservation)
            .map_err(|_| {
                log::error!("provider credential journal finalization failed");
            })
    }

    async fn mutate_provider_api_key(
        &self,
        provider_id: &str,
        expected_version: u64,
        api_key: SecretString,
        kind: CredentialMutationKind,
    ) -> Result<UsageProviderView, AppError> {
        if !provider_credential_is_acceptable(api_key.expose_bytes()) {
            return Err(public_error("credential_required"));
        }
        let lifecycle_guard = Arc::new(self.lifecycle_lock.shared().await.map_err(|_| {
            log::error!("credential lifecycle lock failed");
            public_error("credential_unavailable")
        })?);
        let fingerprint = provider_credential_fingerprint(api_key.expose_bytes());
        let reservation = self
            .db
            .reserve_provider_credential_operation(
                provider_id,
                expected_version,
                kind,
                Some(&fingerprint),
            )
            .map_err(normalize_db_error)?;
        let staging_slot = reservation
            .staging_slot
            .as_deref()
            .ok_or_else(|| public_error("credential_unavailable"))?
            .to_string();
        let protected_secret = Zeroizing::new(api_key.expose_bytes().to_vec());
        if self
            .store_put(staging_slot, protected_secret, lifecycle_guard.clone())
            .await
            .is_err()
        {
            self.cleanup_provider_staging_after_failure(&reservation)
                .await;
            return Err(public_error("credential_unavailable"));
        }
        if let Err(error) = self
            .db
            .publish_provider_credential_operation(&reservation, Some(&fingerprint))
        {
            self.cleanup_provider_staging_after_failure(&reservation)
                .await;
            return Err(normalize_db_error(error));
        }
        let _ = self.finish_published_provider_operation(&reservation).await;
        self.provider_view(provider_id).await
    }

    pub async fn set_provider_api_key(
        &self,
        provider_id: &str,
        expected_version: u64,
        api_key: SecretString,
    ) -> Result<UsageProviderView, AppError> {
        self.mutate_provider_api_key(
            provider_id,
            expected_version,
            api_key,
            CredentialMutationKind::Set,
        )
        .await
    }

    pub async fn replace_provider_api_key(
        &self,
        provider_id: &str,
        expected_version: u64,
        api_key: SecretString,
    ) -> Result<UsageProviderView, AppError> {
        self.mutate_provider_api_key(
            provider_id,
            expected_version,
            api_key,
            CredentialMutationKind::Replace,
        )
        .await
    }

    pub async fn clear_provider_api_key(
        &self,
        provider_id: &str,
        expected_version: u64,
    ) -> Result<UsageProviderView, AppError> {
        let _lifecycle_guard = self.lifecycle_lock.shared().await.map_err(|_| {
            log::error!("credential lifecycle lock failed");
            public_error("credential_unavailable")
        })?;
        let reservation = self
            .db
            .reserve_provider_credential_operation(
                provider_id,
                expected_version,
                CredentialMutationKind::Clear,
                None,
            )
            .map_err(normalize_db_error)?;
        if let Err(error) = self
            .db
            .publish_provider_credential_operation(&reservation, None)
        {
            self.cleanup_provider_staging_after_failure(&reservation)
                .await;
            return Err(normalize_db_error(error));
        }
        let _ = self.finish_published_provider_operation(&reservation).await;
        self.provider_view(provider_id).await
    }

    async fn provider_credential_status(
        &self,
        snapshot: &ProviderCredentialSnapshot,
    ) -> BindingCredentialStatus {
        let (Some(fingerprint), Some(slot)) = (
            snapshot.fingerprint.as_deref(),
            snapshot.credential_slot.as_deref(),
        ) else {
            return if snapshot.fingerprint.is_none() && snapshot.credential_slot.is_none() {
                BindingCredentialStatus::Missing
            } else {
                BindingCredentialStatus::Unavailable
            };
        };
        if fingerprint.len() != 32 || snapshot.credential_version == 0 {
            return BindingCredentialStatus::Unavailable;
        }
        let Ok(Some(secret)) = self.store_get(slot.to_string()).await else {
            return BindingCredentialStatus::Unavailable;
        };
        let actual = provider_credential_fingerprint(secret.as_slice());
        if bool::from(actual.as_slice().ct_eq(fingerprint)) {
            BindingCredentialStatus::Configured
        } else {
            BindingCredentialStatus::Unavailable
        }
    }

    async fn provider_view(&self, provider_id: &str) -> Result<UsageProviderView, AppError> {
        self.list_usage_providers()
            .await?
            .into_iter()
            .find(|provider| provider.id == provider_id)
            .filter(|provider| {
                provider.system_auth_kind == Some(SystemProviderAuthKind::ProviderApiKey)
            })
            .ok_or_else(|| public_error("unsupported_auth"))
    }

    pub async fn list_usage_providers(&self) -> Result<Vec<UsageProviderView>, AppError> {
        let mut providers = self.db.list_usage_providers().map_err(normalize_db_error)?;
        for provider in &mut providers {
            if provider.system_auth_kind != Some(SystemProviderAuthKind::ProviderApiKey) {
                continue;
            }
            let snapshot = self
                .db
                .provider_credential_snapshot(&provider.id)
                .map_err(normalize_db_error)?
                .ok_or_else(|| public_error("credential_unavailable"))?;
            provider.upstream_credential_status = self.provider_credential_status(&snapshot).await;
            provider.upstream_credential_version = snapshot.credential_version;
            provider.can_clear_upstream_credential =
                snapshot.fingerprint.is_some() && snapshot.credential_slot.is_some();
        }
        let verified_bindings = self.list_agent_provider_bindings(None).await?;
        for provider in &mut providers {
            provider.bindings = verified_bindings
                .iter()
                .filter(|binding| binding.provider_id == provider.id)
                .cloned()
                .collect();
        }
        Ok(providers)
    }

    async fn mutate_api_key(
        &self,
        binding_id: &str,
        expected_version: u64,
        api_key: SecretString,
        kind: CredentialMutationKind,
    ) -> Result<AgentProviderBindingView, AppError> {
        if !binding_credential_is_acceptable(api_key.expose_bytes()) {
            return Err(public_error("credential_required"));
        }
        let lifecycle_guard = Arc::new(self.lifecycle_lock.shared().await.map_err(|_| {
            log::error!("credential lifecycle lock failed");
            public_error("credential_unavailable")
        })?);
        let fingerprint = credential_fingerprint(api_key.expose_bytes());
        let reservation = self
            .db
            .reserve_credential_operation(binding_id, expected_version, kind, Some(&fingerprint))
            .map_err(normalize_db_error)?;
        let staging_slot = reservation
            .staging_slot
            .as_deref()
            .ok_or_else(|| public_error("credential_unavailable"))?
            .to_string();
        let protected_secret = Zeroizing::new(api_key.expose_bytes().to_vec());
        if self
            .store_put(staging_slot, protected_secret, lifecycle_guard.clone())
            .await
            .is_err()
        {
            self.cleanup_staging_after_failure(&reservation).await;
            return Err(public_error("credential_unavailable"));
        }
        if let Err(error) = self
            .db
            .publish_credential_operation(&reservation, Some(&fingerprint))
        {
            self.cleanup_staging_after_failure(&reservation).await;
            return Err(normalize_db_error(error));
        }
        // The new generation is already authoritative. Old-slot cleanup is
        // retryable and must not make callers retry the mutation with a stale
        // expected version.
        let _ = self.finish_published_operation(&reservation).await;
        self.binding_view(binding_id).await
    }

    pub async fn set_binding_api_key(
        &self,
        binding_id: &str,
        expected_version: u64,
        api_key: SecretString,
    ) -> Result<AgentProviderBindingView, AppError> {
        self.mutate_api_key(
            binding_id,
            expected_version,
            api_key,
            CredentialMutationKind::Set,
        )
        .await
    }

    pub async fn replace_binding_api_key(
        &self,
        binding_id: &str,
        expected_version: u64,
        api_key: SecretString,
    ) -> Result<AgentProviderBindingView, AppError> {
        self.mutate_api_key(
            binding_id,
            expected_version,
            api_key,
            CredentialMutationKind::Replace,
        )
        .await
    }

    pub async fn clear_binding_api_key(
        &self,
        binding_id: &str,
        expected_version: u64,
    ) -> Result<AgentProviderBindingView, AppError> {
        let _lifecycle_guard = self.lifecycle_lock.shared().await.map_err(|_| {
            log::error!("credential lifecycle lock failed");
            public_error("credential_unavailable")
        })?;
        let reservation = self
            .db
            .reserve_credential_operation(
                binding_id,
                expected_version,
                CredentialMutationKind::Clear,
                None,
            )
            .map_err(normalize_db_error)?;
        if let Err(error) = self.db.publish_credential_operation(&reservation, None) {
            self.cleanup_staging_after_failure(&reservation).await;
            return Err(normalize_db_error(error));
        }
        let _ = self.finish_published_operation(&reservation).await;
        match self.binding_view(binding_id).await {
            Ok(view) => Ok(view),
            Err(_) => {
                log::error!("credential binding view failed closed");
                self.db
                    .credential_binding_fail_closed_view(binding_id)
                    .map_err(normalize_db_error)
            }
        }
    }

    pub async fn delete_binding(
        &self,
        binding_id: &str,
        expected_version: u64,
    ) -> Result<(), AppError> {
        let _lifecycle_guard = self.lifecycle_lock.shared().await.map_err(|_| {
            log::error!("credential lifecycle lock failed");
            public_error("credential_unavailable")
        })?;
        let reservation = self
            .db
            .reserve_credential_operation(
                binding_id,
                expected_version,
                CredentialMutationKind::Delete,
                None,
            )
            .map_err(normalize_db_error)?;
        if let Err(error) = self.db.publish_credential_operation(&reservation, None) {
            self.cleanup_staging_after_failure(&reservation).await;
            return Err(normalize_db_error(error));
        }
        self.finish_published_operation(&reservation)
            .await
            .map_err(|_| public_error("credential_unavailable"))
    }

    async fn credential_status(
        &self,
        snapshot: &CredentialBindingSnapshot,
    ) -> BindingCredentialStatus {
        match snapshot.auth_mode {
            BindingAuthMode::SessionOnly | BindingAuthMode::ManagedAuth => {
                if snapshot.fingerprint.is_none() && snapshot.credential_slot.is_none() {
                    BindingCredentialStatus::NotRequired
                } else {
                    // A provider may change auth modes after a direct key was
                    // configured. Keep the leftover visible as unavailable so
                    // callers can explicitly clear it.
                    BindingCredentialStatus::Unavailable
                }
            }
            BindingAuthMode::Unsupported => BindingCredentialStatus::Unavailable,
            BindingAuthMode::DirectApiKey => {
                let (Some(fingerprint), Some(slot)) = (
                    snapshot.fingerprint.as_deref(),
                    snapshot.credential_slot.as_deref(),
                ) else {
                    return if snapshot.fingerprint.is_none() && snapshot.credential_slot.is_none() {
                        BindingCredentialStatus::Missing
                    } else {
                        BindingCredentialStatus::Unavailable
                    };
                };
                if fingerprint.len() != 32 {
                    return BindingCredentialStatus::Unavailable;
                }
                let Ok(Some(secret)) = self.store_get(slot.to_string()).await else {
                    return BindingCredentialStatus::Unavailable;
                };
                let actual = credential_fingerprint(secret.as_slice());
                if bool::from(actual.as_slice().ct_eq(fingerprint)) {
                    BindingCredentialStatus::Configured
                } else {
                    BindingCredentialStatus::Unavailable
                }
            }
        }
    }

    async fn binding_provider_credential_status(
        &self,
        snapshot: &CredentialBindingSnapshot,
    ) -> BindingCredentialStatus {
        if !snapshot.is_fixed_system_api() {
            return BindingCredentialStatus::NotRequired;
        }
        let (Some(fingerprint), Some(slot)) = (
            snapshot.provider_fingerprint.as_deref(),
            snapshot.provider_credential_slot.as_deref(),
        ) else {
            return if snapshot.provider_fingerprint.is_none()
                && snapshot.provider_credential_slot.is_none()
            {
                BindingCredentialStatus::Missing
            } else {
                BindingCredentialStatus::Unavailable
            };
        };
        if fingerprint.len() != 32 || snapshot.provider_credential_version == 0 {
            return BindingCredentialStatus::Unavailable;
        }
        let Ok(Some(secret)) = self.store_get(slot.to_string()).await else {
            return BindingCredentialStatus::Unavailable;
        };
        let actual = provider_credential_fingerprint(secret.as_slice());
        if bool::from(actual.as_slice().ct_eq(fingerprint)) {
            BindingCredentialStatus::Configured
        } else {
            BindingCredentialStatus::Unavailable
        }
    }

    async fn binding_view(&self, binding_id: &str) -> Result<AgentProviderBindingView, AppError> {
        let snapshot = self
            .db
            .credential_binding_snapshot(binding_id)
            .map_err(normalize_db_error)?
            .ok_or_else(|| public_error("binding_not_found"))?;
        let status = self.credential_status(&snapshot).await;
        let provider_status = self.binding_provider_credential_status(&snapshot).await;
        Ok(snapshot.into_view(status, provider_status))
    }

    pub async fn list_agent_provider_bindings(
        &self,
        agent_module_id: Option<&str>,
    ) -> Result<Vec<AgentProviderBindingView>, AppError> {
        let snapshots = self
            .db
            .credential_binding_snapshots(agent_module_id)
            .map_err(normalize_db_error)?;
        let mut views = Vec::with_capacity(snapshots.len());
        for snapshot in snapshots {
            let status = self.credential_status(&snapshot).await;
            let provider_status = self.binding_provider_credential_status(&snapshot).await;
            views.push(snapshot.into_view(status, provider_status));
        }
        Ok(views)
    }

    pub async fn create_system_api_binding(
        &self,
        input: AgentProviderBindingInput,
    ) -> Result<AgentProviderBindingView, AppError> {
        if input.id.is_some() {
            return Err(public_error("invalid_binding"));
        }
        let provider = self
            .db
            .get_usage_provider(&input.provider_id)
            .map_err(normalize_db_error)?
            .ok_or_else(|| public_error("invalid_binding"))?;
        let preset_key = provider.system_preset_key.as_deref();
        if !is_fixed_api_preset(preset_key)
            || preset_key
                .and_then(|preset| system_binding_route_protocol(preset, &input.agent_module_id))
                .is_none()
        {
            return Err(public_error("invalid_binding"));
        }

        let requested_enabled = input.enabled;
        let reserved = self
            .db
            .save_agent_provider_binding(&AgentProviderBindingInput {
                id: None,
                agent_module_id: input.agent_module_id.clone(),
                provider_id: input.provider_id.clone(),
                enabled: false,
            })
            .map_err(normalize_db_error)?;
        if let Err(error) = self
            .set_binding_api_key(
                &reserved.id,
                reserved.credential_version,
                generate_local_binding_key(),
            )
            .await
        {
            let _ = self
                .db
                .delete_agent_provider_binding_metadata(&reserved.id, reserved.credential_version);
            return Err(error);
        }
        if requested_enabled {
            if let Err(error) = self
                .db
                .save_agent_provider_binding(&AgentProviderBindingInput {
                    id: Some(reserved.id.clone()),
                    agent_module_id: input.agent_module_id,
                    provider_id: input.provider_id,
                    enabled: true,
                })
            {
                let _ = self.delete_binding(&reserved.id, 1).await;
                return Err(normalize_db_error(error));
            }
        }
        self.binding_view(&reserved.id).await
    }

    pub async fn reveal_local_binding_key(
        &self,
        binding_id: &str,
        expected_version: u64,
    ) -> Result<LocalBindingKeyReveal, AppError> {
        let initial = self
            .db
            .credential_binding_snapshot(binding_id)
            .map_err(normalize_db_error)?
            .ok_or_else(|| public_error("binding_not_found"))?;
        if !initial.is_fixed_system_api() {
            return Err(public_error("unsupported_auth"));
        }
        if initial.credential_version != expected_version {
            return Err(public_error("credential_conflict"));
        }
        let fingerprint = initial
            .fingerprint
            .as_deref()
            .filter(|fingerprint| fingerprint.len() == 32)
            .ok_or_else(|| public_error("credential_required"))?;
        let slot = initial
            .credential_slot
            .as_deref()
            .ok_or_else(|| public_error("credential_required"))?
            .to_string();
        let secret = self
            .store_get(slot.clone())
            .await
            .map_err(|_| public_error("credential_unavailable"))?
            .ok_or_else(|| public_error("credential_unavailable"))?;
        if !binding_credential_is_acceptable(secret.as_slice())
            || !bool::from(
                credential_fingerprint(secret.as_slice())
                    .as_slice()
                    .ct_eq(fingerprint),
            )
        {
            return Err(public_error("credential_unavailable"));
        }
        let authoritative = self
            .db
            .credential_binding_snapshot(binding_id)
            .map_err(normalize_db_error)?
            .ok_or_else(|| public_error("binding_not_found"))?;
        if !authoritative.is_fixed_system_api()
            || authoritative.credential_version != expected_version
            || authoritative.credential_slot.as_deref() != Some(slot.as_str())
            || authoritative.fingerprint.as_deref() != Some(fingerprint)
        {
            return Err(public_error("credential_conflict"));
        }
        let local_key = String::from_utf8(secret.to_vec())
            .map_err(|_| public_error("credential_unavailable"))?;
        Ok(LocalBindingKeyReveal {
            binding_id: binding_id.to_string(),
            credential_version: expected_version,
            local_key,
        })
    }

    pub async fn rotate_local_binding_key(
        &self,
        binding_id: &str,
        expected_version: u64,
    ) -> Result<LocalBindingKeyReveal, AppError> {
        let snapshot = self
            .db
            .credential_binding_snapshot(binding_id)
            .map_err(normalize_db_error)?
            .ok_or_else(|| public_error("binding_not_found"))?;
        if !snapshot.is_fixed_system_api() {
            return Err(public_error("unsupported_auth"));
        }
        if snapshot.credential_version != expected_version {
            return Err(public_error("credential_conflict"));
        }
        let rotated = self
            .replace_binding_api_key(binding_id, expected_version, generate_local_binding_key())
            .await?;
        self.reveal_local_binding_key(binding_id, rotated.credential_version)
            .await
    }

    pub async fn ensure_fixed_api_binding_local_keys(
        &self,
    ) -> Result<Vec<AgentProviderBindingView>, AppError> {
        let snapshots = self
            .db
            .credential_binding_snapshots(None)
            .map_err(normalize_db_error)?;
        let mut unavailable_bindings = HashSet::new();
        for snapshot in snapshots {
            if !snapshot.is_fixed_system_api()
                || snapshot.agent_archived_at.is_some()
                || snapshot.fingerprint.is_some()
                || snapshot.credential_slot.is_some()
            {
                continue;
            }
            if let Err(error) = self
                .set_binding_api_key(
                    &snapshot.id,
                    snapshot.credential_version,
                    generate_local_binding_key(),
                )
                .await
            {
                unavailable_bindings.insert(snapshot.id.clone());
                log::error!(
                    "fixed API binding local credential generation failed: {}",
                    error
                );
            }
        }
        let mut views = self.list_agent_provider_bindings(None).await?;
        for view in &mut views {
            if unavailable_bindings.contains(&view.id) {
                view.local_credential_status = BindingCredentialStatus::Unavailable;
                view.credential_status = BindingCredentialStatus::Unavailable;
                view.effective_enabled = false;
            }
        }
        Ok(views)
    }

    pub async fn resolve_binding_api_key(
        &self,
        api_key: SecretString,
    ) -> Result<ResolvedBindingCredential, AppError> {
        if !binding_credential_is_acceptable(api_key.expose_bytes()) {
            return Err(public_error("credential_required"));
        }
        let inbound_fingerprint = credential_fingerprint(api_key.expose_bytes());
        drop(api_key);
        let snapshot = self
            .db
            .credential_binding_by_fingerprint(&inbound_fingerprint)
            .map_err(normalize_db_error)?
            .ok_or_else(|| public_error("binding_not_found"))?;
        if !snapshot.enabled
            || !snapshot.provider_enabled
            || snapshot.agent_archived_at.is_some()
            || snapshot.auth_mode != BindingAuthMode::DirectApiKey
        {
            return Err(public_error("invalid_binding"));
        }
        let db_fingerprint = snapshot
            .fingerprint
            .as_deref()
            .filter(|fingerprint| fingerprint.len() == 32)
            .ok_or_else(|| public_error("credential_unavailable"))?;
        let slot = snapshot
            .credential_slot
            .as_deref()
            .ok_or_else(|| public_error("credential_unavailable"))?;
        if !bool::from(inbound_fingerprint.as_slice().ct_eq(db_fingerprint)) {
            return Err(public_error("credential_unavailable"));
        }
        let initial_binding_id = snapshot.id.clone();
        let initial_credential_version = snapshot.credential_version;
        let slot = slot.to_string();
        let secret = self
            .store_get(slot.clone())
            .await
            .map_err(|_| public_error("credential_unavailable"))?
            .ok_or_else(|| public_error("credential_unavailable"))?;
        if !binding_credential_is_acceptable(secret.as_slice()) {
            return Err(public_error("credential_unavailable"));
        }
        let stored_fingerprint = credential_fingerprint(secret.as_slice());
        if !bool::from(stored_fingerprint.as_slice().ct_eq(db_fingerprint))
            || !bool::from(stored_fingerprint.ct_eq(&inbound_fingerprint))
        {
            return Err(public_error("credential_unavailable"));
        }
        // The protected-store read is asynchronous, so a rotation/clear/delete
        // can publish while it is in flight. Re-read the authoritative binding
        // after the secret has been verified; this second lookup is the
        // pre-send linearization point. A request that crossed it before a
        // mutation keeps its frozen projection, while a mutation that completed
        // first makes the old generation fail locally.
        let snapshot = self
            .db
            .credential_binding_by_fingerprint(&inbound_fingerprint)
            .map_err(normalize_db_error)?
            .ok_or_else(|| public_error("binding_not_found"))?;
        if snapshot.id != initial_binding_id
            || snapshot.credential_version != initial_credential_version
            || snapshot.credential_slot.as_deref() != Some(slot.as_str())
            || !snapshot.enabled
            || !snapshot.provider_enabled
            || snapshot.agent_archived_at.is_some()
            || snapshot.auth_mode != BindingAuthMode::DirectApiKey
        {
            return Err(public_error("invalid_binding"));
        }
        let route_app_type = snapshot
            .route_app_type
            .ok_or_else(|| public_error("invalid_binding"))?;
        let route_config = snapshot
            .route_config
            .ok_or_else(|| public_error("invalid_binding"))?;
        let usage_provider_id = snapshot.provider_id.clone();
        let legacy_pricing_provider_id = snapshot
            .legacy_migration_linked
            .then(|| snapshot.legacy_provider_id.clone())
            .flatten();
        let projection = build_binding_route_projection(
            &route_app_type,
            &usage_provider_id,
            snapshot.provider_name,
            route_config,
            snapshot.quota_config,
            snapshot.legacy_migration_linked,
            snapshot.legacy_provider,
        )
        .map_err(|_| public_error("invalid_binding"))?;
        let exposure_guard = CredentialExposureGuard::from_secret(secret.as_slice());
        let runtime_provider = serde_json::to_value(&projection.runtime_provider)
            .map_err(|_| public_error("invalid_binding"))?;
        if exposure_guard.contains_json_value(&runtime_provider) {
            return Err(public_error("invalid_binding"));
        }
        let ownership = FrozenBindingOwnership {
            binding_id: snapshot.id,
            agent_module_id: snapshot.agent_module_id,
            provider_id: snapshot.provider_id,
        };
        if exposure_guard.contains(&route_app_type)
            || exposure_guard.contains(&snapshot.product_group_id)
            || exposure_guard.contains(&ownership.binding_id)
            || exposure_guard.contains(&ownership.agent_module_id)
            || exposure_guard.contains(&ownership.provider_id)
            || legacy_pricing_provider_id
                .as_deref()
                .is_some_and(|value| exposure_guard.contains(value))
            || projection
                .pricing_override
                .cost_multiplier
                .as_deref()
                .is_some_and(|value| exposure_guard.contains(value))
            || projection
                .pricing_override
                .pricing_model_source
                .as_deref()
                .is_some_and(|value| exposure_guard.contains(value))
        {
            return Err(public_error("invalid_binding"));
        }
        Ok(ResolvedBindingCredential {
            ownership,
            route_app_type,
            product_group_id: snapshot.product_group_id,
            runtime_provider: projection.runtime_provider,
            credential_placement: projection.credential_placement,
            legacy_pricing_provider_id,
            pricing_override: projection.pricing_override,
            secret,
        })
    }

    /// Store-backed authorization gate used before handlers collect/decompress
    /// or parse a potentially large body. This deliberately does not replace
    /// the final resolution immediately before request construction; races with
    /// disable/rotation remain fail-closed at that authoritative boundary.
    pub(crate) async fn preflight_binding_api_key(
        &self,
        api_key: &SecretString,
        expected_route_app_type: &str,
    ) -> Result<(), AppError> {
        if !binding_credential_is_acceptable(api_key.expose_bytes()) {
            return Err(public_error("credential_required"));
        }
        let inbound_fingerprint = credential_fingerprint(api_key.expose_bytes());
        let snapshot = self
            .db
            .credential_binding_by_fingerprint(&inbound_fingerprint)
            .map_err(normalize_db_error)?
            .ok_or_else(|| public_error("binding_not_found"))?;
        let fingerprint_matches = snapshot.fingerprint.as_deref().is_some_and(|fingerprint| {
            fingerprint.len() == inbound_fingerprint.len()
                && bool::from(inbound_fingerprint.as_slice().ct_eq(fingerprint))
        });
        let slot = snapshot.credential_slot.as_deref();
        if !snapshot.enabled
            || !snapshot.provider_enabled
            || snapshot.agent_archived_at.is_some()
            || snapshot.auth_mode != BindingAuthMode::DirectApiKey
            || snapshot.route_app_type.as_deref() != Some(expected_route_app_type)
            || slot.is_none()
            || !fingerprint_matches
        {
            return Err(public_error("invalid_binding"));
        }
        let protected_secret = self
            .store_get(slot.unwrap_or_default().to_string())
            .await
            .map_err(|_| public_error("credential_unavailable"))?
            .ok_or_else(|| public_error("credential_unavailable"))?;
        if !binding_credential_is_acceptable(protected_secret.as_slice()) {
            return Err(public_error("credential_unavailable"));
        }
        let protected_fingerprint = credential_fingerprint(protected_secret.as_slice());
        if !bool::from(protected_fingerprint.ct_eq(&inbound_fingerprint)) {
            return Err(public_error("credential_unavailable"));
        }
        Ok(())
    }

    async fn reconcile_entry(&self, mut entry: CredentialJournalEntry) -> Result<(), ()> {
        if entry.status == "pending" {
            let snapshot = self
                .db
                .credential_reconcile_state(&entry.binding_id)
                .map_err(|_| ())?
                .ok_or(())?;
            let published = match entry.kind {
                CredentialMutationKind::Set | CredentialMutationKind::Replace => {
                    snapshot.credential_version == entry.generation
                        && snapshot.credential_slot.as_deref() == entry.staging_slot.as_deref()
                        && snapshot.has_fingerprint
                }
                CredentialMutationKind::Clear | CredentialMutationKind::Delete => {
                    snapshot.credential_version == entry.generation
                        && snapshot.credential_slot.is_none()
                        && !snapshot.has_fingerprint
                        && !snapshot.enabled
                }
            };
            if published {
                let status = if entry.previous_slot.is_some() {
                    "cleanup"
                } else {
                    "committed"
                };
                self.db
                    .promote_pending_credential_operation(&entry.operation_id, status)
                    .map_err(|_| ())?;
                entry.status = status.to_string();
            } else {
                if let Some(staging_slot) = entry.staging_slot.as_deref() {
                    if self
                        .db
                        .credential_slot_is_active(staging_slot)
                        .map_err(|_| ())?
                    {
                        return Err(());
                    }
                    let reservation = CredentialOperationReservation {
                        operation_id: entry.operation_id.clone(),
                        binding_id: entry.binding_id.clone(),
                        kind: entry.kind,
                        expected_version: entry.generation.saturating_sub(1),
                        generation: entry.generation,
                        staging_slot: entry.staging_slot.clone(),
                        previous_slot: entry.previous_slot.clone(),
                        previous_fingerprint: None,
                    };
                    if !self
                        .db
                        .claim_pending_staging_cleanup(&reservation)
                        .map_err(|_| ())?
                    {
                        return Err(());
                    }
                    if self
                        .db
                        .credential_slot_is_active(staging_slot)
                        .map_err(|_| ())?
                    {
                        return Err(());
                    }
                    self.store_delete(staging_slot.to_string()).await?;
                    self.db
                        .finish_claimed_staging_cleanup(&reservation)
                        .map_err(|_| ())?;
                } else {
                    self.db
                        .delete_pending_journal_entry(
                            &entry.operation_id,
                            &entry.binding_id,
                            entry.generation,
                        )
                        .map_err(|_| ())?;
                }
                return Ok(());
            }
        }
        if entry.status == "cleanup" {
            if let Some(previous_slot) = entry.previous_slot.as_deref() {
                if self
                    .db
                    .credential_slot_is_active(previous_slot)
                    .map_err(|_| ())?
                {
                    return Err(());
                }
                self.store_delete(previous_slot.to_string()).await?;
            }
        }
        self.db.finish_journal_entry(&entry).map_err(|_| ())
    }

    async fn reconcile_provider_entry(
        &self,
        mut entry: ProviderCredentialJournalEntry,
    ) -> Result<(), ()> {
        if entry.status == "pending" {
            let snapshot = self
                .db
                .provider_credential_reconcile_state(&entry.provider_id)
                .map_err(|_| ())?
                .ok_or(())?;
            let published = match entry.kind {
                CredentialMutationKind::Set | CredentialMutationKind::Replace => {
                    snapshot.credential_version == entry.generation
                        && snapshot.credential_slot.as_deref() == entry.staging_slot.as_deref()
                        && snapshot.has_fingerprint
                }
                CredentialMutationKind::Clear => {
                    snapshot.credential_version == entry.generation
                        && snapshot.credential_slot.is_none()
                        && !snapshot.has_fingerprint
                }
                CredentialMutationKind::Delete => false,
            };
            if published {
                let status = if entry.previous_slot.is_some() {
                    "cleanup"
                } else {
                    "committed"
                };
                self.db
                    .promote_pending_provider_credential_operation(&entry.operation_id, status)
                    .map_err(|_| ())?;
                entry.status = status.to_string();
            } else {
                if let Some(staging_slot) = entry.staging_slot.as_deref() {
                    if self
                        .db
                        .provider_credential_slot_is_active(staging_slot)
                        .map_err(|_| ())?
                    {
                        return Err(());
                    }
                    let reservation = ProviderCredentialOperationReservation {
                        operation_id: entry.operation_id.clone(),
                        provider_id: entry.provider_id.clone(),
                        kind: entry.kind,
                        expected_version: entry.generation.saturating_sub(1),
                        generation: entry.generation,
                        staging_slot: entry.staging_slot.clone(),
                        previous_slot: entry.previous_slot.clone(),
                        previous_fingerprint: None,
                    };
                    if !self
                        .db
                        .claim_pending_provider_staging_cleanup(&reservation)
                        .map_err(|_| ())?
                    {
                        return Err(());
                    }
                    if self
                        .db
                        .provider_credential_slot_is_active(staging_slot)
                        .map_err(|_| ())?
                    {
                        return Err(());
                    }
                    self.store_delete(staging_slot.to_string()).await?;
                    self.db
                        .finish_claimed_provider_staging_cleanup(&reservation)
                        .map_err(|_| ())?;
                } else {
                    self.db
                        .delete_pending_provider_journal_entry(
                            &entry.operation_id,
                            &entry.provider_id,
                            entry.generation,
                        )
                        .map_err(|_| ())?;
                }
                return Ok(());
            }
        }
        if entry.status == "cleanup" {
            if let Some(previous_slot) = entry.previous_slot.as_deref() {
                if self
                    .db
                    .provider_credential_slot_is_active(previous_slot)
                    .map_err(|_| ())?
                {
                    return Err(());
                }
                self.store_delete(previous_slot.to_string()).await?;
            }
        }
        self.db
            .finish_provider_journal_entry(&entry)
            .map_err(|_| ())
    }

    async fn reconcile_provider_entries_locked(&self) -> Result<(), AppError> {
        let initial_entries = self
            .db
            .provider_credential_journal_entries()
            .map_err(normalize_db_error)?;
        let mut previous_count = initial_entries.len();
        let max_passes = previous_count.saturating_add(1).max(1);
        let mut next_entries = Some(initial_entries);
        for _ in 0..max_passes {
            let entries = match next_entries.take() {
                Some(entries) => entries,
                None => self
                    .db
                    .provider_credential_journal_entries()
                    .map_err(normalize_db_error)?,
            };
            if entries.is_empty() {
                break;
            }
            for entry in entries {
                if self.reconcile_provider_entry(entry).await.is_err() {
                    log::error!("provider credential startup reconciliation step failed");
                }
            }
            let remaining = self
                .db
                .provider_credential_journal_entries()
                .map_err(normalize_db_error)?
                .len();
            if remaining == 0 {
                break;
            }
            if remaining >= previous_count {
                break;
            }
            previous_count = remaining;
        }
        if self
            .db
            .provider_credential_journal_entries()
            .map_err(normalize_db_error)?
            .is_empty()
        {
            Ok(())
        } else {
            Err(public_error("credential_unavailable"))
        }
    }

    async fn reconcile_locked(&self) -> Result<(), AppError> {
        self.reconcile_provider_entries_locked().await?;
        let initial_entries = self
            .db
            .credential_journal_entries()
            .map_err(normalize_db_error)?;
        let mut previous_count = initial_entries.len();
        let max_passes = previous_count.saturating_add(1).max(1);
        let mut next_entries = Some(initial_entries);
        for _ in 0..max_passes {
            let entries = match next_entries.take() {
                Some(entries) => entries,
                None => self
                    .db
                    .credential_journal_entries()
                    .map_err(normalize_db_error)?,
            };
            if entries.is_empty() {
                break;
            }
            for entry in entries {
                if self.reconcile_entry(entry).await.is_err() {
                    log::error!("credential startup reconciliation step failed");
                }
            }
            let remaining = self
                .db
                .credential_journal_entries()
                .map_err(normalize_db_error)?
                .len();
            if remaining == 0 {
                break;
            }
            if remaining >= previous_count {
                break;
            }
            previous_count = remaining;
        }
        let failed = !self
            .db
            .credential_journal_entries()
            .map_err(normalize_db_error)?
            .is_empty();
        // Audit every active pointer after journal cleanup. Status remains a
        // derived fail-closed view; reconciliation never guesses or rewrites a
        // missing protected value.
        if self.list_agent_provider_bindings(None).await.is_err() {
            log::error!("credential active-pointer audit failed");
        }
        if self.list_usage_providers().await.is_err() {
            log::error!("provider credential active-pointer audit failed");
        }
        if failed {
            Err(public_error("credential_unavailable"))
        } else {
            Ok(())
        }
    }

    pub async fn reconcile_startup(&self) -> Result<(), AppError> {
        let _lifecycle_guard = self.lifecycle_lock.exclusive().await.map_err(|_| {
            log::error!("credential lifecycle lock failed");
            public_error("credential_unavailable")
        })?;
        self.reconcile_locked().await
    }

    pub(crate) async fn run_exclusive_database_change<T, Fut>(
        &self,
        operation: Fut,
    ) -> Result<T, AppError>
    where
        Fut: std::future::Future<Output = Result<T, AppError>>,
    {
        let _lifecycle_guard = self.lifecycle_lock.exclusive().await.map_err(|_| {
            log::error!("credential lifecycle lock failed");
            public_error("credential_unavailable")
        })?;
        self.reconcile_locked().await?;
        let result = operation.await;
        self.reconcile_locked().await?;
        result
    }

    pub(crate) async fn run_exclusive_blocking_database_change<T, Operation>(
        &self,
        operation: Operation,
    ) -> Result<T, AppError>
    where
        T: Send + 'static,
        Operation: FnOnce() -> Result<T, AppError> + Send + 'static,
    {
        let lifecycle_guard = Arc::new(self.lifecycle_lock.exclusive().await.map_err(|_| {
            log::error!("credential lifecycle lock failed");
            public_error("credential_unavailable")
        })?);
        self.reconcile_locked().await?;
        let detached_guard = lifecycle_guard.clone();
        let result = match tokio::task::spawn_blocking(move || {
            let _lifecycle_guard = detached_guard;
            operation()
        })
        .await
        {
            Ok(result) => result,
            Err(_) => {
                log::error!("protected database change task failed");
                Err(public_error("credential_unavailable"))
            }
        };
        self.reconcile_locked().await?;
        result
    }
}
