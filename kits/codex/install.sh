#!/usr/bin/env bash
# agentdash · codex-kit 安装器(W7-003)
# 前提:agentdash 二进制在 PATH。用法:./install.sh [目标项目目录]
# 动作:
#   1. AGENTDASH.md 标记段幂等合入 <目标>/AGENTS.md(codex 原生读取)
#   2. <目标>/.codex/hooks.json 幂等注册四钩子(命令带 --host codex)
#   3. <目标>/.gitignore 幂等追加 .agentdash/
# 注意:codex repo 级 hooks 需在 Codex 内对该项目 trust,并经 /hooks 一次性
#       审查(非受管命令钩子的安全边界,由宿主强制)。
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

# --- AGENTDASH.md 标记段幂等合入 AGENTS.md(harness 单源,C3)---
merge_instructions() {
  # $1 = 指令文件(CLAUDE.md / AGENTS.md)
  local file="$1" block
  block="$(cat "$here/../shared/AGENTDASH.md")"
  if [ ! -f "$file" ]; then
    printf '%s\n' "$block" > "$file"
    echo "[agentdash] instructions created: $file"
  elif grep -q 'agentdash:begin' "$file"; then
    # 已有标记段:整段替换(v1 内容刷新)
    local tmp="$file.tmp-agentdash"
    awk -v blk="$block" 'BEGIN{skip=0} /agentdash:begin/{skip=1; printf "%s\n", blk; next} /agentdash:end/{skip=0; next} skip==0{print}' "$file" > "$tmp"
    mv "$tmp" "$file"
    echo "[agentdash] instructions refreshed: $file"
  else
    { cat "$file"; printf '\n%s\n' "$block"; } >> "$file"
    echo "[agentdash] instructions appended: $file"
  fi
}
merge_instructions "$target/AGENTS.md"

# --- .codex/hooks.json 注册(D3 保守策略)---
codex_dir="$target/.codex"
hooks_file="$codex_dir/hooks.json"
mkdir -p "$codex_dir"
write_fresh() {
  cp "$here/hooks.json" "$hooks_file"
}
if [ ! -f "$hooks_file" ]; then
  write_fresh
  echo "[agentdash] hooks registered (fresh): $hooks_file"
elif grep -q 'agentdash hook' "$hooks_file"; then
  # 自家旧注册:整体刷新为本版(块结构由本 kit 独有,无外部内容可保)
  write_fresh
  echo "[agentdash] hooks refreshed: $hooks_file"
else
  # 他人注册在场:不吞用户文件,备份 + 手工合并指引
  cp "$hooks_file" "$hooks_file.bak-agentdash" 2>/dev/null || true
  echo "[agentdash] $hooks_file 已有其他钩子,未改动;已备份为 $hooks_file.bak-agentdash" >&2
  echo "  请把以下四块并入其 hooks 数组(与 claude-code kit 同构):" >&2
  cat "$here/hooks.json" >&2
fi

# --- .gitignore 幂等追加 .agentdash/ ---
gitignore="$target/.gitignore"
if [ ! -f "$gitignore" ] || ! grep -qE '^[[:space:]]*\.agentdash/?[[:space:]]*$' "$gitignore"; then
  if [ -f "$gitignore" ] && [ -s "$gitignore" ] && [ -n "$(tail -c1 "$gitignore")" ]; then
    printf '\n' >> "$gitignore"
  fi
  printf '.agentdash/\n' >> "$gitignore"
fi

echo "[agentdash] codex-kit installed into $target"
echo "[agentdash] 提醒:repo 级 .codex/hooks.json 需在 Codex 内 trust 本项目,并经 /hooks 一次性审查"
