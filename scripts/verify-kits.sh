#!/usr/bin/env bash
# agentdash · kits 宿主安装体验证一键巡检
# 对 claude-code / codex / opencode / zcode / cursor / deepseek 六个 kit,在
# 全新临时目标目录各跑一轮安装体验证:全新安装产物 → 已装钩子命令端到端
# 事件落盘 → 二次安装幂等 → 既有内容保留(用户键/他人钩子/损坏 JSON 备份
# 重建/粘行防护)。
#
# 用法:scripts/verify-kits.sh [--keep]  (--keep 保留临时目录供人工检查)
# 依赖:agentdash 在 PATH(硬前提,缺即整体 FAIL——hook 直调二进制,这正是
#       dogfood 实测踩过的静默失效点,故列为第一断言);jq 可选(缺时 JSON/
#       事件流断言降级 SKIP,与安装器无-jq 降级语义一致)。
# 退出:零 FAIL(允许 SKIP)→ 0;任一 FAIL → 1。
# 注意:巡检的是 PATH 上**已装**的 agentdash——改动 src/ 后先
# `cargo install --path .` 重装再跑,否则新载荷路径会被旧二进制静默跳过。
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
keep=0
[ "${1:-}" = "--keep" ] && keep=1

tmp="$(mktemp -d "${TMPDIR:-/tmp}/agentdash-verify-kits.XXXXXX")"
# Windows(Git Bash/MSYS)归一:mktemp 产出 POSIX 形制(/tmp/...),而 PATH 上
# 的 agentdash 是 Windows 二进制——载荷 cwd 里的 /tmp/... 会被它按当前盘
# 解析成 \tmp\...,探针读取与二进制落盘分家(六宿主事件断言齐 FAIL 的根因)。
# 统一为 mixed 形制(C:/...,bash 与 Windows 二进制两侧通用);无 cygpath 的
# 平台原样(Linux/macOS 不受影响)。
if command -v cygpath >/dev/null 2>&1; then
  tmp="$(cygpath -m "$tmp")"
fi
if [ "$keep" = 1 ]; then
  echo "[verify-kits] --keep:临时目录保留 $tmp"
else
  trap 'rm -rf "$tmp"' EXIT
fi

pass=0; fail=0; skip=0
ok()  { pass=$((pass+1)); printf '  \033[32mPASS\033[0m %s\n' "$1"; }
bad() { fail=$((fail+1)); printf '  \033[31mFAIL\033[0m %s\n' "$1"; }
na()  { skip=$((skip+1)); printf '  \033[33mSKIP\033[0m %s\n' "$1"; }

section() { printf '\n== %s ==\n' "$1"; }

# 断言原语:got == want(字符串比较,数字亦按字符串),失败带实际值
assert() { # name got want
  if [ "$2" = "$3" ]; then ok "$1"; else bad "$1(实际: $2)"; fi
}
assert_f() { # name path
  if [ -e "$2" ]; then ok "$1"; else bad "$1"; fi
}
# JSON 断言(jq -e:表达式真退 0);无 jq 降级 SKIP
assert_jq() { # name file expr
  if [ "$HAVE_JQ" != yes ]; then na "$1(无 jq)"; return; fi
  if jq -e "$3" "$2" >/dev/null 2>&1; then ok "$1"; else bad "$1"; fi
}
# 安装器调用:成功静默,失败把输出缩进打到 stderr 供定位。
# 安装器语义是"装进既有项目目录"(目标目录须已存在,缺失即退 1),
# 探针目录由本巡检负责创建。
run_installer() { # kit target
  local out
  mkdir -p "$2"
  if out=$(bash "$root/kits/$1/install.sh" "$2" 2>&1); then
    return 0
  fi
  printf '%s\n' "$out" | sed 's/^/    /' >&2
  return 1
}
# 指令文件标记段恰一次(begin/end 各 1)
check_instructions() { # name file
  assert "$1 标记段 begin 恰一次" "$(grep -c 'agentdash:begin' "$2" || true)" "1"
  assert "$1 标记段 end 恰一次" "$(grep -c 'agentdash:end' "$2" || true)" "1"
}
check_gitignore() { # target
  assert ".gitignore 含 .agentdash/ 恰一次" \
    "$(grep -c '^\.agentdash/$' "$1/.gitignore" || true)" "1"
}
check_exec_bit() { # kit
  assert "kits/$1/install.sh 可执行位" "$([ -x "$root/kits/$1/install.sh" ] && echo y || echo n)" "y"
}
# 事件落盘探针:把给定载荷喂给"已装钩子命令"(cmd 为空则直调二进制,与
# opencode 插件 spawn 同参),断言 events.jsonl 出 host 归属行。
# 载荷形制两家:claude/zcode 系用 tool_name/tool_input;cursor 系由调用方
# 传 afterShellExecution 形制(与 kits/cursor 安装注册一一对应)。
probe_event() { # host target payload_json [installed_cmd]
  local host="$1" dir="$2" json="$3" cmd="${4:-}"
  local payload="$dir/.agentdash/events.jsonl"
  if [ -n "$cmd" ]; then
    if [ "$HAVE_JQ" != yes ]; then na "[$host] 已装命令端到端事件落盘(无 jq)"; return; fi
    printf '%s' "$json" | sh -c "$cmd"
  else
    printf '%s' "$json" | agentdash hook --host "$host" posttooluse
  fi
  if [ "$HAVE_JQ" = yes ] && jq -e --arg h "$host" '.kind == "tool" and .host == $h' "$payload" >/dev/null 2>&1; then
    ok "[$host] PostToolUse 事件落盘且归属 host=$host"
  else
    bad "[$host] PostToolUse 事件落盘且归属 host=$host"
  fi
}
# claude/zcode 系标准探针载荷(bash 工具,非 gate 命令)
tooluse_payload() { # target
  printf '{"tool_name":"bash","tool_input":{"command":"echo verify-kits-probe"},"cwd":"%s"}' "$1"
}

echo "[verify-kits] 仓库根: $root"
HAVE_JQ=no
command -v jq >/dev/null 2>&1 && HAVE_JQ=yes

section "环境预检"
if command -v agentdash >/dev/null 2>&1; then
  ok "agentdash 在 PATH($(agentdash --version 2>/dev/null || echo '版本未知'))"
else
  bad "agentdash 不在 PATH —— hook 直调二进制,四 kit 全体不可用"
  printf '  修复: cargo install --path %s  然后重跑本巡检\n' "$root"
  exit 1
fi
if [ "$HAVE_JQ" = yes ]; then
  ok "jq 在 PATH($(jq --version))"
else
  na "jq 不在 PATH —— JSON/事件流断言降级 SKIP(安装器自身可无 jq 运行)"
fi
assert_f "共享指令块 kits/shared/AGENTDASH.md" "$root/kits/shared/AGENTDASH.md"

# ---------- claude-code:五钩子 settings.json + CLAUDE.md + skill ----------
section "claude-code kit"
check_exec_bit claude-code
t="$tmp/cc-main"
if run_installer claude-code "$t"; then ok "全新目录安装器退出 0"; else bad "全新目录安装器退出非 0"; fi
assert_f "CLAUDE.md 生成" "$t/CLAUDE.md"
check_instructions "CLAUDE.md" "$t/CLAUDE.md"
assert_f "skill 就位" "$t/.claude/skills/agentdash/SKILL.md"
check_gitignore "$t"
s="$t/.claude/settings.json"
assert_f "settings.json 生成" "$s"
assert_jq "PostToolUse 注册(--host claude)" "$s" \
  '.hooks.PostToolUse | length == 1 and (.[0].hooks[0].command | test("agentdash hook --host claude posttooluse"))'
assert_jq "PostToolUseFailure 注册(--host claude,W11-002)" "$s" \
  '.hooks.PostToolUseFailure | length == 1 and (.[0].hooks[0].command | test("agentdash hook --host claude posttoolusefailure"))'
assert_jq "PreToolUse matcher=Task|Agent" "$s" \
  '.hooks.PreToolUse | length == 1 and .[0].matcher == "Task|Agent"'
assert_jq "Stop 注册(--host claude)" "$s" \
  '.hooks.Stop | length == 1 and (.[0].hooks[0].command | test("agentdash hook --host claude stop"))'
assert_jq "SubagentStop 注册(--host claude)" "$s" \
  '.hooks.SubagentStop | length == 1 and (.[0].hooks[0].command | test("agentdash hook --host claude subagentstop"))'
if [ "$HAVE_JQ" = yes ]; then
  probe_event claude "$t" "$(tooluse_payload "$t")" "$(jq -r '.hooks.PostToolUse[0].hooks[0].command' "$s")"
else
  na "[claude] 已装命令端到端事件落盘(无 jq)"
fi
if run_installer claude-code "$t"; then ok "二次安装退出 0"; else bad "二次安装退出非 0"; fi
check_instructions "CLAUDE.md(二次)" "$t/CLAUDE.md"
assert "settings.json 自家注册不翻倍(5 处)" \
  "$(grep -o 'agentdash hook --host claude' "$s" | wc -l | tr -d ' ')" "5"
# 既有内容保留:用户键 + 用户钩子并入不吞
t="$tmp/cc-preserve"; mkdir -p "$t/.claude"
printf '%s\n' '{"model":"sonnet","hooks":{"PostToolUse":[{"hooks":[{"type":"command","command":"echo user-keep"}]}]}}' \
  > "$t/.claude/settings.json"
if run_installer claude-code "$t"; then ok "preserve 场景安装器退出 0"; else bad "preserve 场景安装器退出非 0"; fi
s="$t/.claude/settings.json"
assert_jq "用户 model 键保留" "$s" '.model == "sonnet"'
assert_jq "用户钩子保留" "$s" '[.hooks.PostToolUse[].hooks[]?.command] | any(. == "echo user-keep")'
assert_jq "自家注册并入(PostToolUse 共 2 条)" "$s" '.hooks.PostToolUse | length == 2'
# 损坏 JSON:备份后重建
t="$tmp/cc-corrupt"; mkdir -p "$t/.claude"; printf '{broken' > "$t/.claude/settings.json"
if run_installer claude-code "$t"; then ok "corrupt 场景安装器退出 0"; else bad "corrupt 场景安装器退出非 0"; fi
assert_f "损坏 JSON 备份 .bak-agentdash" "$t/.claude/settings.json.bak-agentdash"
assert_jq "损坏 JSON 重建为合法注册" "$t/.claude/settings.json" '.hooks.PostToolUse | length == 1'

# ---------- codex:四钩子 hooks.json(整体刷新)+ AGENTS.md ----------
section "codex kit"
check_exec_bit codex
t="$tmp/cx-main"
if run_installer codex "$t"; then ok "全新目录安装器退出 0"; else bad "全新目录安装器退出非 0"; fi
check_instructions "AGENTS.md" "$t/AGENTS.md"
check_gitignore "$t"
h="$t/.codex/hooks.json"
assert_f ".codex/hooks.json 生成" "$h"
for ev in PostToolUse SubagentStart SubagentStop Stop; do
  assert_jq "$ev 注册(--host codex)" "$h" \
    ".hooks.$ev | length == 1 and (.[0].hooks[0].command | test(\"agentdash hook --host codex\"))"
done
if [ "$HAVE_JQ" = yes ]; then
  probe_event codex "$t" "$(tooluse_payload "$t")" "$(jq -r '.hooks.PostToolUse[0].hooks[0].command' "$h")"
else
  na "[codex] 已装命令端到端事件落盘(无 jq)"
fi
if run_installer codex "$t"; then ok "二次安装退出 0"; else bad "二次安装退出非 0"; fi
check_instructions "AGENTS.md(二次)" "$t/AGENTS.md"
assert "hooks.json 与 kit 模板一致(整体刷新语义)" \
  "$(cmp -s "$h" "$root/kits/codex/hooks.json" && echo same || echo diff)" "same"
# 他人注册在场:原样不动 + 备份
t="$tmp/cx-foreign"; mkdir -p "$t/.codex"
printf '%s\n' '{"hooks":{"PostToolUse":[{"hooks":[{"type":"command","command":"echo user-codex"}]}]}}' \
  > "$t/.codex/hooks.json"
if run_installer codex "$t"; then ok "foreign 场景安装器退出 0"; else bad "foreign 场景安装器退出非 0"; fi
assert "他人 hooks.json 原样未动" \
  "$(cmp -s "$t/.codex/hooks.json" "$t/.codex/hooks.json.bak-agentdash" && echo same || echo diff)" "same"
assert_f "他人 hooks.json 备份 .bak-agentdash" "$t/.codex/hooks.json.bak-agentdash"

# ---------- opencode:插件直调 + AGENTS.md(粘行防护)----------
section "opencode kit"
check_exec_bit opencode
t="$tmp/oc-main"; mkdir -p "$t"; printf 'legacy notes' > "$t/AGENTS.md"
if run_installer opencode "$t"; then ok "预置 AGENTS.md 目录安装器退出 0"; else bad "预置 AGENTS.md 目录安装器退出非 0"; fi
assert "预置 AGENTS.md 原文保留(粘行防护)" "$(head -n 1 "$t/AGENTS.md")" "legacy notes"
check_instructions "AGENTS.md" "$t/AGENTS.md"
check_gitignore "$t"
assert_f "插件就位" "$t/.opencode/plugins/agentdash.js"
assert "插件与 kit 源一致" \
  "$(cmp -s "$t/.opencode/plugins/agentdash.js" "$root/kits/opencode/plugins/agentdash.js" && echo same || echo diff)" "same"
probe_event opencode "$t" "$(tooluse_payload "$t")"
if run_installer opencode "$t"; then ok "二次安装退出 0"; else bad "二次安装退出非 0"; fi
check_instructions "AGENTS.md(二次)" "$t/AGENTS.md"
assert "插件二次安装仍与 kit 源一致" \
  "$(cmp -s "$t/.opencode/plugins/agentdash.js" "$root/kits/opencode/plugins/agentdash.js" && echo same || echo diff)" "same"

# ---------- zcode:四钩子 config.json(enabled 置位)+ AGENTS.md ----------
section "zcode kit"
check_exec_bit zcode
t="$tmp/zc-main"
if run_installer zcode "$t"; then ok "全新目录安装器退出 0"; else bad "全新目录安装器退出非 0"; fi
check_instructions "AGENTS.md" "$t/AGENTS.md"
check_gitignore "$t"
c="$t/.zcode/config.json"
assert_f "config.json 生成" "$c"
assert_jq "hooks.enabled 置位" "$c" '.hooks.enabled == true'
assert_jq "PostToolUse 注册(--host zcode)" "$c" \
  '.hooks.events.PostToolUse | length == 1 and (.[0].hooks[0].command | test("agentdash hook --host zcode posttooluse"))'
assert_jq "PostToolUseFailure 注册(--host zcode)" "$c" \
  '.hooks.events.PostToolUseFailure | length == 1 and (.[0].hooks[0].command | test("agentdash hook --host zcode posttoolusefailure"))'
assert_jq "PreToolUse matcher=Task|Agent" "$c" \
  '.hooks.events.PreToolUse | length == 1 and .[0].matcher == "Task|Agent"'
assert_jq "Stop 注册(--host zcode)" "$c" \
  '.hooks.events.Stop | length == 1 and (.[0].hooks[0].command | test("agentdash hook --host zcode stop"))'
if [ "$HAVE_JQ" = yes ]; then
  probe_event zcode "$t" "$(tooluse_payload "$t")" "$(jq -r '.hooks.events.PostToolUse[0].hooks[0].command' "$c")"
else
  na "[zcode] 已装命令端到端事件落盘(无 jq)"
fi
if run_installer zcode "$t"; then ok "二次安装退出 0"; else bad "二次安装退出非 0"; fi
check_instructions "AGENTS.md(二次)" "$t/AGENTS.md"
assert "config.json 自家注册不翻倍(4 处)" \
  "$(grep -o 'agentdash hook --host zcode' "$c" | wc -l | tr -d ' ')" "4"
assert_jq "二次安装 enabled 仍置位" "$c" '.hooks.enabled == true'
# 既有内容保留:用户键 + 用户钩子 + enabled 翻正
t="$tmp/zc-preserve"; mkdir -p "$t/.zcode"
printf '%s\n' '{"model":"k2","hooks":{"enabled":false,"events":{"PostToolUse":[{"hooks":[{"type":"command","command":"echo user-keep"}]}]}}}' \
  > "$t/.zcode/config.json"
if run_installer zcode "$t"; then ok "preserve 场景安装器退出 0"; else bad "preserve 场景安装器退出非 0"; fi
c="$t/.zcode/config.json"
assert_jq "用户 model 键保留" "$c" '.model == "k2"'
assert_jq "enabled 翻正 true" "$c" '.hooks.enabled == true'
assert_jq "用户钩子保留" "$c" '[.hooks.events.PostToolUse[].hooks[]?.command] | any(. == "echo user-keep")'
assert_jq "自家注册并入(PostToolUse 共 2 条)" "$c" '.hooks.events.PostToolUse | length == 2'
# 损坏 JSON:备份后重建(仅含 hooks)
t="$tmp/zc-corrupt"; mkdir -p "$t/.zcode"; printf '{broken' > "$t/.zcode/config.json"
if run_installer zcode "$t"; then ok "corrupt 场景安装器退出 0"; else bad "corrupt 场景安装器退出非 0"; fi
assert_f "损坏 JSON 备份 .bak-agentdash" "$t/.zcode/config.json.bak-agentdash"
assert_jq "损坏 JSON 重建且 enabled 置位" "$t/.zcode/config.json" '.hooks.enabled == true'

# ---------- cursor:五钩子 hooks.json(version 1)+ AGENTS.md ----------
# 载荷形制:afterShellExecution 顶层 command+output,无 tool_name —— 探针
# 用 Cursor 词表,验证二进制归一路径(hook.rs cursor_shell_payload)。
section "cursor kit"
check_exec_bit cursor
t="$tmp/cu-main"
if run_installer cursor "$t"; then ok "全新目录安装器退出 0"; else bad "全新目录安装器退出非 0"; fi
check_instructions "AGENTS.md" "$t/AGENTS.md"
check_gitignore "$t"
h="$t/.cursor/hooks.json"
assert_f "hooks.json 生成" "$h"
assert_jq "version 置 1" "$h" '.version == 1'
for ev in afterShellExecution postToolUseFailure subagentStart subagentStop stop; do
  assert_jq "$ev 注册(--host cursor)" "$h" \
    ".hooks.$ev | length == 1 and (.[0].command | test(\"agentdash hook --host cursor\"))"
done
if [ "$HAVE_JQ" = yes ]; then
  probe_event cursor "$t" \
    "{\"hook_event_name\":\"afterShellExecution\",\"command\":\"echo verify-kits-probe\",\"output\":\"probe-out\",\"cwd\":\"$t\"}" \
    "$(jq -r '.hooks.afterShellExecution[0].command' "$h")"
else
  na "[cursor] 已装命令端到端事件落盘(无 jq)"
fi
if run_installer cursor "$t"; then ok "二次安装退出 0"; else bad "二次安装退出非 0"; fi
check_instructions "AGENTS.md(二次)" "$t/AGENTS.md"
assert "hooks.json 自家注册不翻倍(5 处)" \
  "$(grep -o 'agentdash hook --host cursor' "$h" | wc -l | tr -d ' ')" "5"
assert_jq "二次安装 version 仍为 1" "$h" '.version == 1'
# 既有内容保留:用户键 + 用户事件块不吞,version 不回改
t="$tmp/cu-preserve"; mkdir -p "$t/.cursor"
printf '%s\n' '{"version":1,"other":"keep","hooks":{"beforeShellExecution":[{"command":"echo user-keep"}]}}' \
  > "$t/.cursor/hooks.json"
if run_installer cursor "$t"; then ok "preserve 场景安装器退出 0"; else bad "preserve 场景安装器退出非 0"; fi
h="$t/.cursor/hooks.json"
assert_jq "用户 other 键保留" "$h" '.other == "keep"'
assert_jq "用户事件块保留(beforeShellExecution 不吞)" "$h" \
  '[.hooks.beforeShellExecution[].command] | any(. == "echo user-keep")'
assert_jq "自家注册并入(afterShellExecution 恰 1)" "$h" '.hooks.afterShellExecution | length == 1'
# 损坏 JSON:备份后重建(仅含 hooks)
t="$tmp/cu-corrupt"; mkdir -p "$t/.cursor"; printf '{broken' > "$t/.cursor/hooks.json"
if run_installer cursor "$t"; then ok "corrupt 场景安装器退出 0"; else bad "corrupt 场景安装器退出非 0"; fi
assert_f "损坏 JSON 备份 .bak-agentdash" "$t/.cursor/hooks.json.bak-agentdash"
assert_jq "损坏 JSON 重建且 version 置 1" "$t/.cursor/hooks.json" '.version == 1'

# ---------- deepseek:DSH 桥接形制(Claude 形制五钩子)+ AGENTS.md ----------
# 桥消费 Claude Code 形制 hooks.json,命令带 --host deepseek;探针走
# claude 词表载荷,验证桥将消费的同一文件形态。
section "deepseek kit"
check_exec_bit deepseek
t="$tmp/ds-main"
if run_installer deepseek "$t"; then ok "全新目录安装器退出 0"; else bad "全新目录安装器退出非 0"; fi
check_instructions "AGENTS.md" "$t/AGENTS.md"
check_gitignore "$t"
h="$t/.deepseek/agentdash-hooks.json"
assert_f "agentdash-hooks.json 生成" "$h"
for ev in PostToolUse PreToolUse SubagentStart SubagentStop Stop; do
  assert_jq "$ev 注册(--host deepseek)" "$h" \
    ".hooks.$ev | length == 1 and (.[0].hooks[0].command | test(\"agentdash hook --host deepseek\"))"
done
assert_jq "PreToolUse matcher=Task|Agent" "$h" \
  '.hooks.PreToolUse[0].matcher == "Task|Agent"'
if [ "$HAVE_JQ" = yes ]; then
  probe_event deepseek "$t" "$(tooluse_payload "$t")" "$(jq -r '.hooks.PostToolUse[0].hooks[0].command' "$h")"
else
  na "[deepseek] 已装命令端到端事件落盘(无 jq)"
fi
if run_installer deepseek "$t"; then ok "二次安装退出 0"; else bad "二次安装退出非 0"; fi
check_instructions "AGENTS.md(二次)" "$t/AGENTS.md"
assert "hooks.json 自家注册不翻倍(5 处)" \
  "$(grep -o 'agentdash hook --host deepseek' "$h" | wc -l | tr -d ' ')" "5"
# 他人内容在场:备份 + 不吞
t="$tmp/ds-foreign"; mkdir -p "$t/.deepseek"
printf '%s\n' '{"hooks":{"PostToolUse":[{"hooks":[{"type":"command","command":"echo user-dsh"}]}]}}' \
  > "$t/.deepseek/agentdash-hooks.json"
if run_installer deepseek "$t"; then ok "foreign 场景安装器退出 0"; else bad "foreign 场景安装器退出非 0"; fi
assert "他人 hooks 文件原样未动" \
  "$(cmp -s "$t/.deepseek/agentdash-hooks.json" "$t/.deepseek/agentdash-hooks.json.bak-agentdash" && echo same || echo diff)" "same"
assert_f "他人内容备份 .bak-agentdash" "$t/.deepseek/agentdash-hooks.json.bak-agentdash"
# 损坏 JSON:grep 形制安装器无从判定归属,按他人内容保守处理(备份 + 不动)
t="$tmp/ds-corrupt"; mkdir -p "$t/.deepseek"; printf '{broken' > "$t/.deepseek/agentdash-hooks.json"
if run_installer deepseek "$t"; then ok "corrupt 场景安装器退出 0"; else bad "corrupt 场景安装器退出非 0"; fi
assert "损坏文件原样未动(保守策略)" \
  "$(cmp -s "$t/.deepseek/agentdash-hooks.json" "$t/.deepseek/agentdash-hooks.json.bak-agentdash" && echo same || echo diff)" "same"

section "汇总"
echo "PASS $pass / FAIL $fail / SKIP $skip(临时目录: $tmp)"
if [ "$fail" -gt 0 ]; then
  echo "[verify-kits] 存在 FAIL,六宿主安装体验证未过"
  exit 1
fi
echo "[verify-kits] 六宿主 kit 安装体验证全部通过"
