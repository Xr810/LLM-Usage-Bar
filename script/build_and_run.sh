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
SIGNING_IDENTITY="${LLM_USAGE_BAR_SIGNING_IDENTITY:-LLM Usage Bar Local Development}"
LOGIN_KEYCHAIN="${HOME}/Library/Keychains/login.keychain-db"

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

require_signing_identity() {
  if [[ ! -f "$LOGIN_KEYCHAIN" ]]; then
    echo "Login keychain not found: $LOGIN_KEYCHAIN" >&2
    exit 1
  fi
  if ! /usr/bin/security find-identity -p codesigning -v "$LOGIN_KEYCHAIN" 2>/dev/null \
    | /usr/bin/grep -F "\"$SIGNING_IDENTITY\"" >/dev/null; then
    echo "Missing trusted code-signing identity: $SIGNING_IDENTITY" >&2
    echo "Import the local development identity into the login keychain first." >&2
    exit 1
  fi
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
  if ! /usr/bin/grep -F "Identifier=$BUNDLE_ID" <<<"$signing_info" >/dev/null; then
    echo "Unexpected bundle identifier for $app_path" >&2
    echo "$signing_info" >&2
    return 1
  fi
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
