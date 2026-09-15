//! claude-code kit hook 的 Rust 集成测试(W1-008b)。
//!
//! 移植原 `kits/claude-code/tests/test_record_event.py` 的 fixture 断言(该 Python
//! 垫片随二进制方案退役):全链路子进程回放 `agentdash hook <event>` + stdin 载荷,
//! 断言 `.agentdash/events.jsonl` 行序与字段(spec §4.2),含 gate running→passed 折叠、
//! 退出码/摘要提取、降级路径(损坏输入恒退 0 不落盘)与 hooks.json 清单;
//! 另有并发用例:8 线程 × N 行同文件灌入,断言零丢失(文件锁语义)。

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{Value, json};

/// 被测二进制(cargo 注入的绝对路径,bin-only crate 走子进程回放)。
const EXE: &str = env!("CARGO_BIN_EXE_agentdash");
/// 仓库根(用于断言 kits 下的 hooks.json 清单)。
const MANIFEST: &str = env!("CARGO_MANIFEST_DIR");

// ---------------------------------------------------------------- helpers

/// 自清理临时目录(测试工作仓:载荷 `cwd` 指向这里,events.jsonl 落这里)。
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("agentdash-hook-{tag}-{pid}-{n}"));
        fs::create_dir_all(&dir).expect("create tempdir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.0));
    }
}

/// 子进程回放:`args` 为 hook 子命令参数,`raw` 为 stdin 原文。
fn feed(args: &[&str], raw: &str, cwd: &Path) -> Output {
    let mut child = Command::new(EXE)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn agentdash");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(raw.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait agentdash")
}

/// 按事件名回放载荷(`{"hook", event}`)。
fn feed_payload(event: &str, payload: &Value, cwd: &Path) -> Output {
    feed(&["hook", event], &payload.to_string(), cwd)
}

/// 载荷复制并把 `cwd` 指向测试目录。
fn in_cwd(payload: &Value, cwd: &Path) -> Value {
    let mut out = payload.clone();
    out["cwd"] = json!(cwd.to_string_lossy());
    out
}

/// 成功回放且 stderr 为空(hook 铁律:绝不向宿主报错)。
fn assert_silent_success(out: &Output, context: &str) {
    assert!(
        out.status.success(),
        "{context}: hook 退出码非 0,stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "{context}: hook 向宿主报错: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn events_path(cwd: &Path) -> PathBuf {
    cwd.join(".agentdash").join("events.jsonl")
}

fn pending_path(cwd: &Path) -> PathBuf {
    cwd.join(".agentdash").join("pending_gate.json")
}

fn read_events(cwd: &Path) -> Vec<Value> {
    fs::read_to_string(events_path(cwd))
        .expect("events.jsonl readable")
        .lines()
        .map(|line| serde_json::from_str(line).expect("event line json"))
        .collect()
}

/// ISO8601 本地时区秒级:`YYYY-MM-DDTHH:MM:SS±HH:MM`(25 字节)。
fn is_iso8601_local(ts: &str) -> bool {
    let b = ts.as_bytes();
    let digits = |slice: &[u8]| slice.iter().all(u8::is_ascii_digit);
    b.len() == 25
        && digits(&b[0..4])
        && b[4] == b'-'
        && digits(&b[5..7])
        && b[7] == b'-'
        && digits(&b[8..10])
        && b[10] == b'T'
        && digits(&b[11..13])
        && b[13] == b':'
        && digits(&b[14..16])
        && b[16] == b':'
        && digits(&b[17..19])
        && matches!(b[19], b'+' | b'-')
        && digits(&b[20..22])
        && b[22] == b':'
        && digits(&b[23..25])
}

// ---------------------------------------------------------------- fixtures

/// `PostToolUse` · bash `cargo test`(命中 gate)。
fn cargo_test_post() -> Value {
    json!({
        "session_id": "s-w1-008",
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "cargo test --all", "description": "run tests"},
        "tool_response": {
            "stdout": "running 298 tests\ntest result: ok. 298 passed; 0 failed; finished in 1.23s\n",
            "stderr": "",
            "interrupted": false,
            "status": 0
        }
    })
}

/// Stop(折叠在途 gate)。
fn stop_payload() -> Value {
    json!({
        "session_id": "s-w1-008",
        "hook_event_name": "Stop",
        "stop_hook_active": true
    })
}

// ---------------------------------------------------------------- tests

#[test]
fn three_hooks_replay_with_gate_fold() {
    let t = TempDir::new("replay");
    let edit_post = json!({
        "session_id": "s-w1-008",
        "hook_event_name": "PostToolUse",
        "tool_name": "Edit",
        "tool_input": {"file_path": "src/main.rs", "old_string": "a", "new_string": "b"},
        "tool_response": {"structuredPatch": "@@ -1 +1 @@"}
    });
    let subagent_stop = json!({
        "session_id": "s-sub-1",
        "transcript_path": "C:/t/sub.jsonl",
        "hook_event_name": "SubagentStop"
    });

    assert_silent_success(
        &feed_payload(
            "posttooluse",
            &in_cwd(&cargo_test_post(), t.path()),
            t.path(),
        ),
        "cargo test posttooluse",
    );
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&edit_post, t.path()), t.path()),
        "edit posttooluse",
    );
    assert_silent_success(
        &feed_payload("subagentstop", &in_cwd(&subagent_stop, t.path()), t.path()),
        "subagentstop",
    );
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_payload(), t.path()), t.path()),
        "stop",
    );

    let evs = read_events(t.path());
    assert_eq!(evs.len(), 4, "恰四行: gate/tool/agent/gate");
    assert_eq!(
        [
            evs[0]["kind"].as_str().unwrap(),
            evs[1]["kind"].as_str().unwrap(),
            evs[2]["kind"].as_str().unwrap(),
            evs[3]["kind"].as_str().unwrap()
        ],
        ["gate", "tool", "agent", "gate"]
    );
    // 行1:gate running
    assert_eq!(evs[0]["gate"], "cargo-test");
    assert_eq!(evs[0]["state"], "running");
    // 行2:tool 事件 phase=end + exit + summary
    assert_eq!(evs[1]["tool"], "edit");
    assert_eq!(evs[1]["phase"], "end");
    // W4-001 改判:Edit 的 response({"structuredPatch":…})无退出码证据,exit 记
    // null 不臆造 0(此字段为溯源事实,内核重放不读)
    assert!(evs[1]["exit"].is_null(), "无证据 exit 应为 null");
    assert_eq!(evs[1]["summary"], "src/main.rs");
    // 行3:agent completed
    assert_eq!(evs[2]["event"], "completed");
    // 行4:折叠 passed + exit + 摘要行
    assert_eq!(evs[3]["gate"], "cargo-test");
    assert_eq!(evs[3]["state"], "passed");
    assert_eq!(evs[3]["exit"], 0);
    assert!(
        evs[3]["detail"].as_str().unwrap().contains("298 passed"),
        "detail 应含测试数字: {}",
        evs[3]["detail"]
    );
    assert!(
        evs[1..].iter().all(|e| e["state"] != "running"),
        "折叠后不得残留 running"
    );
    // 折叠只做一次:pending_gate.json 消费后删除
    assert!(!pending_path(t.path()).exists(), "暂存应被消费删除");
    // ts:ISO8601 本地时区(带偏移)
    for e in &evs {
        assert!(
            is_iso8601_local(e["ts"].as_str().unwrap()),
            "ts 非本地 ISO8601 秒级: {}",
            e["ts"]
        );
    }
}

#[test]
fn failed_gate_fold() {
    let t = TempDir::new("failed");
    let payload = json!({
        "session_id": "s",
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "go test ./..."},
        "tool_response": {
            "stdout": "",
            "stderr": "FAIL\t./pkg [build failed]\nexit status 1\n",
            "interrupted": false,
            "status": 1
        }
    });
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&payload, t.path()), t.path()),
        "go test posttooluse",
    );
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_payload(), t.path()), t.path()),
        "stop",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 2);
    assert_eq!(
        [
            evs[0]["state"].as_str().unwrap(),
            evs[1]["state"].as_str().unwrap()
        ],
        ["running", "failed"]
    );
    assert_eq!(evs[1]["exit"], 1);
    assert_eq!(
        evs[1]["detail"], "exit status 1",
        "空 stdout 摘要取 stderr 末行"
    );
}

#[test]
fn utf8_payload_roundtrip() {
    let t = TempDir::new("utf8");
    let payload = json!({
        "session_id": "s",
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "echo 仪表盘", "description": "中文描述 ✓"},
        "tool_response": {"stdout": "ok\n", "status": 0}
    });
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&payload, t.path()), t.path()),
        "utf8 posttooluse",
    );
    let raw = fs::read_to_string(events_path(t.path())).expect("events.jsonl");
    assert!(raw.contains("echo 仪表盘"), "中文原文落盘(非转义): {raw}");
    let evs = read_events(t.path());
    assert_eq!(evs[0]["summary"], "echo 仪表盘");
}

#[test]
fn gate_commands() {
    let cases = [
        ("cargo test", Some("cargo-test")),
        ("cargo test --all --offline", Some("cargo-test")),
        (
            "cd /d/agentdash && cargo clippy -- -D warnings",
            Some("cargo-clippy"),
        ),
        ("cargo fmt --check", Some("cargo-fmt")),
        ("go test ./...", Some("go-test")),
        ("npm test -- --watchAll=false", Some("npm-test")),
        ("gh pr checks 12", Some("gh-pr-checks")),
        ("cargo  test", Some("cargo-test")), // 词间多空白(\s+)
        ("cargo\ttest", Some("cargo-test")), // 制表符空白
        ("npm run test", None),              // 连续词序列:中间隔 run,不得跨 token 误命中 npm-test
        ("cargo build && cargo test", Some("cargo-test")), // 后段含连续 cargo test
        ("go build ./x && go test", Some("go-test")), // 后段含连续 go test
        ("go build ./... && test", None),    // test 与 go 之间隔 build:不得跨 token 误命中
        ("go build x/testdata", None),       // test 藏于 testdata(词尾无边界)且与 go 不连续
        ("cargo build --release", None),     // 非验证门
        ("python -m unittest discover -v", None),
        ("xcargo test", None), // 词边界:前缀粘连不匹配
        ("", None),
    ];
    for (command, expected) in cases {
        let t = TempDir::new("gate");
        let payload = json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": command},
            "tool_response": {"stdout": "", "status": 0}
        });
        assert_silent_success(
            &feed_payload("posttooluse", &in_cwd(&payload, t.path()), t.path()),
            "gate replay",
        );
        let evs = read_events(t.path());
        assert_eq!(evs.len(), 1, "command={command:?} 恰一行");
        match expected {
            Some(gate) => {
                assert_eq!(evs[0]["kind"], "gate", "command={command:?}");
                assert_eq!(evs[0]["gate"], gate, "command={command:?}");
            }
            None => assert_eq!(evs[0]["kind"], "tool", "command={command:?}"),
        }
    }
}

#[test]
fn non_gate_bash_is_tool_event() {
    let t = TempDir::new("tool");
    let payload = json!({
        "session_id": "s",
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "ls -la"},
        "tool_response": {"stdout": "total 0\n", "status": 0}
    });
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&payload, t.path()), t.path()),
        "ls posttooluse",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0]["kind"], "tool");
    assert_eq!(evs[0]["tool"], "bash");
    assert_eq!(evs[0]["exit"], 0);
    assert_eq!(evs[0]["summary"], "ls -la");
}

#[test]
fn summary_truncated_to_80() {
    let t = TempDir::new("clip");
    let payload = json!({
        "session_id": "s",
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "x".repeat(200)},
        "tool_response": {"stdout": "", "status": 0}
    });
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&payload, t.path()), t.path()),
        "clip posttooluse",
    );
    let evs = read_events(t.path());
    assert_eq!(
        evs[0]["summary"].as_str().unwrap().chars().count(),
        80,
        "摘要按字符截 80"
    );
}

#[test]
fn exit_code_variants() {
    let cases = [
        (json!({"status": 0}), json!(0)),
        (json!({"exit_code": 3}), json!(3)),
        (json!({"interrupted": true, "status": 0}), json!(130)),
        (json!({"is_error": true}), json!(1)),
        // W4-001 改判:无退出码证据(缺字段/非对象/载荷缺失)记 null,不臆造 0
        (json!({"ok": true}), Value::Null),
        (json!("plain string response"), Value::Null),
        (Value::Null, Value::Null),
    ];
    for (response, expected) in cases {
        let t = TempDir::new("exit");
        let payload = json!({
            "session_id": "s",
            "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "ls"},
            "tool_response": response
        });
        assert_silent_success(
            &feed_payload("posttooluse", &in_cwd(&payload, t.path()), t.path()),
            "exit replay",
        );
        let evs = read_events(t.path());
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0]["exit"], expected, "response={response}");
    }
}

#[test]
fn summary_line_variants() {
    // 摘要提取经 gate 折叠路径可见(bash 非 gate 的 tool 行 summary 恒为命令本身);
    // 对象 fixture 显式带 status:0——本组只测摘要,不可知折叠语义由
    // unknown_exit_gate_folds_failed 专测钉住(W4-001)
    let cases = [
        (
            json!({"stdout": "a\n\nb  \n", "stderr": "", "status": 0}),
            "b",
        ),
        (
            json!({"stdout": "", "stderr": "boom\nboom\n", "status": 0}),
            "boom",
        ),
        (json!({"stdout": "", "status": 0}), ""),
        // 非对象 response:摘要为空,折叠走不可知 failed 路径
        (json!("not-a-dict"), "(exit unknown)"),
    ];
    for (response, expected) in cases {
        let t = TempDir::new("summary");
        let payload = json!({
            "session_id": "s",
            "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "cargo test"},
            "tool_response": response
        });
        assert_silent_success(
            &feed_payload("posttooluse", &in_cwd(&payload, t.path()), t.path()),
            "summary replay",
        );
        assert_silent_success(
            &feed_payload("stop", &in_cwd(&stop_payload(), t.path()), t.path()),
            "summary fold",
        );
        let evs = read_events(t.path());
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[1]["detail"], expected, "response={response}");
    }
}

#[test]
fn unknown_exit_gate_folds_failed_with_tailnote() {
    // W4-001 D1:gate 命中但 response 无退出码证据 → 折叠记 failed(不虚报
    // passed),exit 记 null,detail 补 `(exit unknown)` 尾注区分"真失败"与
    // "证据缺失";summary 在场时尾注接在摘要后。
    let t = TempDir::new("unknownexit");
    let payload = json!({
        "session_id": "s",
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "cargo test"},
        "tool_response": {"stdout": "ok 10\n", "stderr": ""}
    });
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&payload, t.path()), t.path()),
        "unknown gate running",
    );
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_payload(), t.path()), t.path()),
        "unknown gate fold",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 2);
    assert_eq!(evs[0]["state"], "running");
    assert_eq!(
        evs[1]["state"], "failed",
        "不可知折叠为 failed,不虚报 passed"
    );
    assert!(evs[1]["exit"].is_null(), "不可知 exit 记 null: {}", evs[1]);
    assert_eq!(evs[1]["detail"], "ok 10 (exit unknown)");

    // response 整体缺失:detail 只有尾注
    let t2 = TempDir::new("unknownexit2");
    let bare = json!({
        "session_id": "s",
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "cargo clippy"}
    });
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&bare, t2.path()), t2.path()),
        "bare gate running",
    );
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_payload(), t2.path()), t2.path()),
        "bare gate fold",
    );
    let evs = read_events(t2.path());
    assert_eq!(evs.len(), 2);
    assert_eq!(evs[1]["state"], "failed");
    assert_eq!(evs[1]["detail"], "(exit unknown)");
}

// ------------------------------------------------------------ 降级路径(铁律)

#[test]
fn invalid_json_stdin_degrades() {
    let t = TempDir::new("corrupt");
    assert_silent_success(
        &feed(&["hook", "posttooluse"], "{{{{not json\n", t.path()),
        "损坏 JSON",
    );
    assert!(!events_path(t.path()).exists(), "损坏输入不得落盘");
}

#[test]
fn empty_stdin_degrades() {
    let t = TempDir::new("empty");
    assert_silent_success(&feed(&["hook", "stop"], "", t.path()), "空 stdin");
    assert!(!events_path(t.path()).exists(), "空输入不得落盘");
}

#[test]
fn non_dict_payload_degrades() {
    let t = TempDir::new("nondict");
    assert_silent_success(
        &feed(
            &["hook", "posttooluse"],
            "[\"list\", \"not\", \"dict\"]",
            t.path(),
        ),
        "非对象载荷",
    );
    assert!(!events_path(t.path()).exists(), "非对象载荷不得落盘");
}

#[test]
fn stop_without_pending_writes_nothing() {
    let t = TempDir::new("nopending");
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_payload(), t.path()), t.path()),
        "无暂存 stop",
    );
    assert!(!events_path(t.path()).exists(), "无暂存不落任何行");
}

#[test]
fn stop_with_corrupt_pending_degrades() {
    let t = TempDir::new("corruptpending");
    let pending = pending_path(t.path());
    fs::create_dir_all(pending.parent().unwrap()).expect("mkdir .agentdash");
    fs::write(&pending, "{{corrupt").expect("write pending");
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_payload(), t.path()), t.path()),
        "损坏暂存 stop",
    );
    assert!(!pending.exists(), "损坏暂存应被消费删除");
    assert!(!events_path(t.path()).exists(), "损坏暂存不产生幽灵事件");
}

#[test]
fn payload_without_tool_name_skipped() {
    let t = TempDir::new("notool");
    let payload = json!({"hook_event_name": "PostToolUse", "cwd": t.path().to_string_lossy()});
    assert_silent_success(
        &feed_payload("posttooluse", &payload, t.path()),
        "无 tool_name",
    );
    assert!(!events_path(t.path()).exists(), "无 tool_name 不得落盘");
}

#[test]
fn unknown_event_arg_is_silent() {
    let t = TempDir::new("bogus");
    let payload = json!({"hook_event_name": "Stop", "cwd": t.path().to_string_lossy()});
    assert_silent_success(
        &feed(&["hook", "bogus"], &payload.to_string(), t.path()),
        "未知事件",
    );
    assert!(!events_path(t.path()).exists(), "未知事件不得落盘");
}

// ------------------------------------------------------------ PreToolUse dispatched(W3-003)

/// `PreToolUse` · 子代理派发工具(载荷形制承 Claude Code Task/Agent 新旧名)。
fn pre_tool_use_payload(tool: &str, input: &Value) -> Value {
    json!({
        "session_id": "s-w3-003",
        "hook_event_name": "PreToolUse",
        "tool_name": tool,
        "tool_input": input
    })
}

#[test]
fn pretooluse_task_dispatch_emits_agent_dispatched() {
    let t = TempDir::new("dispatch");
    let payload = pre_tool_use_payload(
        "Task",
        &json!({"agentType": "general-purpose", "description": "实现 W3", "prompt": "…"}),
    );
    assert_silent_success(
        &feed_payload("pretooluse", &in_cwd(&payload, t.path()), t.path()),
        "Task 派发回放",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 1, "Task 派发恰一行 dispatched");
    assert_eq!(evs[0]["kind"], "agent");
    assert_eq!(evs[0]["event"], "dispatched");
    assert_eq!(evs[0]["who"], "general-purpose");
    assert_eq!(evs[0]["task"], "实现 W3");
    assert!(
        is_iso8601_local(evs[0]["ts"].as_str().unwrap()),
        "ts 非本地 ISO8601 秒级: {}",
        evs[0]["ts"]
    );
}

#[test]
fn pretooluse_non_agent_tool_writes_nothing() {
    let t = TempDir::new("prebash");
    // 防御:matcher 之外直调 pretooluse 也不产事件(matcher 只兜宿主,不兜误调)
    let payload = pre_tool_use_payload(
        "Bash",
        &json!({"command": "cargo test", "description": "run tests"}),
    );
    assert_silent_success(
        &feed_payload("pretooluse", &in_cwd(&payload, t.path()), t.path()),
        "Bash 直调 pretooluse",
    );
    assert!(!events_path(t.path()).exists(), "非 agent 工具零写入");
}

#[test]
fn pretooluse_who_falls_back_to_agent_without_agent_type() {
    let t = TempDir::new("whofallback");
    let payload = pre_tool_use_payload("Agent", &json!({"prompt": "do something"}));
    assert_silent_success(
        &feed_payload("pretooluse", &in_cwd(&payload, t.path()), t.path()),
        "无 agentType 派发回放",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0]["event"], "dispatched");
    assert_eq!(evs[0]["who"], "agent", "who 缺省回退 agent");
    assert!(
        evs[0].get("task").is_none(),
        "无 description 应整字段省略: {}",
        evs[0]
    );
}

// ------------------------------------------------------------ 事件名回退与清单

#[test]
fn stop_folds_without_event_arg_via_payload_name() {
    let t = TempDir::new("noarg");
    assert_silent_success(
        &feed_payload(
            "posttooluse",
            &in_cwd(&cargo_test_post(), t.path()),
            t.path(),
        ),
        "cargo test posttooluse",
    );
    // 不传事件参数:载荷 hook_event_name 分派(老版本宿主防御)
    let stop = json!({"hook_event_name": "Stop", "cwd": t.path().to_string_lossy()});
    assert_silent_success(&feed(&["hook"], &stop.to_string(), t.path()), "无参 stop");
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 2);
    assert_eq!(
        [
            evs[0]["kind"].as_str().unwrap(),
            evs[1]["kind"].as_str().unwrap()
        ],
        ["gate", "gate"]
    );
    assert_eq!(evs[1]["state"], "passed");
}

#[test]
fn pretooluse_dispatches_without_event_arg_via_payload_name() {
    let t = TempDir::new("noargpre");
    // 不传事件参数:载荷 hook_event_name 分派(老版本宿主防御),与 stop 同法
    let payload = pre_tool_use_payload(
        "Task",
        &json!({"subagent_type": "Explore", "description": "长".repeat(100)}),
    );
    assert_silent_success(
        &feed(&["hook"], &in_cwd(&payload, t.path()).to_string(), t.path()),
        "无参 pretooluse",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0]["event"], "dispatched");
    assert_eq!(evs[0]["who"], "Explore", "agentType 缺位时取 subagent_type");
    assert_eq!(
        evs[0]["task"].as_str().unwrap().chars().count(),
        80,
        "task 按字符截 80: {}",
        evs[0]["task"]
    );
}

#[test]
fn hooks_json_registers_four_events_with_binary_command() {
    let path = Path::new(MANIFEST).join("kits/claude-code/hooks/hooks.json");
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("hooks.json readable"))
            .expect("hooks.json 合法 JSON");
    let hooks = manifest["hooks"].as_object().expect("hooks 对象");
    assert_eq!(hooks.len(), 4, "恰注册四事件");
    for event in ["PostToolUse", "Stop", "SubagentStop"] {
        let blocks = hooks[event]
            .as_array()
            .unwrap_or_else(|| panic!("{event} 无注册块"));
        assert!(!blocks.is_empty(), "{event} 无注册块");
        for block in blocks {
            assert!(
                block.get("matcher").is_none(),
                "{event} 不应设 matcher(全工具)"
            );
            for hook_entry in block["hooks"].as_array().expect("hook 列表") {
                let cmd = hook_entry["command"].as_str().expect("command");
                assert!(
                    cmd.contains("agentdash hook --host claude"),
                    "{event} 非二进制直调或缺宿主归属: {cmd}"
                );
                assert!(
                    cmd.ends_with("|| true"),
                    "{event} 缺 || true 静默保险: {cmd}"
                );
                assert!(
                    !cmd.contains("record_event.py") && !cmd.contains("python"),
                    "{event} 残留 Python 垫片: {cmd}"
                );
            }
        }
    }
    // PreToolUse:matcher 限定 Task/Agent 派发工具,命令同二进制直调 + || true
    let blocks = hooks["PreToolUse"].as_array().expect("PreToolUse 注册块");
    assert_eq!(blocks.len(), 1, "PreToolUse 恰一块");
    assert_eq!(
        blocks[0]["matcher"].as_str(),
        Some("Task|Agent"),
        "matcher 应限定派发工具 Task/Agent"
    );
    for hook_entry in blocks[0]["hooks"].as_array().expect("hook 列表") {
        let cmd = hook_entry["command"].as_str().expect("command");
        assert!(
            cmd.contains("agentdash hook --host claude pretooluse"),
            "PreToolUse 非二进制直调或缺宿主归属: {cmd}"
        );
        assert!(
            cmd.ends_with("|| true"),
            "PreToolUse 缺 || true 静默保险: {cmd}"
        );
    }
}

// ------------------------------------------------------------ 并发零丢失

#[test]
fn concurrent_appends_zero_loss() {
    const THREADS: usize = 8;
    const LINES_PER_THREAD: usize = 12;
    let t = TempDir::new("conc");
    let cwd = t.path().to_path_buf();
    let handles: Vec<_> = (0..THREADS)
        .map(|thread| {
            let cwd = cwd.clone();
            std::thread::spawn(move || {
                for line in 0..LINES_PER_THREAD {
                    let payload = json!({
                        "hook_event_name": "SubagentStop",
                        "cwd": cwd.to_string_lossy(),
                        "agent_name": format!("t{thread}-{line}")
                    });
                    let out = feed_payload("subagentstop", &payload, &cwd);
                    assert!(
                        out.status.success(),
                        "hook 失败: {}",
                        String::from_utf8_lossy(&out.stderr)
                    );
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("thread 完整结束");
    }

    let text = fs::read_to_string(events_path(&cwd)).expect("events.jsonl");
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut total = 0;
    for line in text.lines() {
        let ev: Value = serde_json::from_str(line).expect("每行都应是完整 JSON(零撕裂)");
        let who = ev["who"].as_str().expect("who 完整").to_owned();
        *counts.entry(who).or_insert(0) += 1;
        total += 1;
    }
    assert_eq!(
        total,
        THREADS * LINES_PER_THREAD,
        "零丢失:{THREADS} 线程 × {LINES_PER_THREAD} 行应全部落盘"
    );
    for thread in 0..THREADS {
        for line in 0..LINES_PER_THREAD {
            assert_eq!(
                counts[format!("t{thread}-{line}").as_str()],
                1,
                "t{thread}-{line} 应恰落一次"
            );
        }
    }
}

// ------------------------------------------------------------ 文件锁(陈锁自愈)

fn lock_path(cwd: &Path) -> PathBuf {
    cwd.join(".agentdash").join(".lock")
}

/// 预置 `.agentdash/.lock` 并把 mtime 回拨 `age_secs` 秒(0 = 新鲜锁),
/// 模拟持有方崩溃残留的陈锁 / 在途的正常锁。
fn seed_lock(cwd: &Path, age_secs: u64) {
    let lock = lock_path(cwd);
    fs::create_dir_all(lock.parent().unwrap()).expect("mkdir .agentdash");
    fs::write(&lock, b"held-by-ghost").expect("write lock");
    let f = fs::OpenOptions::new()
        .write(true)
        .open(&lock)
        .expect("open lock for backdate");
    f.set_times(
        fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(age_secs)),
    )
    .expect("backdate lock mtime");
}

fn subagentstop_payload(cwd: &Path, who: &str) -> Value {
    json!({
        "hook_event_name": "SubagentStop",
        "cwd": cwd.to_string_lossy(),
        "agent_name": who
    })
}

#[test]
fn stale_lock_self_heals_without_wait() {
    let t = TempDir::new("stalelock");
    seed_lock(t.path(), 30); // 超过 10s 陈锁阈值:应摘除立即重抢,不白等 LOCK_MAX_WAIT
    let start = Instant::now();
    let out = feed_payload(
        "subagentstop",
        &subagentstop_payload(t.path(), "after-stale"),
        t.path(),
    );
    let elapsed = start.elapsed();
    assert_silent_success(&out, "陈锁自愈回放");
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 1, "陈锁自愈后应正常落盘");
    assert_eq!(evs[0]["who"], "after-stale");
    assert!(
        elapsed < Duration::from_millis(1500),
        "陈锁应摘除重抢而非白等 LOCK_MAX_WAIT 再退化: {elapsed:?}"
    );
    assert!(
        !lock_path(t.path()).exists(),
        "自愈获取的锁退出时应删除(残留则后续 hook 继续白等)"
    );
}

#[test]
fn fresh_lock_still_waits_then_degrades_in_place() {
    let t = TempDir::new("freshlock");
    seed_lock(t.path(), 0); // 阈值内的新鲜锁:不摘,自旋超时后按降级铁律退化直接写
    let start = Instant::now();
    let out = feed_payload(
        "subagentstop",
        &subagentstop_payload(t.path(), "degraded"),
        t.path(),
    );
    let elapsed = start.elapsed();
    assert_silent_success(&out, "新鲜锁退化回放");
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 1, "退化路径仍应落盘(不丢事件)");
    assert_eq!(evs[0]["who"], "degraded");
    assert!(
        elapsed >= Duration::from_millis(1500),
        "新鲜锁应等满 LOCK_MAX_WAIT 再退化,不得误摘他人锁: {elapsed:?}"
    );
    assert!(lock_path(t.path()).exists(), "未持有锁不得删除他人锁文件");
}

// ------------------------------------------------------------ 多槽位暂存(W2-008)

#[test]
fn multi_slot_gates_fold_each_terminal() {
    let t = TempDir::new("multislot");
    let go_failed = json!({
        "session_id": "s",
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "go test ./..."},
        "tool_response": {
            "stdout": "",
            "stderr": "FAIL\t./pkg [build failed]\n",
            "interrupted": false,
            "status": 1
        }
    });
    // 同刻两个在途 gate:cargo-test(passed)+ go-test(failed),各占一槽
    assert_silent_success(
        &feed_payload(
            "posttooluse",
            &in_cwd(&cargo_test_post(), t.path()),
            t.path(),
        ),
        "gate 1 running",
    );
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&go_failed, t.path()), t.path()),
        "gate 2 running",
    );

    // 暂存中间态:数组两槽,各自的 exit/detail 配对
    let pending: Value =
        serde_json::from_str(&fs::read_to_string(pending_path(t.path())).expect("pending"))
            .expect("暂存应为合法 JSON");
    let slots = pending.as_array().expect("多槽位格式:顶层数组");
    assert_eq!(slots.len(), 2, "两个在途 gate 各占一槽");
    assert_eq!(slots[0]["gate"], "cargo-test");
    assert_eq!(slots[0]["exit"], 0);
    assert_eq!(slots[1]["gate"], "go-test");
    assert_eq!(slots[1]["exit"], 1);

    // Stop:全部折叠,各槽落各自终态(互不串档)
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_payload(), t.path()), t.path()),
        "stop fold all",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 4, "恰两 running + 两终态");
    assert_eq!(evs[0]["gate"], "cargo-test");
    assert_eq!(evs[0]["state"], "running");
    assert_eq!(evs[1]["gate"], "go-test");
    assert_eq!(evs[1]["state"], "running");
    assert_eq!(evs[2]["gate"], "cargo-test");
    assert_eq!(evs[2]["state"], "passed", "槽 1 折叠自身终态");
    assert_eq!(evs[2]["exit"], 0);
    assert_eq!(evs[3]["gate"], "go-test");
    assert_eq!(evs[3]["state"], "failed", "槽 2 折叠自身终态");
    assert_eq!(evs[3]["exit"], 1);
    assert_eq!(evs[3]["detail"], "FAIL\t./pkg [build failed]");
    assert!(!pending_path(t.path()).exists(), "暂存应被消费删除");
}

#[test]
fn legacy_single_object_pending_still_folds() {
    let t = TempDir::new("legacy");
    // 旧单对象格式(多槽位改造前落盘的暂存):读入兼容,折一槽
    let pending = pending_path(t.path());
    fs::create_dir_all(pending.parent().unwrap()).expect("mkdir .agentdash");
    fs::write(
        &pending,
        r#"{"gate":"cargo-clippy","exit":3,"detail":"warning: unused import"}"#,
    )
    .expect("write legacy pending");
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_payload(), t.path()), t.path()),
        "legacy fold",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 1, "旧格式恰折一槽");
    assert_eq!(evs[0]["gate"], "cargo-clippy");
    assert_eq!(evs[0]["state"], "failed");
    assert_eq!(evs[0]["exit"], 3);
    assert_eq!(evs[0]["detail"], "warning: unused import");
    assert!(!pending.exists(), "旧格式暂存同样消费删除");
}

// ------------------------------------------------------------ events 轮转(W2-008)

/// 预置 `.agentdash/events.jsonl` 为 `size` 字节的填充内容,并可选给旧
/// `events.jsonl.1` 写入哨兵内容(验证轮转覆盖)。
fn seed_events(cwd: &Path, size: usize, sentinel_dot1: bool) {
    let dir = cwd.join(".agentdash");
    fs::create_dir_all(&dir).expect("mkdir .agentdash");
    let mut blob = vec![b'x'; size];
    blob[size - 1] = b'\n';
    fs::write(dir.join("events.jsonl"), &blob).expect("seed events");
    if sentinel_dot1 {
        fs::write(dir.join("events.jsonl.1"), b"OLD-ROTATED-SENTINEL").expect("seed dot1");
    }
}

#[test]
fn oversized_events_rotate_to_dot1() {
    let t = TempDir::new("rotate");
    seed_events(t.path(), 6 * 1024 * 1024, true); // 6MB > 5MB 阈值
    assert_silent_success(
        &feed_payload(
            "subagentstop",
            &subagentstop_payload(t.path(), "after-rotate"),
            t.path(),
        ),
        "rotation replay",
    );
    let rotated = t.path().join(".agentdash").join("events.jsonl.1");
    let rotated_meta = fs::metadata(&rotated).expect("events.jsonl.1 应存在");
    assert_eq!(
        rotated_meta.len(),
        6 * 1024 * 1024,
        "轮转落点应承接收缩前的完整旧文件"
    );
    let rotated_text = fs::read_to_string(&rotated).expect("rotated readable");
    assert!(
        !rotated_text.contains("OLD-ROTATED-SENTINEL"),
        "新轮转应覆盖旧 .1"
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 1, "轮转后新建 events.jsonl 只含本次新行");
    assert_eq!(evs[0]["who"], "after-rotate");
}

#[test]
fn under_threshold_events_not_rotated() {
    let t = TempDir::new("norotate");
    seed_events(t.path(), 1024 * 1024, false); // 1MB < 5MB 阈值
    assert_silent_success(
        &feed_payload(
            "subagentstop",
            &subagentstop_payload(t.path(), "in-place"),
            t.path(),
        ),
        "no rotation replay",
    );
    assert!(
        !t.path().join(".agentdash").join("events.jsonl.1").exists(),
        "阈值内不得轮转"
    );
    // 1MB 填充非 JSON,不整文件反解:原文件保留且新行就地追加
    let raw = fs::read_to_string(events_path(t.path())).expect("events readable");
    assert!(raw.starts_with('x'), "阈值内原文件原样保留");
    assert!(raw.contains("\"who\":\"in-place\""), "新行同文件追加");
}

// ------------------------------------------------------------ --host 与 subagentstart(W7-001)

#[test]
fn host_flag_stamps_events_position_tolerant() {
    // 旗标在事件名前;事件落盘带 host
    let t = TempDir::new("hostflag");
    assert_silent_success(
        &feed(
            &["hook", "--host", "codex", "posttooluse"],
            &in_cwd(&cargo_test_post(), t.path()).to_string(),
            t.path(),
        ),
        "host 前置",
    );
    assert_silent_success(
        &feed(
            &["hook", "posttooluse", "--host=codex"],
            &in_cwd(&cargo_test_post(), t.path()).to_string(),
            t.path(),
        ),
        "host 后置 = 形态",
    );
    assert_silent_success(
        &feed(
            &["hook", "--host", "codex", "stop"],
            &in_cwd(&stop_payload(), t.path()).to_string(),
            t.path(),
        ),
        "stop 同盖 host",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 4, "两 running + 两折叠");
    for e in &evs {
        assert_eq!(e["host"], "codex", "每条事件都带归属: {e}");
    }
}

#[test]
fn no_host_flag_omits_field() {
    // 向后兼容:未传 --host 时事件不含 host 字段
    let t = TempDir::new("nohost");
    assert_silent_success(
        &feed_payload(
            "posttooluse",
            &in_cwd(&cargo_test_post(), t.path()),
            t.path(),
        ),
        "无 host 回放",
    );
    let evs = read_events(t.path());
    assert!(evs[0].get("host").is_none(), "未传旗标不得出现 host 字段");
}

#[test]
fn empty_host_value_is_ignored() {
    let t = TempDir::new("emptyhost");
    let out = feed(
        &["hook", "--host", "  ", "posttooluse"],
        &in_cwd(&cargo_test_post(), t.path()).to_string(),
        t.path(),
    );
    assert!(out.status.success());
    let evs = read_events(t.path());
    assert!(evs[0].get("host").is_none(), "空 host 视同未传");
}

#[test]
fn subagentstart_maps_to_dispatched_with_host() {
    let t = TempDir::new("substart");
    let payload = json!({
        "session_id": "s",
        "hook_event_name": "SubagentStart",
        "agent_type": "explore",
        "cwd": t.path().to_string_lossy()
    });
    assert_silent_success(
        &feed(
            &["hook", "--host=codex", "subagentstart"],
            &payload.to_string(),
            t.path(),
        ),
        "subagentstart 回放",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0]["kind"], "agent");
    assert_eq!(evs[0]["event"], "dispatched");
    assert_eq!(evs[0]["who"], "explore", "who 取 agent_type");
    assert_eq!(evs[0]["host"], "codex");
}
