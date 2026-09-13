#!/usr/bin/env bash
# agentdash · claude-code 集成包安装器(bash 版;Windows PowerShell 用 install.ps1)
# 用法:./install.sh [目标项目目录](默认当前目录)
# 动作:hooks/skill 复制进 <目标>/.claude/,并在 settings.json 幂等注册三钩子。
# 幂等:重复执行只刷新文件与自家注册,不动 settings.json 其他内容。
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
target="${1:-$PWD}"
if [ ! -d "$target" ]; then
  echo "[agentdash] 目标目录不存在: $target" >&2
  exit 1
fi

mkdir -p "$target/.claude/agentdash/hooks" "$target/.claude/skills/agentdash"
cp "$here/hooks/record_event.py" "$target/.claude/agentdash/hooks/record_event.py"
cp "$here/skills/agentdash/SKILL.md" "$target/.claude/skills/agentdash/SKILL.md"

# 探测 hook 运行时解释器(python3 → python → py -3)
py=""
for c in python3 python "py -3"; do
  if $c -c "import sys" >/dev/null 2>&1; then py="$c"; break; fi
done
if [ -z "$py" ]; then
  echo "[agentdash] 未找到 python3/python —— hook 需要它;文件已复制,请装好 Python 后重跑" >&2
  exit 1
fi

script="$target/.claude/agentdash/hooks/record_event.py"
case "$script" in
  /*) abs="$script" ;;
  *)  abs="$(cd "$(dirname "$script")" && pwd)/$(basename "$script")" ;;
esac
# MSYS/Git Bash:POSIX 绝对路径(/tmp、/d/…)对原生 Windows python 无效,
# baked 进 settings.json 的命令须是真 Windows 路径(cygpath -m 混合式,双平台可读)。
if command -v cygpath >/dev/null 2>&1; then
  abs="$(cygpath -m "$abs")"
  target_w="$(cygpath -m "$target")"
else
  target_w="$target"
fi
cmd="$py \"$abs\" || true"

"$py" - "$target_w/.claude/settings.json" "$cmd" <<'PYEOF'
import json
import shutil
import sys
from pathlib import Path

settings_path, hook_cmd = sys.argv[1], sys.argv[2]
path = Path(settings_path)
raw = None
if path.exists():
    try:
        raw = path.read_text(encoding="utf-8")
    except OSError:
        raw = None
try:
    data = json.loads(raw) if raw else {}
except ValueError:
    # 损坏的 settings.json:备份后重建,不吞用户文件
    shutil.copy2(path, str(path) + ".bak-agentdash")
    data = {}
if not isinstance(data, dict):
    data = {}
hooks = data.setdefault("hooks", {})
if not isinstance(hooks, dict):
    hooks = {}
    data["hooks"] = hooks

MARKER_CMD, MARKER_FILE = "agentdash", "record_event.py"
entry = {"type": "command", "command": hook_cmd}


def is_ours(h) -> bool:
    if not isinstance(h, dict):
        return False
    c = str(h.get("command", ""))
    return MARKER_CMD in c and MARKER_FILE in c


def clean(existing):
    """剔除自家旧注册(幂等),保留其他内容;返回 list。"""
    if not isinstance(existing, list):
        return []
    out = []
    for block in existing:
        if not isinstance(block, dict):
            continue
        hs = block.get("hooks")
        if isinstance(hs, list):
            kept = [h for h in hs if not is_ours(h)]
            if not kept:
                continue
            block = dict(block, hooks=kept)
        out.append(block)
    return out


for event in ("PostToolUse", "Stop", "SubagentStop"):
    hooks[event] = clean(hooks.get(event)) + [{"hooks": [dict(entry)]}]
    # PostToolUse 不设 matcher = 全工具(其他工具也要落 tool 事件)

path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
print("[agentdash] hooks registered in %s: PostToolUse/Stop/SubagentStop" % path)
PYEOF

echo "[agentdash] installed into $target/.claude (skill: /agentdash)"
