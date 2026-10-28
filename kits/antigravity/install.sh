#!/usr/bin/env bash
# agentdash · antigravity-kit 安装器(W14;Google Antigravity 宿主)
# 前提:agentdash 二进制在 PATH。用法:./install.sh [目标项目目录]
# 动作:
#   1. AGENTDASH.md 标记段幂等合入 <目标>/AGENTS.md(Antigravity 原生层级规则读取)
#   2. <目标>/.agents/hooks.json 幂等注册顶层 "agentdash" 命名空间三钩子
#      (PreToolUse[invoke_subagent] / PostToolUse[*] / 扁平 Stop,--host agy;
#       保留同文件其他插件/用户顶层命名空间)
#   3. 三套技能(agentdash / agentdash-setup / agentdash-orchestrate)复制进 <目标>/.agents/skills/
#   4. <目标>/.gitignore 幂等追加 .agentdash/
# 注意:Antigravity 在 Windows 下经 cmd /c 执行钩子且把 stdout 作 JSON 解析——
#       命令不带 `|| true`,由 `agentdash hook --host agy` 原生保证恒退 0 与合法 JSON 回执。
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
target="${1:-$PWD}"
if [ ! -d "$target" ]; then
  echo "[agentdash] 目标目录不存在: $target" >&2
  exit 1
fi

if ! command -v agentdash >/dev/null 2>&1; then
  echo "[agentdash] 未找到 agentdash —— hook 直调二进制,需要它先在 PATH。" >&2
  echo "  1) cargo install --path <agentdash 仓库根>   2) Releases 下载放入 PATH" >&2
  exit 1
fi

# --- AGENTDASH.md 标记段幂等合入 AGENTS.md(harness 单源,C3)。块经文件传入
# awk(赋值前缀而非 -v:BSD awk 对 -v 值做转义处理,多行块会炸——W7 实测)---
merge_instructions() {
  local file="$1" block_file="$here/../shared/AGENTDASH.md" tmp
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
merge_instructions "$target/AGENTS.md"

# --- .agents/hooks.json 注册顶层 "agentdash" 命名空间(保留其余顶层键/插件)---
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
  # JSON 损坏或非对象根:备份后重建
  cp "$config_file" "$config_file.bak-agentdash"
  cp "$tpl_file" "$config_file"
  echo "[agentdash] $config_file 损坏,已备份为 $config_file.bak-agentdash 后重建(仅含 agentdash)" >&2
else
  # 无 jq:不动用户配置,打印手工合并指引到 stderr
  echo "[agentdash] 未找到 jq,不改动既有 $config_file;请把以下 agentdash 顶层命名空间并入:" >&2
  cat "$tpl_file" >&2
fi

# --- 复制三套技能进 .agents/skills/ ---
for skill in agentdash agentdash-setup agentdash-orchestrate; do
  mkdir -p "$agents_dir/skills/$skill"
  cp "$here/skills/$skill/SKILL.md" "$agents_dir/skills/$skill/SKILL.md"
done
echo "[agentdash] skills synced: $agents_dir/skills/{agentdash,agentdash-setup,agentdash-orchestrate}"

# --- .gitignore 幂等追加 .agentdash/(已有任何 .agentdash 条目则跳过)---
gitignore="$target/.gitignore"
if [ ! -f "$gitignore" ] || ! grep -q '\.agentdash' "$gitignore"; then
  if [ -f "$gitignore" ] && [ -s "$gitignore" ] && [ -n "$(tail -c1 "$gitignore")" ]; then
    printf '\n' >> "$gitignore"
  fi
  printf '.agentdash/\n' >> "$gitignore"
fi

echo "[agentdash] antigravity-kit installed into $target"
echo "[agentdash] 提醒:首次写入 .agents/hooks.json 与 .agents/skills/ 后请新建会话或重载窗口生效"
