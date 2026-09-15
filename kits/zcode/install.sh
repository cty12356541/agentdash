#!/usr/bin/env bash
# agentdash · zcode-kit 安装器(W8;公司内部宿主)
# 前提:agentdash 二进制在 PATH。用法:./install.sh [目标项目目录]
# 动作:
#   1. AGENTDASH.md 标记段幂等合入 <目标>/AGENTS.md(zcode 原生读取)
#   2. <目标>/.zcode/config.json 幂等注册三钩子(PostToolUse/PreToolUse/Stop,
#      --host zcode;只动 hooks 键,其余配置键原样;enabled:true 置位)
#   3. <目标>/.gitignore 幂等追加 .agentdash/
# 注意:工作台钩子于会话启动加载——已运行会话需重开生效。zcode 无子代理
#       事件,在跑 agent 面板对本宿主不可见(如实标注)。
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

# --- .zcode/config.json 注册三钩子(D1:只动 hooks 键)---
zcode_dir="$target/.zcode"
config_file="$zcode_dir/config.json"
mkdir -p "$zcode_dir"
merge_with_jq() {
  # 只接管 hooks 键:自家事件块过滤后重加,enabled 置位;其余键原样
  local tmp="$config_file.tmp-agentdash"
  if jq '
    .hooks //= {}
    | .hooks.enabled = true
    | .hooks.events //= {}
    | def ours: (.hooks // []) | map(.command // "") | any(test("agentdash hook"));
    .hooks.events.PostToolUse = (((.hooks.events.PostToolUse // []) | map(.hooks |= map(select((.command // "") | test("agentdash hook") | not)))) | map(select(.hooks | length > 0)))
      + [{"hooks": [{"type": "command", "command": "agentdash hook --host zcode posttooluse || true"}]}]
    | .hooks.events.PostToolUseFailure = (((.hooks.events.PostToolUseFailure // []) | map(.hooks |= map(select((.command // "") | test("agentdash hook") | not)))) | map(select(.hooks | length > 0)))
      + [{"hooks": [{"type": "command", "command": "agentdash hook --host zcode posttoolusefailure || true"}]}]
    | .hooks.events.PreToolUse = (((.hooks.events.PreToolUse // []) | map(.hooks |= map(select((.command // "") | test("agentdash hook") | not)))) | map(select(.hooks | length > 0)))
      + [{"matcher": "Task|Agent",
          "hooks": [{"type": "command", "command": "agentdash hook --host zcode pretooluse || true"}]}]
    | .hooks.events.Stop = (((.hooks.events.Stop // []) | map(.hooks |= map(select((.command // "") | test("agentdash hook") | not)))) | map(select(.hooks | length > 0)))
      + [{"hooks": [{"type": "command", "command": "agentdash hook --host zcode stop || true"}]}]
  ' "$config_file" > "$tmp" 2>/dev/null; then
    mv "$tmp" "$config_file"
    return 0
  fi
  rm -f "$tmp"
  return 1
}
if [ ! -f "$config_file" ]; then
  cp "$here/hooks.template.json" "$config_file"
  echo "[agentdash] hooks registered (fresh): $config_file"
elif command -v jq >/dev/null 2>&1 && jq -e . "$config_file" >/dev/null 2>&1 && merge_with_jq; then
  echo "[agentdash] hooks registered (merged): $config_file"
elif command -v jq >/dev/null 2>&1; then
  # JSON 损坏:备份后重建(仅含 hooks;其余键已随损坏不可考,备份在案)
  cp "$config_file" "$config_file.bak-agentdash"
  cp "$here/hooks.template.json" "$config_file"
  echo "[agentdash] $config_file 损坏,已备份为 $config_file.bak-agentdash 后重建(仅含 hooks)" >&2
else
  # 无 jq:不动用户配置,打印手工合并指引
  echo "[agentdash] 未找到 jq,不改动既有 $config_file;请把以下 hooks 块并入(注意 enabled:true):" >&2
  cat "$here/hooks.template.json" >&2
fi

# --- .gitignore 幂等追加 .agentdash/(已有任何 .agentdash 条目则跳过——
# 防止裸忽略行落在白名单例外之后,语义反转,如本仓的台账白名单)---
gitignore="$target/.gitignore"
if [ ! -f "$gitignore" ] || ! grep -q '\.agentdash' "$gitignore"; then
  if [ -f "$gitignore" ] && [ -s "$gitignore" ] && [ -n "$(tail -c1 "$gitignore")" ]; then
    printf '\n' >> "$gitignore"
  fi
  printf '.agentdash/\n' >> "$gitignore"
fi

echo "[agentdash] zcode-kit installed into $target"
echo "[agentdash] 提醒:工作台钩子于会话启动加载,已运行会话请重开生效;zcode 无子代理事件,在跑 agent 面板对本宿主不可见"
