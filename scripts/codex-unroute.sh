#!/usr/bin/env bash
# Codex 本地路由的逃生门(设计文档决定 37)。
#
# app 挂掉时,app 里的按钮点不了,所以这条回退路径必须是纯 shell、不依赖
# app 存活。本脚本只碰 ~/.codex/config.toml,绝不读写 ~/.codex/auth.json。
#
# 用法:
#   ./scripts/codex-unroute.sh <provider_id>
#   例如:./scripts/codex-unroute.sh packyapi
#
# 行为(两条路,动手前都会先把当前文件备份成 config.toml.bak-<时间戳>-before-unroute):
#   1. 目录里有 config.toml.bak-*-before-router(T7 写指针前留下的备份)时,
#      还原最近的一份,<provider_id> 参数被忽略;
#   2. 没有备份时,把顶层 model_provider 改成 <provider_id>。
#
# 可用 CODEX_CONFIG_DIR 环境变量覆盖配置目录(默认 $HOME/.codex)。
set -euo pipefail

usage() {
  echo "用法: $0 <provider_id>" >&2
  echo "例如: $0 packyapi" >&2
  echo "把 \$HOME/.codex/config.toml 的顶层 model_provider 改回直连;" >&2
  echo "若目录里有 config.toml.bak-*-before-router 备份,则优先还原最近一份(忽略参数)。" >&2
}

[ "$#" -eq 1 ] || { usage; exit 64; }
PROVIDER_ID="$1"
case "$PROVIDER_ID" in
  ""|*[!A-Za-z0-9._-]*)
    echo "provider id 只允许字母数字和 . _ -,拿到的是: $PROVIDER_ID" >&2
    exit 64
    ;;
esac

CONFIG_DIR="${CODEX_CONFIG_DIR:-$HOME/.codex}"
CONFIG="$CONFIG_DIR/config.toml"

[ -f "$CONFIG" ] || { echo "找不到 $CONFIG,没有可改的东西。" >&2; exit 0; }
cd "$CONFIG_DIR" || exit 1

# 先备份当前文件再动手,与 Rust 侧同一备份习惯。
BACKUP="config.toml.bak-$(date +%Y%m%d-%H%M%S)-before-unroute"
cp "$CONFIG" "$BACKUP"
echo "已把当前配置备份到 $CONFIG_DIR/$BACKUP"

# 路径 1:找最近一份 -before-router 备份并还原。
shopt -s nullglob
backups=(config.toml.bak-*-before-router)
if [ "${#backups[@]}" -gt 0 ]; then
  latest="$(ls -1t "${backups[@]}" | head -n 1)"
  tmp="config.toml.tmp-restore-$$"
  cp "$latest" "$tmp"
  mv "$tmp" "$CONFIG"
  echo "已用最近的备份还原 model_provider: $CONFIG_DIR/$latest"
  exit 0
fi

# 路径 2:没有备份,把顶层 model_provider 改成参数指定的值。
# 只碰顶层的 model_provider(行首无缩进);表内的同名键不动。
# 没有这一行时插在第一个 [table] 之前;连表都没有就追加到文件末尾。
tmp="config.toml.tmp-unroute-$$"
awk -v id="$PROVIDER_ID" '
  /^model_provider[ \t]*=/ { replaced = 1 }
  { lines[NR] = $0 }
  END {
    inserted = 0
    for (i = 1; i <= NR; i++) {
      line = lines[i]
      if (!inserted && !replaced && line ~ /^[ \t]*\[/) {
        print "model_provider = \"" id "\""
        inserted = 1
      }
      if (line ~ /^model_provider[ \t]*=/) {
        line = "model_provider = \"" id "\""
      }
      print line
    }
    if (!replaced && !inserted) print "model_provider = \"" id "\""
  }
' "$CONFIG" > "$tmp"
mv "$tmp" "$CONFIG"
echo "已把顶层 model_provider 改成 $PROVIDER_ID"
echo "如果这不是你想要的,可执行: cp $CONFIG_DIR/$BACKUP $CONFIG"
