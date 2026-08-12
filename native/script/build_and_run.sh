#!/bin/zsh
set -euo pipefail

MODE=${1:-run}
PREVIEW_DATA_MODE=fixture
if [[ "$MODE" == "--live" || "$MODE" == "live" ]]; then
  MODE=run
  PREVIEW_DATA_MODE=bridge
fi
NATIVE_ROOT=${0:A:h:h}
PROJECT_ROOT=${NATIVE_ROOT:h}
DERIVED_DATA="$NATIVE_ROOT/.build/xcode-preview"
APP_PATH="$DERIVED_DATA/Build/Products/Preview/LLM Usage Bar Native Preview.app"
EXECUTABLE_PATH="$APP_PATH/Contents/MacOS/LLM Usage Bar Native Preview"
BUNDLE_ID="com.llmusagebar.native.preview"

build_native() {
  xcodebuild \
    -project "$NATIVE_ROOT/LLMUsageBarNative.xcodeproj" \
    -scheme "Native Preview" \
    -configuration Preview \
    -derivedDataPath "$DERIVED_DATA" \
    CODE_SIGNING_ALLOWED=NO \
    build
}

stop_native_preview() {
  while IFS= read -r pid; do
    /bin/kill "$pid"
  done < <(/usr/bin/pgrep -f "$EXECUTABLE_PATH" || true)
}

open_native_preview() {
  /usr/bin/open -n "$APP_PATH" --args "--preview-data=$PREVIEW_DATA_MODE"
}

run_bridge_e2e() {
  cd "$PROJECT_ROOT"
  pnpm rust -- build --manifest-path src-tauri/Cargo.toml --bin llm-usage-bar
  local target_dir
  target_dir=$(node scripts/with-cargo-target.mjs cargo metadata \
    --manifest-path src-tauri/Cargo.toml \
    --no-deps \
    --format-version 1 \
    | node -e 'let input=""; process.stdin.on("data", chunk => input += chunk); process.stdin.on("end", () => console.log(JSON.parse(input).target_directory));')
  node native/script/bridge_e2e.mjs "$target_dir/debug/llm-usage-bar"
}

case "$MODE" in
  --bridge-e2e|bridge-e2e)
    run_bridge_e2e
    exit 0
    ;;
  run|--debug|debug|--logs|logs|--telemetry|telemetry|--verify|verify)
    ;;
  *)
    echo "usage: $0 [run|--live|--debug|--logs|--telemetry|--verify|--bridge-e2e]" >&2
    exit 2
    ;;
esac

build_native
stop_native_preview

case "$MODE" in
  run)
    open_native_preview
    ;;
  --debug|debug)
    lldb -- "$EXECUTABLE_PATH" "--preview-data=$PREVIEW_DATA_MODE"
    ;;
  --logs|logs)
    open_native_preview
    /usr/bin/log stream --info --style compact --predicate "process == \"LLM Usage Bar Native Preview\""
    ;;
  --telemetry|telemetry)
    open_native_preview
    /usr/bin/log stream --info --style compact --predicate "subsystem == \"$BUNDLE_ID\""
    ;;
  --verify|verify)
    open_native_preview
    /bin/sleep 1
    /usr/bin/pgrep -f "$EXECUTABLE_PATH" >/dev/null
    ;;
esac
