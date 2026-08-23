#!/usr/bin/env bash
# HIG 改造计划 P0 的三条硬规则。CI 与本地都跑。
#
# 规则本身在 docs/design/2026-08-13-native-hig-restyle-plan.md §P0,
# 主题结构见 docs/design/2026-08-23-router-panel-visual-direction.md。
#
# 用法:native/script/lint_design.sh
set -uo pipefail

cd "$(dirname "$0")/.." || exit 2

SOURCES="Sources"
DESIGN_SYSTEM="Sources/NativeUI/NativeDesignSystem.swift"
failed=0

# 注释行不算违规 —— 规则描述本身常常包含被禁的写法。
strip_comments() {
  grep -v ':[[:space:]]*//' | grep -v ':[[:space:]]*\*'
}

report() {
  local rule="$1" hits="$2"
  if [ -n "$hits" ]; then
    echo "✗ $rule"
    echo "$hits" | sed 's/^/    /'
    failed=1
  else
    echo "✓ $rule"
  fi
}

# 1. 禁止写死字号。语义刻度见 NativeTextStyle ——
#    macOS 没有 Dynamic Type,用语义刻度的理由是「那就是苹果自己的刻度」,不是自动缩放。
hits=$(grep -rn '\.font(\.system(size:' "$SOURCES" 2>/dev/null | strip_comments | grep -v 'lint:allow-size')
report "禁止 .font(.system(size:)（需豁免时行尾加 // lint:allow-size 并写明理由）" "$hits"

# 2. 圆角必须 continuous。普通圆弧一眼看出是网页。
hits=$(grep -rn 'cornerRadius' "$SOURCES" 2>/dev/null \
  | strip_comments \
  | grep -v 'style: *\.continuous' \
  | grep -v 'NativeRadius' \
  | grep -v 'lint:allow-radius')
report "圆角必须带 style: .continuous（或走 NativeRadius.shape）" "$hits"

# 3. 字面颜色只允许出现在设计系统文件里。
hits=$(grep -rn 'Color(red:\|Color(hex:\|NSColor(red:\|Color(\.sRGB' "$SOURCES" 2>/dev/null \
  | strip_comments \
  | grep -v "^$DESIGN_SYSTEM:")
report "字面颜色只能出现在 $DESIGN_SYSTEM" "$hits"

echo
if [ "$failed" -ne 0 ]; then
  echo "设计 lint 未通过。"
  exit 1
fi
echo "设计 lint 全部通过。"
