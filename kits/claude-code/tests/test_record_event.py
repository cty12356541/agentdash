"""claude-code kit · fixture 回放测试。

以子进程回放三钩子(PostToolUse/Stop/SubagentStop)样例载荷,断言
`<cwd>/.agentdash/events.jsonl` 的行序与字段(spec §4.2),含 gate running→passed 折叠。
另覆盖纯函数(退出码/摘要提取/gate 命令匹配)与降级路径(损坏输入不抛)。
"""
from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

KIT_DIR = Path(__file__).resolve().parents[1]      # kits/claude-code
HOOKS_DIR = KIT_DIR / "hooks"
sys.path.insert(0, str(HOOKS_DIR))

import record_event  # noqa: E402


# ---------------------------------------------------------------- helpers

def run_hook(payload: dict, cwd: Path) -> None:
    """子进程全链路回放:stdin JSON(UTF-8)→ 入口重配 → 落盘。"""
    proc = subprocess.run(
        [sys.executable, str(HOOKS_DIR / "record_event.py")],
        input=json.dumps(payload, ensure_ascii=False),
        capture_output=True, text=True, encoding="utf-8", errors="replace",
        cwd=str(cwd))
    assert proc.returncode == 0, f"hook 退出码 {proc.returncode}: {proc.stderr}"


def feed(text: str, cwd: Path) -> None:
    """喂原始 stdin 文本(损坏输入用)。"""
    proc = subprocess.run(
        [sys.executable, str(HOOKS_DIR / "record_event.py")],
        input=text, capture_output=True, text=True, encoding="utf-8",
        errors="replace", cwd=str(cwd))
    assert proc.returncode == 0, f"hook 退出码 {proc.returncode}: {proc.stderr}"


def events_path(cwd: Path) -> Path:
    return cwd / ".agentdash" / "events.jsonl"


def read_events(cwd: Path) -> list:
    return [json.loads(ln) for ln in events_path(cwd).read_text(encoding="utf-8").splitlines()]


def in_cwd(payload: dict, cwd: Path) -> dict:
    out = dict(payload)
    out["cwd"] = str(cwd)
    return out


# ---------------------------------------------------------------- fixtures

CARGO_TEST_POST = {                       # PostToolUse · bash cargo test(命中 gate)
    "session_id": "s-w1-008",
    "hook_event_name": "PostToolUse",
    "tool_name": "Bash",
    "tool_input": {"command": "cargo test --all", "description": "run tests"},
    "tool_response": {
        "stdout": ("running 298 tests\n"
                   "test result: ok. 298 passed; 0 failed; finished in 1.23s\n"),
        "stderr": "", "interrupted": False, "status": 0,
    },
}

EDIT_POST = {                             # PostToolUse · 其他工具
    "session_id": "s-w1-008",
    "hook_event_name": "PostToolUse",
    "tool_name": "Edit",
    "tool_input": {"file_path": "src/main.rs", "old_string": "a", "new_string": "b"},
    "tool_response": {"structuredPatch": "@@ -1 +1 @@"},
}

SUBAGENT_STOP = {                         # SubagentStop
    "session_id": "s-sub-1",
    "transcript_path": "C:/t/sub.jsonl",
    "hook_event_name": "SubagentStop",
}

STOP = {                                  # Stop(折叠在途 gate)
    "session_id": "s-w1-008",
    "hook_event_name": "Stop",
    "stop_hook_active": True,
}

TS_RE = r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}[+-]\d{2}:\d{2}$"


# ---------------------------------------------------------------- tests

class TestFixtureReplay(unittest.TestCase):

    def test_three_hooks_replay_with_gate_fold(self):
        """三钩子样例载荷(cargo test PostToolUse + 对应 Stop)→ 行序与字段。"""
        with tempfile.TemporaryDirectory() as tmp:
            t = Path(tmp)
            run_hook(in_cwd(CARGO_TEST_POST, t), t)   # gate running
            run_hook(in_cwd(EDIT_POST, t), t)         # tool end
            run_hook(in_cwd(SUBAGENT_STOP, t), t)     # agent completed
            run_hook(in_cwd(STOP, t), t)              # gate → passed

            evs = read_events(t)
            self.assertEqual([e["kind"] for e in evs],
                             ["gate", "tool", "agent", "gate"])
            # 行1:gate running
            self.assertEqual(evs[0]["gate"], "cargo-test")
            self.assertEqual(evs[0]["state"], "running")
            # 行2:tool 事件 phase=end + exit + summary
            self.assertEqual(evs[1]["tool"], "edit")
            self.assertEqual(evs[1]["phase"], "end")
            self.assertEqual(evs[1]["exit"], 0)
            self.assertEqual(evs[1]["summary"], "src/main.rs")
            # 行3:agent completed
            self.assertEqual(evs[2]["event"], "completed")
            # 行4:折叠 passed + exit + 摘要行
            self.assertEqual(evs[3]["gate"], "cargo-test")
            self.assertEqual(evs[3]["state"], "passed")
            self.assertEqual(evs[3]["exit"], 0)
            self.assertIn("298 passed", evs[3]["detail"])
            self.assertNotIn("running", [e["state"] for e in evs if e["kind"] == "gate"][1:])
            # 折叠只做一次:pending_gate.json 消费后删除
            self.assertFalse((t / ".agentdash" / "pending_gate.json").exists())
            # ts:ISO8601 本地时区(带偏移)
            for e in evs:
                self.assertRegex(e["ts"], TS_RE)

    def test_failed_gate_fold(self):
        """非零退出码 → failed;摘要空 stdout 时取 stderr 末行。"""
        payload = {
            "session_id": "s", "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "go test ./..."},
            "tool_response": {"stdout": "", "stderr": "FAIL\t./pkg [build failed]\nexit status 1\n",
                              "interrupted": False, "status": 1},
        }
        with tempfile.TemporaryDirectory() as tmp:
            t = Path(tmp)
            run_hook(in_cwd(payload, t), t)
            run_hook(in_cwd(STOP, t), t)
            evs = read_events(t)
            self.assertEqual([e["state"] for e in evs], ["running", "failed"])
            self.assertEqual(evs[1]["exit"], 1)
            self.assertEqual(evs[1]["detail"], "exit status 1")

    def test_utf8_payload_roundtrip(self):
        """中文载荷全程 UTF-8(入口 stdio 重配,承 claude-dash 教训)。"""
        payload = {
            "session_id": "s", "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "echo 仪表盘", "description": "中文描述 ✓"},
            "tool_response": {"stdout": "ok\n", "status": 0},
        }
        with tempfile.TemporaryDirectory() as tmp:
            t = Path(tmp)
            run_hook(in_cwd(payload, t), t)
            raw = events_path(t).read_text(encoding="utf-8")
            self.assertIn("echo 仪表盘", raw)  # ensure_ascii=False,中文原文落盘
            self.assertEqual(read_events(t)[0]["summary"], "echo 仪表盘")


class TestGateMatching(unittest.TestCase):

    def test_gate_commands(self):
        cases = [
            ("cargo test", "cargo-test"),
            ("cargo test --all --offline", "cargo-test"),
            ("cd /d/agentdash && cargo clippy -- -D warnings", "cargo-clippy"),
            ("cargo fmt --check", "cargo-fmt"),
            ("go test ./...", "go-test"),
            ("npm test -- --watchAll=false", "npm-test"),
            ("gh pr checks 12", "gh-pr-checks"),
            ("cargo build --release", None),   # 非验证门
            ("python -m unittest discover -v", None),
            ("", None),
        ]
        for command, expected in cases:
            with self.subTest(command=command):
                self.assertEqual(record_event._gate_name(command), expected)

    def test_non_gate_bash_is_tool_event(self):
        payload = {
            "session_id": "s", "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "ls -la"},
            "tool_response": {"stdout": "total 0\n", "status": 0},
        }
        with tempfile.TemporaryDirectory() as tmp:
            t = Path(tmp)
            run_hook(in_cwd(payload, t), t)
            evs = read_events(t)
            self.assertEqual(len(evs), 1)
            self.assertEqual(evs[0]["kind"], "tool")
            self.assertEqual(evs[0]["tool"], "bash")
            self.assertEqual(evs[0]["exit"], 0)
            self.assertEqual(evs[0]["summary"], "ls -la")

    def test_summary_truncated_to_80(self):
        payload = {
            "session_id": "s", "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "x" * 200},
            "tool_response": {"stdout": "", "status": 0},
        }
        with tempfile.TemporaryDirectory() as tmp:
            t = Path(tmp)
            run_hook(in_cwd(payload, t), t)
            self.assertEqual(len(read_events(t)[0]["summary"]), 80)


class TestExtraction(unittest.TestCase):

    def test_exit_code_variants(self):
        self.assertEqual(record_event._exit_code({"status": 0}), 0)
        self.assertEqual(record_event._exit_code({"exit_code": 3}), 3)
        self.assertEqual(record_event._exit_code({"interrupted": True, "status": 0}), 130)
        self.assertEqual(record_event._exit_code({"is_error": True}), 1)
        self.assertEqual(record_event._exit_code({"ok": True}), 0)
        self.assertEqual(record_event._exit_code("plain string response"), 0)
        self.assertEqual(record_event._exit_code(None), 0)

    def test_summary_line(self):
        self.assertEqual(
            record_event._summary_line({"stdout": "a\n\nb  \n", "stderr": ""}), "b")
        self.assertEqual(
            record_event._summary_line({"stdout": "", "stderr": "boom\nboom\n"}), "boom")
        self.assertEqual(record_event._summary_line({"stdout": ""}), "")
        self.assertEqual(record_event._summary_line("not-a-dict"), "")


class TestDegrade(unittest.TestCase):
    """铁律:损坏输入降级跳过不抛,进程恒退 0,绝不阻塞会话。"""

    def test_invalid_json_stdin(self):
        with tempfile.TemporaryDirectory() as tmp:
            t = Path(tmp)
            feed("{{{{not json\n", t)
            self.assertFalse(events_path(t).exists())

    def test_empty_stdin(self):
        with tempfile.TemporaryDirectory() as tmp:
            t = Path(tmp)
            feed("", t)
            self.assertFalse(events_path(t).exists())

    def test_non_dict_payload(self):
        with tempfile.TemporaryDirectory() as tmp:
            t = Path(tmp)
            feed('["list", "not", "dict"]', t)
            self.assertFalse(events_path(t).exists())

    def test_stop_without_pending_writes_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            t = Path(tmp)
            run_hook(in_cwd(STOP, t), t)
            self.assertFalse(events_path(t).exists())

    def test_stop_with_corrupt_pending_degrades(self):
        with tempfile.TemporaryDirectory() as tmp:
            t = Path(tmp)
            pending = t / ".agentdash" / "pending_gate.json"
            pending.parent.mkdir(parents=True, exist_ok=True)
            pending.write_text("{{corrupt", encoding="utf-8")
            run_hook(in_cwd(STOP, t), t)
            self.assertFalse(pending.exists())          # 损坏暂存被消费删除
            self.assertFalse(events_path(t).exists())   # 不产生幽灵事件

    def test_payload_without_tool_name_skipped(self):
        with tempfile.TemporaryDirectory() as tmp:
            t = Path(tmp)
            run_hook({"hook_event_name": "PostToolUse", "cwd": str(t)}, t)
            self.assertFalse(events_path(t).exists())


class TestHooksManifest(unittest.TestCase):

    def test_hooks_json_registers_three_events(self):
        manifest = json.loads((HOOKS_DIR / "hooks.json").read_text(encoding="utf-8"))
        hooks = manifest["hooks"]
        self.assertEqual(set(hooks), {"PostToolUse", "Stop", "SubagentStop"})
        for event, blocks in hooks.items():
            self.assertTrue(blocks, f"{event} 无注册块")
            commands = [h["command"] for b in blocks for h in b["hooks"]]
            self.assertTrue(all("record_event.py" in c for c in commands),
                            f"{event} 存在非本 kit 命令")
            self.assertTrue(all(c.endswith("|| true") for c in commands),
                            f"{event} 缺 || true 静默保险")
        # PostToolUse 不设 matcher = 全工具(其他工具也要落 tool 事件)
        for block in hooks["PostToolUse"]:
            self.assertNotIn("matcher", block)


if __name__ == "__main__":
    unittest.main()
