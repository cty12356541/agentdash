#!/usr/bin/env python3
"""agentdash 事件记录器(claude-code kit)。

读 hook stdin JSON 载荷,按契约 spec §4.2 追加一行事件到 `<cwd>/.agentdash/events.jsonl`:

- PostToolUse · bash 且命令命中验证门(cargo test/clippy/fmt、go test、npm test、gh pr checks)
  → `gate` 事件 state=running;退出码与一行摘要暂存 `.agentdash/pending_gate.json`
  (hook 是一次性进程,Stop 折叠须经落盘交接)
- PostToolUse · 其他工具 → `tool` 事件 phase=end + exit + summary
- Stop → 把最近 running 的 gate 折叠为 passed/failed(退出码 + detail 摘要行)
- SubagentStop → `agent` 事件 event=completed

铁律:自身任何失败静默退出 0,绝不阻塞会话(hooks.json 已带 || true 双保险);
一切损坏输入降级跳过不抛。
"""
from __future__ import annotations

import json
import re
import sys
from datetime import datetime
from pathlib import Path

# 中文 Windows 默认 GBK stdio(承 claude-dash 教训):入口统一重配 UTF-8,
# 否则含中文/特殊字符的载荷在 stdin 解码处即 UnicodeDecodeError。
for _stream in (sys.stdin, sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8", errors="replace")
    except (AttributeError, OSError, ValueError):
        pass

MAX_SUMMARY = 80
PENDING_GATE = "pending_gate.json"

GATE_PATTERNS = [
    (re.compile(r"\bcargo\s+test\b"), "cargo-test"),
    (re.compile(r"\bcargo\s+clippy\b"), "cargo-clippy"),
    (re.compile(r"\bcargo\s+fmt\b"), "cargo-fmt"),
    (re.compile(r"\bgo\s+test\b"), "go-test"),
    (re.compile(r"\bnpm\s+test\b"), "npm-test"),
    (re.compile(r"\bgh\s+pr\s+checks\b"), "gh-pr-checks"),
]


def _now() -> str:
    """ISO8601 本地时区(带偏移),秒级。"""
    return datetime.now().astimezone().isoformat(timespec="seconds")


def _events_dir(payload: dict) -> Path:
    cwd = payload.get("cwd")
    if isinstance(cwd, str) and cwd.strip():
        return Path(cwd) / ".agentdash"
    return Path.cwd() / ".agentdash"


def _append(events_dir: Path, event: dict) -> None:
    events_dir.mkdir(parents=True, exist_ok=True)
    with (events_dir / "events.jsonl").open("a", encoding="utf-8") as f:
        f.write(json.dumps(event, ensure_ascii=False) + "\n")


def _gate_name(command: str) -> str | None:
    for pattern, name in GATE_PATTERNS:
        if pattern.search(command):
            return name
    return None


def _exit_code(tool_response) -> int:
    """退出码:Bash 的 status/exit_code;interrupted → 130;其他工具 is_error→1/0;不可知→0。"""
    if isinstance(tool_response, dict):
        if tool_response.get("interrupted"):
            return 130
        for key in ("status", "exit_code", "exit"):
            v = tool_response.get(key)
            if isinstance(v, int):
                return v
        return 1 if tool_response.get("is_error") else 0
    return 0


def _summary_line(tool_response) -> str:
    """一行摘要:stdout(空则 stderr)最后一条非空行,截 MAX_SUMMARY。"""
    if not isinstance(tool_response, dict):
        return ""
    for key in ("stdout", "stderr"):
        text = tool_response.get(key)
        if isinstance(text, str):
            lines = [ln.strip() for ln in text.splitlines() if ln.strip()]
            if lines:
                return lines[-1][:MAX_SUMMARY]
    return ""


def _write_pending(events_dir: Path, pending: dict) -> None:
    try:
        events_dir.mkdir(parents=True, exist_ok=True)
        (events_dir / PENDING_GATE).write_text(
            json.dumps(pending, ensure_ascii=False), encoding="utf-8")
    except OSError:
        pass  # 暂存失败只损失折叠,不损 events.jsonl


def _on_post_tool_use(payload: dict, events_dir: Path) -> None:
    tool = str(payload.get("tool_name") or "")
    if not tool:
        return
    tool_input = payload.get("tool_input")
    if not isinstance(tool_input, dict):
        tool_input = {}
    response = payload.get("tool_response")

    if tool.lower() == "bash":
        command = str(tool_input.get("command") or "")
        gate = _gate_name(command)
        if gate is not None:
            _append(events_dir, {"ts": _now(), "kind": "gate",
                                 "gate": gate, "state": "running"})
            _write_pending(events_dir, {"gate": gate,
                                        "exit": _exit_code(response),
                                        "detail": _summary_line(response)})
            return
        summary = command
    else:
        summary = ""
        for key in ("description", "file_path", "command", "pattern"):
            v = tool_input.get(key)
            if v:
                summary = str(v)
                break

    _append(events_dir, {"ts": _now(), "kind": "tool", "tool": tool.lower(),
                         "phase": "end", "exit": _exit_code(response),
                         "summary": summary[:MAX_SUMMARY]})


def _on_stop(payload: dict, events_dir: Path) -> None:
    pending_path = events_dir / PENDING_GATE
    pending = None
    try:
        pending = json.loads(pending_path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        pending = None  # 无在途 gate 或暂存损坏:降级跳过
    try:
        pending_path.unlink()  # 折叠只做一次:无论成败都消费掉
    except OSError:
        pass
    if isinstance(pending, dict) and pending.get("gate"):
        exit_code = pending.get("exit")
        _append(events_dir, {"ts": _now(), "kind": "gate",
                             "gate": str(pending["gate"]),
                             "state": "passed" if exit_code == 0 else "failed",
                             "exit": exit_code,
                             "detail": str(pending.get("detail") or "")[:MAX_SUMMARY]})


def _on_subagent_stop(payload: dict, events_dir: Path) -> None:
    event = {"ts": _now(), "kind": "agent", "event": "completed"}
    who = payload.get("agent_name") or payload.get("who")
    if who:
        event["who"] = str(who)[:MAX_SUMMARY]
    _append(events_dir, event)


def main(stdin_text: str) -> None:
    try:
        payload = json.loads(stdin_text or "{}")
        if not isinstance(payload, dict):
            return
        name = payload.get("hook_event_name") or ""
        if not name and payload.get("tool_name"):
            name = "PostToolUse"  # 防御:老版本载荷缺 hook_event_name
        events_dir = _events_dir(payload)
        if name == "PostToolUse":
            _on_post_tool_use(payload, events_dir)
        elif name == "Stop":
            _on_stop(payload, events_dir)
        elif name == "SubagentStop":
            _on_subagent_stop(payload, events_dir)
    except Exception:
        pass  # 铁律:静默,绝不抛出阻塞会话


if __name__ == "__main__":
    try:
        _text = sys.stdin.read()
    except Exception:
        _text = ""
    main(_text)
    sys.exit(0)
