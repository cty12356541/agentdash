#!/usr/bin/env bash
# agentdash · claude-code 集成包安装器(bash 版;Windows PowerShell 用 install.ps1)
# 前提:agentdash 二进制在 PATH——hook 直调二进制子命令,零 Python 前置(spec §6 修订)。
# 用法:./install.sh [目标项目目录](默认当前目录)
# 动作:
#   1. 检测 agentdash 在 PATH(缺失 → 打印安装指引并退出)
#   2. skill 复制进 <目标>/.claude/skills/agentdash/
#   3. <目标>/.claude/settings.json 幂等注册五钩子(命令为常量,无路径 baked)
#   4. <目标>/.gitignore 幂等追加 .agentdash/(M-2)
#   5. 清理老版本 Python 垫片残留(record_event.py 及空目录)
# 幂等:重复执行只刷新自家注册与文件,不动 settings.json 其他内容。
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
target="${1:-$PWD}"
if [ ! -d "$target" ]; then
  echo "[agentdash] 目标目录不存在: $target" >&2
  exit 1
fi

# --- 前提检测:agentdash 在 PATH ---
if ! command -v agentdash >/dev/null 2>&1; then
  echo "[agentdash] 未找到 agentdash —— hook 直调二进制,需要它先在 PATH。安装方式二选一:" >&2
  echo "  1) 仓库内安装:  cargo install --path <agentdash 仓库根目录>" >&2
  echo "  2) 发布件:      从项目 Releases 下载对应平台二进制,放入 PATH" >&2
  exit 1
fi

mkdir -p "$target/.claude/skills/agentdash"
cp "$here/skills/agentdash/SKILL.md" "$target/.claude/skills/agentdash/SKILL.md"

# --- AGENTDASH.md 标记段幂等合入 CLAUDE.md(harness 单源,W7-005/C3)。块经
# 文件传入 awk(赋值前缀而非 -v:BSD awk 对 -v 值做转义处理,多行块会炸)---
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
merge_instructions "$target/CLAUDE.md"

# --- 清理被二进制方案替代的 Python 垫片残留(老版本安装产物)---
rm -f "$target/.claude/agentdash/hooks/record_event.py"
rmdir "$target/.claude/agentdash/hooks" "$target/.claude/agentdash" 2>/dev/null || true

# --- M-2:目标仓 .gitignore 幂等追加 .agentdash/ ---
gitignore="$target/.gitignore"
if [ ! -f "$gitignore" ] || ! grep -qE '^[[:space:]]*\.agentdash/?[[:space:]]*$' "$gitignore"; then
  # 末行无换行时先补一个:否则 .agentdash/ 粘上最后一行失效,且幂等检查永远失配
  # ($(… ) 命令替换会吞掉尾部换行:尾字节是换行时结果为空,非换行时为该字符)
  if [ -f "$gitignore" ] && [ -s "$gitignore" ] && [ -n "$(tail -c1 "$gitignore")" ]; then
    printf '\n' >> "$gitignore"
  fi
  printf '.agentdash/\n' >> "$gitignore"
fi
echo "[agentdash] .gitignore ensured: .agentdash/ ($gitignore)"

# --- settings.json 幂等注册五钩子 ---
settings="$target/.claude/settings.json"

write_fresh_settings() {
  cat > "$settings" <<'EOF'
{
  "hooks": {
    "PostToolUse": [
      {"hooks": [{"type": "command", "command": "agentdash hook --host claude posttooluse || true"}]}
    ],
    "PostToolUseFailure": [
      {"hooks": [{"type": "command", "command": "agentdash hook --host claude posttoolusefailure || true"}]}
    ],
    "PreToolUse": [
      {"matcher": "Task|Agent", "hooks": [{"type": "command", "command": "agentdash hook --host claude pretooluse || true"}]}
    ],
    "Stop": [
      {"hooks": [{"type": "command", "command": "agentdash hook --host claude stop || true"}]}
    ],
    "SubagentStop": [
      {"hooks": [{"type": "command", "command": "agentdash hook --host claude subagentstop || true"}]}
    ]
  }
}
EOF
}

merge_with_jq() {
  # 语义自家注册判定:新形态 `agentdash hook …`;旧形态含 record_event.py(一并替换)
  local tmp="$settings.tmp-agentdash"
  if jq '
    def ours: (.command // "") | (test("record_event[.]py") or test("agentdash hook"));
    def clean:
      map((.hooks //= []) | .hooks |= map(select((ours) | not)))
      | map(select((.hooks | length) > 0));
    .hooks //= {}
    | .hooks.PostToolUse  = ((.hooks.PostToolUse  // []) | clean)
        + [{hooks: [{type: "command", command: "agentdash hook --host claude posttooluse || true"}]}]
    | .hooks.PostToolUseFailure = ((.hooks.PostToolUseFailure // []) | clean)
        + [{hooks: [{type: "command", command: "agentdash hook --host claude posttoolusefailure || true"}]}]
    | .hooks.PreToolUse   = ((.hooks.PreToolUse   // []) | clean)
        + [{matcher: "Task|Agent",
            hooks: [{type: "command", command: "agentdash hook --host claude pretooluse || true"}]}]
    | .hooks.Stop         = ((.hooks.Stop         // []) | clean)
        + [{hooks: [{type: "command", command: "agentdash hook --host claude stop || true"}]}]
    | .hooks.SubagentStop = ((.hooks.SubagentStop // []) | clean)
        + [{hooks: [{type: "command", command: "agentdash hook --host claude subagentstop || true"}]}]
  ' "$settings" > "$tmp" 2>/dev/null; then
    mv "$tmp" "$settings"
    return 0
  fi
  rm -f "$tmp"
  return 1
}

if [ ! -f "$settings" ]; then
  write_fresh_settings
  echo "[agentdash] hooks registered (fresh): $settings"
elif command -v jq >/dev/null 2>&1 && jq -e . "$settings" >/dev/null 2>&1 && merge_with_jq; then
  echo "[agentdash] hooks registered (merged): $settings"
elif command -v jq >/dev/null 2>&1; then
  # JSON 损坏:备份后重建,不吞用户文件
  cp "$settings" "$settings.bak-agentdash"
  write_fresh_settings
  echo "[agentdash] settings.json 损坏,已备份为 $settings.bak-agentdash 后重建注册"
else
  # 降级:无 jq 不动用户文件,打印手工合并指引(其余安装产物已完成)
  echo "[agentdash] 未找到 jq,不改动既有 $settings;请把以下四段并入其 hooks(或装 jq 后重跑):" >&2
  cat >&2 <<'EOF'
    "PostToolUse":  [{"hooks": [{"type": "command", "command": "agentdash hook --host claude posttooluse || true"}]}],
    "PreToolUse":   [{"matcher": "Task|Agent", "hooks": [{"type": "command", "command": "agentdash hook --host claude pretooluse || true"}]}],
    "Stop":         [{"hooks": [{"type": "command", "command": "agentdash hook --host claude stop || true"}]}],
    "SubagentStop": [{"hooks": [{"type": "command", "command": "agentdash hook --host claude subagentstop || true"}]}]
EOF
fi

echo "[agentdash] installed into $target/.claude (skill: /agentdash)"
