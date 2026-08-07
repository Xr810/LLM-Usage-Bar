#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-run}"
if [[ $# -gt 1 ]]; then
  echo "usage: $0 [run|--debug|--logs|--telemetry|--verify|--build-only]" >&2
  exit 2
fi

case "$MODE" in
  run|--debug|debug|--logs|logs|--telemetry|telemetry|--verify|verify|--build-only|build-only)
    ;;
  *)
    echo "usage: $0 [run|--debug|--logs|--telemetry|--verify|--build-only]" >&2
    exit 2
    ;;
esac

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This local build/install entrypoint only supports macOS." >&2
  exit 1
fi

APP_NAME="LLM Usage Bar"
PROCESS_NAME="llm-usage-bar"
BUNDLE_ID="com.llmusagebar.desktop"
SIGNING_IDENTITY="${LLM_USAGE_BAR_SIGNING_IDENTITY:-LLM Usage Bar Local Development Signing 2026}"
SIGNING_IDENTITY_PINNED="${LLM_USAGE_BAR_SIGNING_IDENTITY+x}"
# Used only when the preferred identity's private key turns out to be
# unreachable; see require_signing_identity. Named explicitly rather than
# "whatever is in the login keychain" so the fallback stays auditable.
FALLBACK_SIGNING_IDENTITY="LLM Usage Bar Local Development"
EXPECTED_SIGNING_IDENTITY_SHA1="${LLM_USAGE_BAR_SIGNING_IDENTITY_SHA1:-}"
EXPECTED_SIGNING_ROOT_AUTHORITY="${LLM_USAGE_BAR_SIGNING_ROOT_AUTHORITY:-}"
EXPECTED_SIGNING_ROOT_SHA1="${LLM_USAGE_BAR_SIGNING_ROOT_SHA1:-}"
LOGIN_SIGNING_KEYCHAIN="${HOME}/Library/Keychains/login.keychain-db"
DEFAULT_SIGNING_KEYCHAIN="$LOGIN_SIGNING_KEYCHAIN"
DEDICATED_SIGNING_KEYCHAIN="${HOME}/Library/Keychains/llm-usage-bar-signing.keychain-db"
# Prefer the dedicated keychain only when this script can actually open it.
# Choosing it on file existence alone means codesign later blocks on a GUI
# unlock prompt for a password the build has no way to supply — which reads as
# a hung build, not a configuration problem. Pin LLM_USAGE_BAR_SIGNING_KEYCHAIN
# to use it interactively after unlocking it yourself.
if [[ -f "$DEDICATED_SIGNING_KEYCHAIN" \
  && "${LLM_USAGE_BAR_SIGNING_KEYCHAIN_PASSWORD+x}" == "x" ]]; then
  DEFAULT_SIGNING_KEYCHAIN="$DEDICATED_SIGNING_KEYCHAIN"
fi
SIGNING_KEYCHAIN="${LLM_USAGE_BAR_SIGNING_KEYCHAIN:-$DEFAULT_SIGNING_KEYCHAIN}"
SIGNING_KEYCHAIN_PINNED="${LLM_USAGE_BAR_SIGNING_KEYCHAIN+x}"
RESOLVED_SIGNING_IDENTITY_SHA1=""

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="$ROOT_DIR/release/tauri-target"
BUILT_APP="$TARGET_DIR/release/bundle/macos/$APP_NAME.app"
INSTALL_APP="/Applications/$APP_NAME.app"
APP_BINARY="$INSTALL_APP/Contents/MacOS/$PROCESS_NAME"
STAGING_APP="/Applications/.$APP_NAME.codex-staging-$$.app"
BACKUP_APP="/Applications/.$APP_NAME.codex-backup-$$.app"

cleanup_staging() {
  if [[ -e "$STAGING_APP" ]]; then
    /bin/rm -rf -- "$STAGING_APP"
  fi
}
trap cleanup_staging EXIT

stop_running_app() {
  /usr/bin/pkill -x "$PROCESS_NAME" >/dev/null 2>&1 || true
  for _ in {1..20}; do
    if ! /usr/bin/pgrep -x "$PROCESS_NAME" >/dev/null 2>&1; then
      return
    fi
    sleep 0.1
  done
  echo "Could not stop the running $APP_NAME process." >&2
  exit 1
}

# `security find-identity -v` reports a certificate as a valid identity without
# ever reaching for its private key, so a locked keychain sails through the name
# check and the build only dies six minutes later inside codesign. Sign a
# throwaway Mach-O to find out now, before the compile.
#
# Bounded by a watchdog: codesign against a locked keychain does not fail fast,
# it raises a GUI unlock dialog and waits. An unattended build must treat that
# as "cannot sign" rather than hanging, and must not leave a prompt sitting in
# front of whoever happens to be at the machine.
SIGNING_PROBE_TIMEOUT_SECONDS=5

signing_identity_can_sign() {
  local identity_hash="$1"
  local keychain="$2"
  local probe
  local probe_pid
  local waited=0
  local status=0

  probe="$(/usr/bin/mktemp -t llm-usage-bar-signing-probe)" || return 1
  if ! /bin/cp /usr/bin/true "$probe" 2>/dev/null; then
    /bin/rm -f -- "$probe"
    return 1
  fi

  /usr/bin/codesign --force --sign "$identity_hash" \
    --keychain "$keychain" "$probe" >/dev/null 2>&1 &
  probe_pid=$!
  while /bin/kill -0 "$probe_pid" 2>/dev/null; do
    if (( waited >= SIGNING_PROBE_TIMEOUT_SECONDS * 10 )); then
      /bin/kill -TERM "$probe_pid" 2>/dev/null || true
      sleep 0.2
      /bin/kill -KILL "$probe_pid" 2>/dev/null || true
      wait "$probe_pid" 2>/dev/null || true
      /bin/rm -f -- "$probe"
      return 1
    fi
    sleep 0.1
    waited=$((waited + 1))
  done
  wait "$probe_pid" 2>/dev/null || status=1
  /bin/rm -f -- "$probe"
  return "$status"
}

resolve_signing_identity() {
  local wanted_identity="$1"
  local keychain="$2"
  local identities
  local identity_hash
  local identity_name
  local match_count=0
  local matched_hash=""

  [[ -f "$keychain" ]] || return 1
  identities="$(/usr/bin/security find-identity -p codesigning -v "$keychain" 2>/dev/null)" || return 1
  while IFS= read -r identity_line; do
    if [[ "$identity_line" =~ ^[[:space:]]*[0-9]+\)[[:space:]]+([[:xdigit:]]+)[[:space:]]+\"([^\"]+)\"[[:space:]]*$ ]]; then
      identity_hash="${BASH_REMATCH[1]}"
      identity_name="${BASH_REMATCH[2]}"
      [[ "$identity_name" == "$wanted_identity" ]] || continue
      match_count=$((match_count + 1))
      matched_hash="$identity_hash"
    fi
  done <<<"$identities"

  [[ "$match_count" -eq 1 ]] || return 1
  signing_identity_can_sign "$matched_hash" "$keychain" || return 1
  RESOLVED_SIGNING_IDENTITY_SHA1="$matched_hash"
  return 0
}

require_signing_identity() {
  local identities
  local identity_hash
  local identity_name
  local match_count=0
  local matched_hash=""
  local matched_name=""

  # Preferred identity first; fall back only when its key is genuinely out of
  # reach and the caller pinned neither the identity nor the keychain.
  if resolve_signing_identity "$SIGNING_IDENTITY" "$SIGNING_KEYCHAIN"; then
    return 0
  fi
  if [[ "$SIGNING_IDENTITY_PINNED" != "x" && "$SIGNING_KEYCHAIN_PINNED" != "x" \
    && "$SIGNING_IDENTITY" != "$FALLBACK_SIGNING_IDENTITY" ]] \
    && resolve_signing_identity "$FALLBACK_SIGNING_IDENTITY" "$LOGIN_SIGNING_KEYCHAIN"; then
    echo "WARNING: cannot sign with '$SIGNING_IDENTITY' from $SIGNING_KEYCHAIN --" >&2
    echo "WARNING: it is absent there, or its private key is unreachable." >&2
    echo "WARNING: Falling back to '$FALLBACK_SIGNING_IDENTITY' in" >&2
    echo "WARNING: $LOGIN_SIGNING_KEYCHAIN. The build IS signed, but with a" >&2
    echo "WARNING: different identity than the default -- set" >&2
    echo "WARNING: LLM_USAGE_BAR_SIGNING_IDENTITY to silence this." >&2
    SIGNING_IDENTITY="$FALLBACK_SIGNING_IDENTITY"
    SIGNING_KEYCHAIN="$LOGIN_SIGNING_KEYCHAIN"
    return 0
  fi

  if [[ ! -f "$SIGNING_KEYCHAIN" ]]; then
    echo "Signing keychain not found: $SIGNING_KEYCHAIN" >&2
    exit 1
  fi
  if [[ "${LLM_USAGE_BAR_SIGNING_KEYCHAIN_PASSWORD+x}" == "x" ]]; then
    if ! /usr/bin/security unlock-keychain \
      -p "$LLM_USAGE_BAR_SIGNING_KEYCHAIN_PASSWORD" "$SIGNING_KEYCHAIN"; then
      echo "Could not unlock signing keychain: $SIGNING_KEYCHAIN" >&2
      exit 1
    fi
  fi

  if ! identities="$(/usr/bin/security find-identity -p codesigning -v "$SIGNING_KEYCHAIN" 2>/dev/null)"; then
    echo "Could not inspect code-signing identities in $SIGNING_KEYCHAIN" >&2
    exit 1
  fi
  while IFS= read -r identity_line; do
    if [[ "$identity_line" =~ ^[[:space:]]*[0-9]+\)[[:space:]]+([[:xdigit:]]+)[[:space:]]+\"([^\"]+)\"[[:space:]]*$ ]]; then
      identity_hash="${BASH_REMATCH[1]}"
      identity_name="${BASH_REMATCH[2]}"
      [[ "$identity_name" == "$SIGNING_IDENTITY" ]] || continue
      match_count=$((match_count + 1))
      matched_hash="$identity_hash"
      matched_name="$identity_name"
    fi
  done <<<"$identities"

  if [[ "$match_count" -ne 1 ]]; then
    echo "Expected exactly one trusted code-signing identity named: $SIGNING_IDENTITY" >&2
    echo "Found $match_count matching identities in $SIGNING_KEYCHAIN." >&2
    echo "Unlock the keychain or set LLM_USAGE_BAR_SIGNING_IDENTITY explicitly." >&2
    exit 1
  fi

  # Exactly one certificate by that name, yet resolve_signing_identity already
  # declined it: the private key is unreachable, not the certificate missing.
  if ! signing_identity_can_sign "$matched_hash" "$SIGNING_KEYCHAIN"; then
    echo "Found '$SIGNING_IDENTITY' in $SIGNING_KEYCHAIN, but codesign cannot" >&2
    echo "use its private key — the keychain is locked." >&2
    echo "Either unlock it:" >&2
    echo "  security unlock-keychain $SIGNING_KEYCHAIN" >&2
    echo "or build with an identity you can actually reach:" >&2
    echo "  LLM_USAGE_BAR_SIGNING_IDENTITY=\"$FALLBACK_SIGNING_IDENTITY\" \\" >&2
    echo "  LLM_USAGE_BAR_SIGNING_KEYCHAIN=\"$LOGIN_SIGNING_KEYCHAIN\" \\" >&2
    echo "  pnpm build:local:mac" >&2
    exit 1
  fi

  SIGNING_IDENTITY="$matched_name"
  RESOLVED_SIGNING_IDENTITY_SHA1="$matched_hash"
  if [[ -n "$EXPECTED_SIGNING_IDENTITY_SHA1" \
    && "$RESOLVED_SIGNING_IDENTITY_SHA1" != "$EXPECTED_SIGNING_IDENTITY_SHA1" ]]; then
    echo "Unexpected SHA-1 for signing identity: $SIGNING_IDENTITY" >&2
    echo "Actual: $RESOLVED_SIGNING_IDENTITY_SHA1" >&2
    echo "Expected: $EXPECTED_SIGNING_IDENTITY_SHA1" >&2
    exit 1
  fi
}

certificate_sha1() {
  /usr/bin/openssl x509 -inform DER -in "$1" -noout -fingerprint -sha1 \
    | /usr/bin/awk -F= '{ value = toupper($2); gsub(":", "", value); print value }'
}

cleanup_embedded_chain() {
  local chain_dir="$1"

  /bin/rm -rf -- "$chain_dir"
}

verify_embedded_chain() {
  local app_path="$1"
  local chain_dir
  local certificate_prefix
  local leaf_sha1
  local last_certificate
  local root_sha1
  local certificate_count=0

  chain_dir="$(/usr/bin/mktemp -d "${TMPDIR:-/private/tmp}/llm-usage-bar-signature.XXXXXX")"
  certificate_prefix="$chain_dir/certificate"

  if ! /usr/bin/codesign -d --extract-certificates="$certificate_prefix" "$app_path" >/dev/null 2>&1; then
    cleanup_embedded_chain "$chain_dir"
    echo "Could not extract the embedded signing chain from $app_path" >&2
    return 1
  fi
  while [[ -f "${certificate_prefix}${certificate_count}" ]]; do
    last_certificate="${certificate_prefix}${certificate_count}"
    certificate_count=$((certificate_count + 1))
  done
  if [[ "$certificate_count" -eq 0 ]]; then
    cleanup_embedded_chain "$chain_dir"
    echo "Expected an embedded signing certificate in $app_path" >&2
    return 1
  fi

  leaf_sha1="$(certificate_sha1 "${certificate_prefix}0")"
  if [[ "$leaf_sha1" != "$RESOLVED_SIGNING_IDENTITY_SHA1" ]]; then
    cleanup_embedded_chain "$chain_dir"
    echo "Unexpected embedded signing certificate in $app_path" >&2
    echo "Leaf SHA-1: $leaf_sha1" >&2
    echo "Expected: $RESOLVED_SIGNING_IDENTITY_SHA1" >&2
    return 1
  fi
  if [[ -n "$EXPECTED_SIGNING_ROOT_SHA1" ]]; then
    if [[ "$certificate_count" -lt 2 ]]; then
      cleanup_embedded_chain "$chain_dir"
      echo "Expected an embedded certificate chain in $app_path" >&2
      return 1
    fi
    root_sha1="$(certificate_sha1 "$last_certificate")"
    if [[ "$root_sha1" != "$EXPECTED_SIGNING_ROOT_SHA1" ]]; then
      cleanup_embedded_chain "$chain_dir"
      echo "Unexpected embedded root certificate in $app_path" >&2
      echo "Root SHA-1: $root_sha1" >&2
      echo "Expected: $EXPECTED_SIGNING_ROOT_SHA1" >&2
      return 1
    fi
  fi
  # This proves local X.509/code-signing trust. Apple platform distribution
  # still requires an Apple-issued identity and notarization.
  if ! /usr/bin/security verify-cert -L -p codeSign \
    -c "${certificate_prefix}0" -k "$SIGNING_KEYCHAIN" -q; then
    cleanup_embedded_chain "$chain_dir"
    echo "macOS rejected the embedded code-signing chain in $app_path" >&2
    return 1
  fi

  cleanup_embedded_chain "$chain_dir"
}

verify_signed_app() {
  local app_path="$1"
  local signing_info

  /usr/bin/codesign --verify --deep --strict --verbose=2 "$app_path"
  signing_info="$(/usr/bin/codesign -d --verbose=4 "$app_path" 2>&1)"
  if ! /usr/bin/grep -F "Authority=$SIGNING_IDENTITY" <<<"$signing_info" >/dev/null; then
    echo "Unexpected signing authority for $app_path" >&2
    echo "$signing_info" >&2
    return 1
  fi
  if [[ -n "$EXPECTED_SIGNING_ROOT_AUTHORITY" ]]; then
    if ! /usr/bin/grep -F "Authority=$EXPECTED_SIGNING_ROOT_AUTHORITY" <<<"$signing_info" >/dev/null; then
      echo "Missing expected root authority for $app_path" >&2
      echo "$signing_info" >&2
      return 1
    fi
  fi
  if ! /usr/bin/grep -F "Identifier=$BUNDLE_ID" <<<"$signing_info" >/dev/null; then
    echo "Unexpected bundle identifier for $app_path" >&2
    echo "$signing_info" >&2
    return 1
  fi
  verify_embedded_chain "$app_path"
}

build_app() {
  require_signing_identity
  echo "Building $APP_NAME in worktree-local target: $TARGET_DIR"
  (
    cd "$ROOT_DIR"
    CI=true APPLE_SIGNING_IDENTITY="$SIGNING_IDENTITY" \
      pnpm tauri -- build --bundles app
  )
  if [[ ! -d "$BUILT_APP" ]]; then
    echo "Expected app bundle was not produced: $BUILT_APP" >&2
    exit 1
  fi
  verify_signed_app "$BUILT_APP"
  echo "Verified signed build: $BUILT_APP"
  /usr/bin/codesign -d -r- "$BUILT_APP" 2>&1
}

install_app() {
  cleanup_staging
  /usr/bin/ditto "$BUILT_APP" "$STAGING_APP"
  verify_signed_app "$STAGING_APP"

  if [[ -e "$BACKUP_APP" ]]; then
    echo "Refusing to overwrite unexpected backup path: $BACKUP_APP" >&2
    exit 1
  fi
  if [[ -e "$INSTALL_APP" ]]; then
    /bin/mv "$INSTALL_APP" "$BACKUP_APP"
  fi

  if ! /bin/mv "$STAGING_APP" "$INSTALL_APP"; then
    if [[ -e "$BACKUP_APP" && ! -e "$INSTALL_APP" ]]; then
      /bin/mv "$BACKUP_APP" "$INSTALL_APP"
    fi
    echo "Failed to install $INSTALL_APP; restored the previous app when possible." >&2
    exit 1
  fi

  if ! verify_signed_app "$INSTALL_APP"; then
    /bin/rm -rf -- "$INSTALL_APP"
    if [[ -e "$BACKUP_APP" ]]; then
      /bin/mv "$BACKUP_APP" "$INSTALL_APP"
    fi
    echo "Installed bundle failed verification; restored the previous app." >&2
    exit 1
  fi

  if [[ -e "$BACKUP_APP" ]]; then
    /bin/rm -rf -- "$BACKUP_APP"
  fi
  echo "Installed verified app: $INSTALL_APP"
}

open_app() {
  /usr/bin/open -n "$INSTALL_APP"
}

stop_running_app
build_app

if [[ "$MODE" == "--build-only" || "$MODE" == "build-only" ]]; then
  exit 0
fi

install_app

case "$MODE" in
  run)
    open_app
    ;;
  --debug|debug)
    /usr/bin/lldb -- "$APP_BINARY"
    ;;
  --logs|logs)
    open_app
    /usr/bin/log stream --info --style compact --predicate "process == \"$PROCESS_NAME\""
    ;;
  --telemetry|telemetry)
    open_app
    /usr/bin/log stream --info --style compact --predicate "subsystem == \"$BUNDLE_ID\""
    ;;
  --verify|verify)
    open_app
    sleep 2
    /usr/bin/pgrep -x "$PROCESS_NAME" >/dev/null
    echo "$APP_NAME is running."
    ;;
esac
