#!/usr/bin/env bash
# agentdash · deepseek-kit 安装器(W9-006;DeepSeek Harness 宿主,桥接形制)
# 前提:agentdash 二进制在 PATH;DSH 侧经 dsh-hooks-claude-code 桥挂载本 kit
# 产出的 hooks.json(官方 API:packages/hooks,configPath 指向)。
# 动作:
#   1. AGENTDASH.md 标记段幂等合入 <目标>/AGENTS.md(DSH 为 AGENTS.md 原生)
#   2. <目标>/.deepseek/agentdash-hooks.json 幂等写入五钩子(PostToolUse/
#      PreToolUse/SubagentStart/SubagentStop/Stop,--host deepseek;Claude
#      Code 形制,桥直接消费)。本文件由本 kit 独占:既有 agentdash 注册 →
#      整体刷新;他人内容在场 → 备份 + 手工合并指引,不吞用户文件
#   3. <目标>/.gitignore 幂等追加 .agentdash/
# 注意:桥的 configPath 是进程级、启动时一次读取(官方 TODO:项目级自动
#       发现未实现),挂载需在 DSH 组合(cordis.yml)里登记 dsh-hooks-
#       claude-code 包并把 configPath 指到本文件——安装器只打印指引不动
#       用户的组合文件。钩子在会话工作区内运行,CLAUDE_PROJECT_DIR 由桥
#       自动导出,本 kit 命令不依赖任何模板变量。
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

# --- .deepseek/agentdash-hooks.json 写入(D3 保守策略,同 codex kit)---
dsh_dir="$target/.deepseek"
hooks_file="$dsh_dir/agentdash-hooks.json"
mkdir -p "$dsh_dir"
write_fresh() {
  cp "$here/hooks.json" "$hooks_file"
}
if [ ! -f "$hooks_file" ]; then
  write_fresh
  echo "[agentdash] hooks written (fresh): $hooks_file"
elif grep -q 'agentdash hook' "$hooks_file"; then
  # 本 kit 独占文件:整体刷新为本版
  write_fresh
  echo "[agentdash] hooks refreshed: $hooks_file"
else
  # 他人内容在场:不吞用户文件,备份 + 手工合并指引
  cp "$hooks_file" "$hooks_file.bak-agentdash" 2>/dev/null || true
  echo "[agentdash] $hooks_file 已有其他内容,未改动;已备份为 $hooks_file.bak-agentdash" >&2
  echo "  请把以下五块并入其 hooks 对象(Claude Code 形制,命令换 --host deepseek):" >&2
  cat "$here/hooks.json" >&2
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

echo "[agentdash] deepseek-kit installed into $target"
echo "[agentdash] 挂载指引:DSH 组合里登记桥包,configPath 指向本文件(进程级,启动时一次读取):"
cat <<'EOF'
    - name: '@deepseek-ai/dsh-hooks-claude-code'
      config:
        configPath: ./.deepseek/agentdash-hooks.json
EOF
echo "[agentdash] 提醒:相对 configPath 自 DSH 进程启动目录解析;钩子在会话工作区运行"
