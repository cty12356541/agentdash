#!/usr/bin/env bash
# agentdash · opencode-kit 安装器(W7-004)
# 前提:agentdash 二进制在 PATH。用法:./install.sh [目标项目目录]
# 动作:
#   1. AGENTDASH.md 标记段幂等合入 <目标>/AGENTS.md(opencode 原生读取)
#   2. 插件复制进 <目标>/.opencode/plugins/agentdash.js(启动自动加载)
#   3. <目标>/.gitignore 幂等追加 .agentdash/
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
target="${1:-$PWD}"
if [ ! -d "$target" ]; then
  echo "[agentdash] 目标目录不存在: $target" >&2
  exit 1
fi

if ! command -v agentdash >/dev/null 2>&1; then
  echo "[agentdash] 未找到 agentdash —— 插件直调二进制,需要它先在 PATH。" >&2
  echo "  1) cargo install --path <agentdash 仓库根>   2) Releases 下载放入 PATH" >&2
  exit 1
fi

# --- AGENTDASH.md 标记段幂等合入 AGENTS.md(harness 单源,C3)---
merge_instructions() {
  # $1 = 指令文件(CLAUDE.md / AGENTS.md)。块经文件传入 awk(赋值前缀而非
  # -v:BSD awk 对 -v 值做转义处理,多行块会炸——生态 dogfood 实测)
  local file="$1" block_file="$here/../shared/AGENTDASH.md" tmp
  if [ ! -f "$file" ]; then
    cp "$block_file" "$file"
    echo "[agentdash] instructions created: $file"
  elif grep -q 'agentdash:begin' "$file"; then
    # 已有标记段:整段替换(v1 内容刷新)
    tmp="$file.tmp-agentdash"
    awk '
      /<!-- agentdash:begin/ {
        skip = 1
        while ((getline line < block_file) > 0) print line
        close(block_file)
        next
      }
      /<!-- agentdash:end/ { skip = 0; next }
      skip == 0 { print }
    ' block_file="$block_file" "$file" > "$tmp"
    mv "$tmp" "$file"
    echo "[agentdash] instructions refreshed: $file"
  else
    # 无标记段:末尾追加(先补末行换行,防粘行)
    tmp="$file.tmp-agentdash"
    { cat "$file"; [ -s "$file" ] && [ -n "$(tail -c1 "$file")" ] && printf '\n'; cat "$block_file"; } > "$tmp"
    mv "$tmp" "$file"
    echo "[agentdash] instructions appended: $file"
  fi
}
merge_instructions "$target/AGENTS.md"

# --- 插件复制(启动自动加载,幂等覆盖)---
plugin_dir="$target/.opencode/plugins"
mkdir -p "$plugin_dir"
cp "$here/plugins/agentdash.js" "$plugin_dir/agentdash.js"
echo "[agentdash] plugin installed: $plugin_dir/agentdash.js"

# --- .gitignore 幂等追加 .agentdash/ ---
gitignore="$target/.gitignore"
if [ ! -f "$gitignore" ] || ! grep -qE '^[[:space:]]*\.agentdash/?[[:space:]]*$' "$gitignore"; then
  if [ -f "$gitignore" ] && [ -s "$gitignore" ] && [ -n "$(tail -c1 "$gitignore")" ]; then
    printf '\n' >> "$gitignore"
  fi
  printf '.agentdash/\n' >> "$gitignore"
fi

echo "[agentdash] opencode-kit installed into $target"
echo "[agentdash] 提醒:子代理边界事件 opencode 插件 API 暂未提供,在跑 agent 面板仅 claude-code/codex 宿主可见"
