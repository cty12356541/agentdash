#!/usr/bin/env bash
# agentdash · antigravity-kit 安装器(W14;Google Antigravity 宿主)
# 前提:agentdash 二进制在 PATH。
# 用法:
#   ./install.sh [--global]        # 缺省推荐:本机级注册(~/.gemini/AGENTS.md + ~/.gemini/config/plugins/agentdash/)
#   ./install.sh <目标项目目录>     # 可选项目级注册(<目标>/AGENTS.md + <目标>/.agents/)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if ! command -v agentdash >/dev/null 2>&1; then
  echo "[agentdash] 未找到 agentdash —— hook 直调二进制,需要它先在 PATH。" >&2
  echo "  1) cargo install --path <agentdash 仓库根>   2) Releases 下载放入 PATH" >&2
  exit 1
fi

# --- 标记段幂等合入 AGENTS.md($1=目标文件, $2=模板块文件)---
merge_instructions() {
  local file="$1" block_file="$2" tmp
  mkdir -p "$(dirname "$file")"
  if [ ! -f "$file" ]; then
    cp "$block_file" "$file"
    echo "[agentdash] instructions created: $file"
  elif grep -q 'agentdash:begin' "$file"; then
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
    tmp="$file.tmp-agentdash"
    { cat "$file"; [ -s "$file" ] && [ -n "$(tail -c1 "$file")" ] && printf '\n'; cat "$block_file"; } > "$tmp"
    mv "$tmp" "$file"
    echo "[agentdash] instructions appended: $file"
  fi
}

# ======================= 模式 A:本机级全局注册(缺省 / --global)=======================
if [ $# -eq 0 ] || [ "${1:-}" = "--global" ] || [ "${1:-}" = "-g" ]; then
  gemini_home="${HOME}/.gemini"
  plugin_dir="${gemini_home}/config/plugins/agentdash"

  # 1. 合入本机全局 ~/.gemini/AGENTS.md(Antigravity user_global globalScope)
  merge_instructions "${gemini_home}/AGENTS.md" "$here/AGENTS.global.md"

  # 2. 同步本机全局插件包 ~/.gemini/config/plugins/agentdash/(hooks + skills + manifest;
  #    不重复放 rules/AGENTS.md,避免与本机 ~/.gemini/AGENTS.md 双重注入)
  mkdir -p "$plugin_dir/assets" "$plugin_dir/skills"
  rm -rf "$plugin_dir/rules"
  cp "$here/plugin.json" "$plugin_dir/plugin.json"
  cp "$here/hooks.json" "$plugin_dir/hooks.json"
  cp "$here/assets/logo.svg" "$plugin_dir/assets/logo.svg"
  cp "$here/README.md" "$plugin_dir/README.md"
  for skill in agentdash agentdash-setup agentdash-orchestrate; do
    mkdir -p "$plugin_dir/skills/$skill"
    cp "$here/skills/$skill/SKILL.md" "$plugin_dir/skills/$skill/SKILL.md"
  done

  echo "[agentdash] antigravity-kit installed globally (machine-level):"
  echo "  - Global AGENTS.md : ${gemini_home}/AGENTS.md"
  echo "  - Global Plugin    : ${plugin_dir}"
  echo "[agentdash] 提醒:首次创建全局插件目录后请重启 Antigravity 客户端使全局钩子对所有项目生效"
  exit 0
fi

# ======================= 模式 B:显式指定项目目录安装 =======================
target="$1"
if [ ! -d "$target" ]; then
  echo "[agentdash] 目标目录不存在: $target" >&2
  exit 1
fi

merge_instructions "$target/AGENTS.md" "$here/../shared/AGENTDASH.md"

agents_dir="$target/.agents"
config_file="$agents_dir/hooks.json"
tpl_file="$here/hooks.template.json"
if command -v cygpath >/dev/null 2>&1; then
  tpl_jq="$(cygpath -m "$tpl_file")"
else
  tpl_jq="$tpl_file"
fi
mkdir -p "$agents_dir"

merge_with_jq() {
  local tmp="$config_file.tmp-agentdash" cfg_jq="$config_file"
  if command -v cygpath >/dev/null 2>&1; then
    cfg_jq="$(cygpath -m "$config_file")"
  fi
  if jq --slurpfile tpl "$tpl_jq" \
    'if type == "object" then .agentdash = $tpl[0].agentdash else error("non-object") end' \
    "$cfg_jq" > "$tmp" 2>/dev/null; then
    mv "$tmp" "$config_file"
    return 0
  fi
  rm -f "$tmp"
  return 1
}

valid_json() {
  local cfg_jq="$config_file"
  if command -v cygpath >/dev/null 2>&1; then
    cfg_jq="$(cygpath -m "$config_file")"
  fi
  jq -e . "$cfg_jq" >/dev/null 2>&1
}

if [ ! -f "$config_file" ]; then
  cp "$tpl_file" "$config_file"
  echo "[agentdash] hooks registered (fresh): $config_file"
elif command -v jq >/dev/null 2>&1 && valid_json && merge_with_jq; then
  echo "[agentdash] hooks registered (merged): $config_file"
elif command -v jq >/dev/null 2>&1; then
  cp "$config_file" "$config_file.bak-agentdash"
  cp "$tpl_file" "$config_file"
  echo "[agentdash] $config_file 损坏,已备份为 $config_file.bak-agentdash 后重建(仅含 agentdash)" >&2
else
  echo "[agentdash] 未找到 jq,不改动既有 $config_file;请把以下 agentdash 顶层命名空间并入:" >&2
  cat "$tpl_file" >&2
fi

for skill in agentdash agentdash-setup agentdash-orchestrate; do
  mkdir -p "$agents_dir/skills/$skill"
  cp "$here/skills/$skill/SKILL.md" "$agents_dir/skills/$skill/SKILL.md"
done
echo "[agentdash] skills synced: $agents_dir/skills/{agentdash,agentdash-setup,agentdash-orchestrate}"

gitignore="$target/.gitignore"
if [ ! -f "$gitignore" ] || ! grep -q '\.agentdash' "$gitignore"; then
  if [ -f "$gitignore" ] && [ -s "$gitignore" ] && [ -n "$(tail -c1 "$gitignore")" ]; then
    printf '\n' >> "$gitignore"
  fi
  printf '.agentdash/\n' >> "$gitignore"
fi

echo "[agentdash] antigravity-kit installed into $target"
echo "[agentdash] 提醒:首次写入 .agents/hooks.json 与 .agents/skills/ 后请新建会话或重载窗口生效"
