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
        // W11-001:与 stop_payload() 同会话(池归属后 Stop 只折同会话槽)
        "session_id": "s-w1-008",
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
        // W4-001 改判:无退出码证据(缺字段/载荷缺失)记 null,不臆造 0
        (json!({"ok": true}), Value::Null),
        // W9-003:字符串 response 走 dsh-shell 渲染契约——尾部标记即退出码,
        // 皆无标记 = 干净退出 0(官方语义,DSH 桥实测)
        (json!("plain string response"), json!(0)),
        (json!("boom\n[exit code: 3]"), json!(3)),
        (json!("\n[killed by signal: SIGKILL]"), json!(130)),
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
        // W9-003:字符串 response 摘要取去尾标记后的正文末行
        (json!("not-a-dict"), "not-a-dict"),
        (
            json!("error: no manifest\n[exit code: 101]"),
            "error: no manifest",
        ),
        // 无标记且正文空:摘要为空(该形制的折叠终态由 dsh_string_* 用例覆盖)
        (json!(""), ""),
    ];
    for (response, expected) in cases {
        let t = TempDir::new("summary");
        let payload = json!({
            // W11-001:与 stop_payload() 同会话(池归属后 Stop 只折同会话槽)
            "session_id": "s-w1-008",
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
        // W11-001:与 stop_payload() 同会话(池归属后 Stop 只折同会话槽)
        "session_id": "s-w1-008",
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
        // W11-001:与 stop_payload() 同会话(池归属后 Stop 只折同会话槽)
        "session_id": "s-w1-008",
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
    // W11-001:与 cargo_test_post() 同会话(池归属后 Stop 只折同会话槽)
    let stop = json!({
        "session_id": "s-w1-008",
        "hook_event_name": "Stop",
        "cwd": t.path().to_string_lossy()
    });
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
fn hooks_json_registers_five_events_with_binary_command() {
    let path = Path::new(MANIFEST).join("kits/claude-code/hooks/hooks.json");
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("hooks.json readable"))
            .expect("hooks.json 合法 JSON");
    let hooks = manifest["hooks"].as_object().expect("hooks 对象");
    assert_eq!(hooks.len(), 5, "恰注册五事件");
    for event in ["PostToolUse", "PostToolUseFailure", "Stop", "SubagentStop"] {
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
        // W11-001:与 cargo_test_post()/stop_payload() 同会话(池归属后 Stop
        // 只折同会话槽,两槽须同池才能整折)
        "session_id": "s-w1-008",
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

// ------------------------------------------------------------ pending 池会话归属(W11-001)

/// 指定 session 的 gate 命中 PostToolUse(W11-001 双会话 fixture)。
fn gate_post_for(session: &str, command: &str, response: &Value) -> Value {
    json!({
        "session_id": session,
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": command},
        "tool_response": response
    })
}

/// 指定 session 的 Stop(W11-001 双会话 fixture)。
fn stop_for(session: &str) -> Value {
    json!({
        "session_id": session,
        "hook_event_name": "Stop",
        "stop_hook_active": true
    })
}

/// 读暂存并归一为槽数组(测试侧只读镜像 `pending_slots`)。
fn read_pending_slots(cwd: &Path) -> Vec<Value> {
    let raw = fs::read_to_string(pending_path(cwd)).expect("pending 可读");
    match serde_json::from_str::<Value>(&raw).expect("暂存应为合法 JSON") {
        Value::Array(items) => items,
        obj @ Value::Object(_) => vec![obj],
        other => panic!("暂存形状异常: {other}"),
    }
}

#[test]
fn stop_folds_only_own_session_slots() {
    // W10 活体发现的回归钉:双会话共享 pending_gate.json 时,A 的 Stop 只折
    // A 槽,B 槽原位保留等 B 自己的 Stop——嵌套会话不再互折退出证据。
    let t = TempDir::new("pool-own");
    let gate_a = gate_post_for(
        "sess-A",
        "cargo test --all",
        &json!({"stdout": "test result: ok. 9 passed\n", "status": 0}),
    );
    let gate_b = gate_post_for(
        "sess-B",
        "go test ./...",
        &json!({
            "stdout": "",
            "stderr": "FAIL\t./pkg [build failed]\nexit status 1\n",
            "status": 1
        }),
    );
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&gate_a, t.path()), t.path()),
        "A gate",
    );
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&gate_b, t.path()), t.path()),
        "B gate",
    );

    // 暂存中间态:两槽各记归属会话
    let slots = read_pending_slots(t.path());
    assert_eq!(slots.len(), 2);
    assert_eq!(slots[0]["session"], "sess-A", "槽位记录归属会话");
    assert_eq!(slots[1]["session"], "sess-B", "槽位记录归属会话");

    // A 的 Stop:只折 A 槽;B 槽原样保留
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_for("sess-A"), t.path()), t.path()),
        "A stop",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 3, "A/B 两 running + 仅 A 的折叠");
    assert_eq!(evs[0]["gate"], "cargo-test");
    assert_eq!(evs[1]["gate"], "go-test");
    assert_eq!(evs[2]["gate"], "cargo-test", "A 的 Stop 只折 A 的门");
    assert_eq!(evs[2]["state"], "passed");
    assert_eq!(evs[2]["exit"], 0);
    let slots = read_pending_slots(t.path());
    assert_eq!(slots.len(), 1, "B 槽未被 A 的 Stop 消费");
    assert_eq!(slots[0]["gate"], "go-test");
    assert_eq!(slots[0]["session"], "sess-B");

    // B 的 Stop:再折 B 槽,暂存消费删除
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_for("sess-B"), t.path()), t.path()),
        "B stop",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 4);
    assert_eq!(evs[3]["gate"], "go-test");
    assert_eq!(evs[3]["state"], "failed", "B 槽折叠自身终态");
    assert_eq!(evs[3]["exit"], 1);
    assert!(!pending_path(t.path()).exists(), "B 折叠后暂存消费删除");
}

#[test]
fn sessionless_default_pool_folds_mutually_as_today() {
    // 载荷无 session_id(default 池):行为与今日完全一致——首个 Stop 折叠
    // 全部 default 槽(跨会话 default 互折保持,规格 §1.1 明文);且不落
    // session 字段,与遗留格式同形,零迁移。
    let t = TempDir::new("pool-default");
    let a = json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "cargo test --all"},
        "tool_response": {"stdout": "test result: ok. 3 passed\n", "status": 0}
    });
    let b = json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "go test ./..."},
        "tool_response": {"stdout": "", "stderr": "FAIL\nexit status 1\n", "status": 1}
    });
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&a, t.path()), t.path()),
        "default gate A",
    );
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&b, t.path()), t.path()),
        "default gate B",
    );
    let slots = read_pending_slots(t.path());
    assert_eq!(slots.len(), 2);
    assert!(
        slots.iter().all(|s| s.get("session").is_none()),
        "无 session_id 载荷不落归属字段(default 池与遗留同形)"
    );

    // 无 session_id 的 Stop:default 池互折(同今日)
    let stop = json!({"hook_event_name": "Stop"});
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop, t.path()), t.path()),
        "stop",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 4, "两 running + 两折叠");
    assert_eq!(evs[2]["gate"], "cargo-test");
    assert_eq!(evs[2]["state"], "passed");
    assert_eq!(evs[3]["gate"], "go-test");
    assert_eq!(evs[3]["state"], "failed");
    assert!(!pending_path(t.path()).exists(), "default 池折叠后消费删除");
}

#[test]
fn stop_folds_own_plus_legacy_fieldless_slots() {
    // 遗留槽(旧格式落盘,无 session 字段)保持现行为:任意 Stop 折叠全部
    // (规格 §1.1:首个 Stop 折叠全部,向后兼容不做格式迁移)
    let t = TempDir::new("pool-legacy");
    let pending = pending_path(t.path());
    fs::create_dir_all(pending.parent().unwrap()).expect("mkdir .agentdash");
    fs::write(
        &pending,
        r#"{"gate":"cargo-clippy","exit":3,"detail":"warning: unused import"}"#,
    )
    .expect("write legacy pending");
    assert_silent_success(
        &feed_payload(
            "posttooluse",
            &in_cwd(
                &gate_post_for(
                    "sess-A",
                    "cargo test",
                    &json!({"stdout": "ok\n", "status": 0}),
                ),
                t.path(),
            ),
            t.path(),
        ),
        "A gate",
    );
    // A 的 Stop:遗留槽 + 同会话槽都折
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_for("sess-A"), t.path()), t.path()),
        "A stop",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 3, "running + 遗留槽折叠 + 自槽折叠");
    assert_eq!(evs[1]["gate"], "cargo-clippy");
    assert_eq!(evs[1]["state"], "failed");
    assert_eq!(evs[1]["exit"], 3);
    assert_eq!(evs[2]["gate"], "cargo-test");
    assert_eq!(evs[2]["state"], "passed");
    assert!(!pending.exists(), "全部消费删除");
}

#[test]
fn failure_supersede_pairs_same_session_only() {
    // posttoolusefailure 顶替同样只配本会话槽:B 的失败不得顶替 A 的在途
    // 证据(无同池槽 → 补 running+暂存对);A/B 各自的 Stop 各折各的证据。
    let t = TempDir::new("pool-supersede");
    let gate_a = gate_post_for(
        "sess-A",
        "cargo test --all",
        &json!({"stdout": "ok\n", "status": 0}),
    );
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&gate_a, t.path()), t.path()),
        "A gate",
    );
    let failure_b = json!({
        "session_id": "sess-B",
        "hook_event_name": "PostToolUseFailure",
        "tool_name": "Bash",
        "tool_input": {"command": "cargo test --all"},
        "tool_response": {"stderr": "error: test failed\n", "status": 101}
    });
    assert_silent_success(
        &feed_payload(
            "posttoolusefailure",
            &in_cwd(&failure_b, t.path()),
            t.path(),
        ),
        "B failure",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 2, "A running + B 补 running 对(未顶替)");
    let slots = read_pending_slots(t.path());
    assert_eq!(slots.len(), 2, "A 槽保留 + B 补新槽");
    assert_eq!(slots[0]["session"], "sess-A");
    assert_eq!(slots[0]["exit"], 0, "A 槽证据未被 B 的失败顶替");
    assert_eq!(slots[1]["session"], "sess-B");
    assert_eq!(slots[1]["exit"], 101);

    // A 的 Stop:A 槽以自身证据折叠 passed(B 的失败不串档)
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_for("sess-A"), t.path()), t.path()),
        "A stop",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 3);
    assert_eq!(evs[2]["gate"], "cargo-test");
    assert_eq!(evs[2]["state"], "passed");
    assert_eq!(evs[2]["exit"], 0);
    assert!(pending_path(t.path()).exists(), "B 槽仍在途");

    // B 的 Stop:B 槽折叠 failed 101
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_for("sess-B"), t.path()), t.path()),
        "B stop",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 4);
    assert_eq!(evs[3]["gate"], "cargo-test");
    assert_eq!(evs[3]["state"], "failed");
    assert_eq!(evs[3]["exit"], 101);
    assert!(!pending_path(t.path()).exists());
}

#[test]
fn failure_supersede_same_session_still_pairs() {
    // 同会话顶替不回退:failure 与在途槽同 session_id 时照旧换证据(位置
    // 不变、不加行),归属字段保留
    let t = TempDir::new("pool-supersede-own");
    let gate_a = gate_post_for(
        "sess-A",
        "cargo test --all",
        &json!({"stdout": "ok\n"}), // 无退出码证据:exit 暂存 null
    );
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&gate_a, t.path()), t.path()),
        "A gate",
    );
    let failure = json!({
        "session_id": "sess-A",
        "hook_event_name": "PostToolUseFailure",
        "tool_name": "Bash",
        "tool_input": {"command": "cargo test --all"},
        "tool_response": {"stderr": "FAIL ./pkg\nexit status 2\n", "status": 2}
    });
    assert_silent_success(
        &feed_payload("posttoolusefailure", &in_cwd(&failure, t.path()), t.path()),
        "A failure",
    );
    let slots = read_pending_slots(t.path());
    assert_eq!(slots.len(), 1, "同会话顶替:槽位不新增");
    assert_eq!(slots[0]["exit"], 2, "真证据换入");
    assert_eq!(slots[0]["session"], "sess-A", "归属字段保留");
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_for("sess-A"), t.path()), t.path()),
        "A stop",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 2, "running + 折叠,顶替不加行");
    assert_eq!(evs[1]["state"], "failed");
    assert_eq!(evs[1]["exit"], 2);
    assert!(!pending_path(t.path()).exists());
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

// ------------------------------------------------------------ Cursor 载荷词表(W9)

/// Cursor `afterShellExecution` 载荷形制:顶层 `command`+`output`,无
/// `tool_name`/`tool_input`;`hook_event_name` 保留宿主原名(官方 docs §Hooks)。
fn cursor_shell_payload(command: &str, output: &str, cwd: &Path) -> Value {
    json!({
        "conversation_id": "c-w9",
        "hook_event_name": "afterShellExecution",
        "command": command,
        "output": output,
        "workspace_roots": [cwd.to_string_lossy()],
        "cwd": cwd.to_string_lossy()
    })
}

#[test]
fn cursor_shell_gate_runs_then_folds_failed_exit_unknown() {
    // afterShellExecution 载荷无退出码证据:running 照发,折叠按铁律
    // failed + `(exit unknown)`——detail 里留 stdout 末行物证,不虚报通过
    let t = TempDir::new("cursorgate");
    let payload = cursor_shell_payload(
        "cargo test --quiet",
        "test result: ok. 9 passed\n",
        t.path(),
    );
    assert_silent_success(
        &feed(
            &["hook", "--host", "cursor", "posttooluse"],
            &in_cwd(&payload, t.path()).to_string(),
            t.path(),
        ),
        "cursor gate posttooluse",
    );
    assert_silent_success(
        &feed(
            &["hook", "--host", "cursor", "stop"],
            &in_cwd(&stop_payload(), t.path()).to_string(),
            t.path(),
        ),
        "cursor stop",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 2, "running + 折叠恰两行");
    assert_eq!(evs[0]["kind"], "gate");
    assert_eq!(evs[0]["gate"], "cargo-test");
    assert_eq!(evs[0]["state"], "running");
    assert_eq!(evs[0]["host"], "cursor");
    assert_eq!(evs[1]["state"], "failed", "无退出证据不得虚报 passed");
    assert_eq!(evs[1]["exit"], Value::Null, "exit 落 null 不臆造 0");
    assert_eq!(
        evs[1]["detail"], "test result: ok. 9 passed (exit unknown)",
        "detail 留 stdout 末行 + exit unknown 尾注"
    );
}

#[test]
fn cursor_shell_non_gate_is_bash_tool_event() {
    let t = TempDir::new("cursortool");
    let payload = cursor_shell_payload("echo 仪表盘", "仪表盘\n", t.path());
    assert_silent_success(
        &feed(
            &["hook", "--host", "cursor", "posttooluse"],
            &in_cwd(&payload, t.path()).to_string(),
            t.path(),
        ),
        "cursor 非 gate 回放",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0]["kind"], "tool");
    assert_eq!(evs[0]["tool"], "bash", "归一成 bash 工具行");
    assert_eq!(evs[0]["phase"], "end");
    assert_eq!(evs[0]["summary"], "echo 仪表盘");
    assert_eq!(evs[0]["exit"], Value::Null);
    assert_eq!(evs[0]["host"], "cursor");
}

#[test]
fn cursor_shape_requires_host_event_name_marker() {
    // 防御:顶层 command+output 但 hook_event_name 非 afterShellExecution
    // (亦无 tool_name)——不得凭形状误撞归一,静默零写入
    let t = TempDir::new("cursorguard");
    let payload = json!({
        "hook_event_name": "PostToolUse",
        "command": "cargo test",
        "output": "ok",
        "cwd": t.path().to_string_lossy()
    });
    assert_silent_success(
        &feed_payload("posttooluse", &payload, t.path()),
        "形状误撞防御",
    );
    assert!(!events_path(t.path()).exists(), "非 Cursor 词表不得落盘");
}

#[test]
fn subagentstop_who_falls_back_to_subagent_type() {
    // Cursor SubagentStop 载荷带 `subagent_type`(官方 docs §Hooks):agent_name
    // 缺席时回退取之;Claude 系 agent_name 优先序不变
    let t = TempDir::new("cursorsubstop");
    let payload = json!({
        "hook_event_name": "SubagentStop",
        "subagent_type": "generalPurpose",
        "cwd": t.path().to_string_lossy()
    });
    assert_silent_success(
        &feed(
            &["hook", "--host", "cursor", "subagentstop"],
            &payload.to_string(),
            t.path(),
        ),
        "cursor subagentstop 回放",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0]["kind"], "agent");
    assert_eq!(evs[0]["event"], "completed");
    assert_eq!(evs[0]["who"], "generalPurpose");
    assert_eq!(evs[0]["host"], "cursor");
}

#[test]
fn cursor_hooks_template_registers_four_events() {
    let path = Path::new(MANIFEST).join("kits/cursor/hooks.template.json");
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("hooks.template.json readable"))
            .expect("hooks.template.json 合法 JSON");
    assert_eq!(manifest["version"], 1, "Cursor hooks schema version 1");
    let hooks = manifest["hooks"].as_object().expect("hooks 对象");
    assert_eq!(hooks.len(), 5, "恰注册五事件");
    for event in [
        "afterShellExecution",
        "postToolUseFailure",
        "subagentStart",
        "subagentStop",
        "stop",
    ] {
        let blocks = hooks[event]
            .as_array()
            .unwrap_or_else(|| panic!("{event} 无注册块"));
        assert_eq!(blocks.len(), 1, "{event} 恰一块");
        // Cursor 形制:定义直接平铺(无 Claude 系内层 hooks 数组)
        assert!(
            blocks[0].get("hooks").is_none(),
            "{event} 应为平铺定义,非 Claude 形制"
        );
        let cmd = blocks[0]["command"].as_str().expect("command");
        assert!(
            cmd.contains("agentdash hook --host cursor"),
            "{event} 非二进制直调或缺宿主归属: {cmd}"
        );
        assert!(
            cmd.ends_with("|| true"),
            "{event} 缺 || true 静默保险: {cmd}"
        );
    }
    // afterShellExecution 归一为 agentdash posttooluse;stop/subagent* 同名直映
    assert!(
        hooks["afterShellExecution"][0]["command"]
            .as_str()
            .unwrap()
            .contains(" posttooluse"),
        "afterShellExecution 应映射 posttooluse"
    );
    // postToolUseFailure 映射 agentdash posttoolusefailure(失败证据配对,W9-003)
    assert!(
        hooks["postToolUseFailure"][0]["command"]
            .as_str()
            .unwrap()
            .contains(" posttoolusefailure"),
        "postToolUseFailure 应映射 posttoolusefailure"
    );
}

#[test]
fn deepseek_hooks_registers_claude_form_five_events() {
    // DSH 桥消费 Claude Code 形制;五事件含 SubagentStart(桥原生支持),
    // 命令全部 --host deepseek 归属
    let path = Path::new(MANIFEST).join("kits/deepseek/hooks.json");
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("hooks.json readable"))
            .expect("hooks.json 合法 JSON");
    let hooks = manifest["hooks"].as_object().expect("hooks 对象");
    assert_eq!(hooks.len(), 5, "恰注册五事件");
    for event in [
        "PostToolUse",
        "PreToolUse",
        "SubagentStart",
        "SubagentStop",
        "Stop",
    ] {
        let blocks = hooks[event]
            .as_array()
            .unwrap_or_else(|| panic!("{event} 无注册块"));
        assert_eq!(blocks.len(), 1, "{event} 恰一块");
        let hook_entry = &blocks[0]["hooks"][0];
        let cmd = hook_entry["command"].as_str().expect("command");
        assert!(
            cmd.contains("agentdash hook --host deepseek"),
            "{event} 非二进制直调或缺宿主归属: {cmd}"
        );
        assert!(
            cmd.ends_with("|| true"),
            "{event} 缺 || true 静默保险: {cmd}"
        );
    }
    assert_eq!(
        hooks["PreToolUse"][0]["matcher"].as_str(),
        Some("Task|Agent"),
        "PreToolUse 应限定派发工具"
    );
}

// ------------------------------------------------------------ PostToolUseFailure 失败证据配对(W9-003)

#[test]
fn failure_event_supersedes_pending_gate_with_real_exit() {
    // zcode 形制:posttooluse 先落 exit=null 暂存,failure 事件顶替为真证据
    // (exit=1),Stop 折叠 failed 且不再带 `(exit unknown)` 尾注
    let t = TempDir::new("failpair");
    let payload = json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "go test ./..."},
        "tool_response": {"stdout": "ok\n"},
        "cwd": t.path().to_string_lossy()
    });
    assert_silent_success(
        &feed_payload("posttooluse", &payload, t.path()),
        "posttooluse 回执",
    );
    let failure = json!({
        "hook_event_name": "PostToolUseFailure",
        "tool_name": "Bash",
        "tool_input": {"command": "go test ./..."},
        "tool_response": {"stderr": "FAIL ./pkg\nexit status 2\n", "status": 2},
        "cwd": t.path().to_string_lossy()
    });
    assert_silent_success(
        &feed(
            &["hook", "--host", "zcode", "posttoolusefailure"],
            &in_cwd(&failure, t.path()).to_string(),
            t.path(),
        ),
        "failure 回执",
    );
    assert_silent_success(
        &feed(
            &["hook", "--host", "zcode", "stop"],
            &in_cwd(&stop_payload(), t.path()).to_string(),
            t.path(),
        ),
        "stop(折叠行 host 戳取自 stop 调用)",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 2, "running + 折叠,顶替不加行");
    assert_eq!(evs[1]["state"], "failed");
    assert_eq!(evs[1]["exit"], 2, "载荷显式退出码优先");
    assert_eq!(
        evs[1]["detail"], "exit status 2",
        "无 (exit unknown) 尾注——真证据在案"
    );
    assert_eq!(evs[1]["host"], "zcode");
}

#[test]
fn failure_event_without_prior_posttooluse_emits_running_pair() {
    // 宿主失败路径未先发 posttooluse:failure 臂补 running+暂存原子对,
    // Stop 折叠仍有源;缺退出码字段落 1(失败事件在场即非零证据)
    let t = TempDir::new("failsolo");
    let failure = json!({
        "hook_event_name": "PostToolUseFailure",
        "tool_name": "Bash",
        "tool_input": {"command": "cargo clippy -- -D warnings"},
        "error_message": "one or more warnings produced",
        "cwd": t.path().to_string_lossy()
    });
    assert_silent_success(
        &feed(
            &["hook", "--host", "zcode", "posttoolusefailure"],
            &failure.to_string(),
            t.path(),
        ),
        "failure 单发",
    );
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_payload(), t.path()), t.path()),
        "stop",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 2);
    assert_eq!(evs[0]["state"], "running");
    assert_eq!(evs[1]["state"], "failed");
    assert_eq!(evs[1]["exit"], 1, "缺码落 1 不落 0/unknown");
    assert_eq!(evs[1]["detail"], "one or more warnings produced");
}

#[test]
fn failure_event_interrupted_maps_130() {
    let t = TempDir::new("failint");
    let failure = json!({
        "hook_event_name": "PostToolUseFailure",
        "tool_name": "Bash",
        "tool_input": {"command": "npm test"},
        "is_interrupt": true,
        "cwd": t.path().to_string_lossy()
    });
    assert_silent_success(
        &feed(
            &["hook", "posttoolusefailure"],
            &in_cwd(&failure, t.path()).to_string(),
            t.path(),
        ),
        "中断失败",
    );
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_payload(), t.path()), t.path()),
        "stop",
    );
    let evs = read_events(t.path());
    assert_eq!(
        evs[1]["exit"], 130,
        "is_interrupt 映射 130(承 interrupted 约定)"
    );
}

#[test]
fn failure_event_claude_error_head_carries_real_exit() {
    // claude 形制(W11-002 真机捕获):失败侧**无 tool_response**,真退出码在
    // 顶层 `error` 串头 `Exit code 101`;fold 落 101 + error 尾行摘要,无尾注
    let t = TempDir::new("failclaude");
    let failure = json!({
        "hook_event_name": "PostToolUseFailure",
        "tool_name": "Bash",
        "tool_input": {"command": "cargo test", "timeout": 600_000},
        "error": "Exit code 101\n    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.31s\n\nrunning 1 test\ntest tests::t ... FAILED\n\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n\nerror: test failed, to rerun pass `--lib`",
        "is_interrupt": false,
        "cwd": t.path().to_string_lossy()
    });
    assert_silent_success(
        &feed(
            &["hook", "--host", "claude", "posttoolusefailure"],
            &in_cwd(&failure, t.path()).to_string(),
            t.path(),
        ),
        "claude failure 单发",
    );
    assert_silent_success(
        &feed(
            &["hook", "--host", "claude", "stop"],
            &in_cwd(&stop_payload(), t.path()).to_string(),
            t.path(),
        ),
        "stop(折叠行 host 戳取自 stop 调用)",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 2);
    assert_eq!(evs[1]["state"], "failed");
    assert_eq!(
        evs[1]["exit"], 101,
        "error 串头 Exit code N 即真退出码,不落猜测 1"
    );
    assert_eq!(
        evs[1]["detail"], "error: test failed, to rerun pass `--lib`",
        "error 串取末非空行摘要,无 (exit unknown) 尾注"
    );
    assert_eq!(evs[1]["host"], "claude");
}

#[test]
fn failure_event_error_head_accepts_error_prefix_variant() {
    // claude transcript 形制变体:`Error: Exit code 2` 前缀同样解析
    let t = TempDir::new("failerrpre");
    let failure = json!({
        "hook_event_name": "PostToolUseFailure",
        "tool_name": "Bash",
        "tool_input": {"command": "cargo clippy -- -D warnings"},
        "error": "Error: Exit code 2\nwarning: unused variable\ncould not compile",
        "cwd": t.path().to_string_lossy()
    });
    assert_silent_success(
        &feed(
            &["hook", "--host", "claude", "posttoolusefailure"],
            &in_cwd(&failure, t.path()).to_string(),
            t.path(),
        ),
        "Error: 前缀变体",
    );
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_payload(), t.path()), t.path()),
        "stop",
    );
    let evs = read_events(t.path());
    assert_eq!(evs[1]["exit"], 2, "Error: Exit code N 变体解析");
    assert_eq!(evs[1]["detail"], "could not compile", "末非空行摘要");
}

#[test]
fn failure_event_error_head_zero_or_garbage_falls_to_nonzero_default() {
    // 失败事件在场即非零证据(铁律):串头 `Exit code 0`/非数字码视同无码,落 1
    let t = TempDir::new("failzero");
    let failure = json!({
        "hook_event_name": "PostToolUseFailure",
        "tool_name": "Bash",
        "tool_input": {"command": "cargo test"},
        "error": "Exit code 0\nparadoxical failure",
        "cwd": t.path().to_string_lossy()
    });
    assert_silent_success(
        &feed(
            &["hook", "--host", "claude", "posttoolusefailure"],
            &in_cwd(&failure, t.path()).to_string(),
            t.path(),
        ),
        "Exit code 0",
    );
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_payload(), t.path()), t.path()),
        "stop",
    );
    let evs = read_events(t.path());
    assert_eq!(evs[1]["exit"], 1, "串头 0 不认,落默认 1");
    assert_eq!(evs[1]["detail"], "paradoxical failure");
}

#[test]
fn failure_event_cursor_vocab_attributes_via_top_level_command() {
    let t = TempDir::new("failcursor");
    let failure = json!({
        "hook_event_name": "postToolUseFailure",
        "command": "cargo test",
        "error_message": "1 test failed",
        "cwd": t.path().to_string_lossy()
    });
    assert_silent_success(
        &feed(
            &["hook", "--host", "cursor", "posttoolusefailure"],
            &failure.to_string(),
            t.path(),
        ),
        "cursor failure",
    );
    assert_silent_success(
        &feed(
            &["hook", "--host", "cursor", "stop"],
            &in_cwd(&stop_payload(), t.path()).to_string(),
            t.path(),
        ),
        "stop",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 2);
    assert_eq!(evs[0]["gate"], "cargo-test");
    assert_eq!(evs[1]["state"], "failed");
    assert_eq!(evs[1]["exit"], 1);
    assert_eq!(evs[1]["host"], "cursor");
}

#[test]
fn failure_event_without_command_writes_nothing() {
    // cursor error 载荷无 command 字段(官方词表仅 error_message/failure_type/
    // duration/is_interrupt):不可归因,零写入——宁缺毋造
    let t = TempDir::new("failnocmd");
    let failure = json!({
        "hook_event_name": "postToolUseFailure",
        "error_message": "boom",
        "cwd": t.path().to_string_lossy()
    });
    assert_silent_success(
        &feed(
            &["hook", "--host", "cursor", "posttoolusefailure"],
            &failure.to_string(),
            t.path(),
        ),
        "无 command 失败载荷",
    );
    assert!(!events_path(t.path()).exists(), "不可归因不得落盘");
}

#[test]
fn failure_event_non_gate_command_writes_nothing() {
    let t = TempDir::new("failnongate");
    let failure = json!({
        "hook_event_name": "PostToolUseFailure",
        "tool_name": "Bash",
        "tool_input": {"command": "echo boom"},
        "cwd": t.path().to_string_lossy()
    });
    assert_silent_success(
        &feed(
            &["hook", "posttoolusefailure"],
            &in_cwd(&failure, t.path()).to_string(),
            t.path(),
        ),
        "非验证门失败",
    );
    assert!(!events_path(t.path()).exists(), "非验证门零写入");
}

// ------------------------------------------------------------ 字符串 response(DSH 桥,W9-003 实测)

/// DSH 桥载荷形制:`tool_response` 是纯文本,bash 渲染器对失败追加尾部标记
/// (dsh-shell 渲染契约:`[exit code: N]` / `[killed by signal: X]`,皆无 =
/// 干净退出 0)。
fn dsh_string_post(command: &str, response: &str, cwd: &Path) -> Value {
    json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "bash",
        "tool_input": {"command": command},
        "tool_response": response,
        "cwd": cwd.to_string_lossy()
    })
}

#[test]
fn dsh_string_success_marker_free_folds_passed() {
    // 皆无标记 = 干净退出 0(dsh-shell parseExitStatus 官方语义):
    // deepseek 宿主的绿门禁折叠为真 passed
    let t = TempDir::new("dshpass");
    let payload = dsh_string_post(
        "cargo test --quiet",
        "running 9 tests\ntest result: ok. 9 passed\n",
        t.path(),
    );
    assert_silent_success(
        &feed(
            &["hook", "--host", "deepseek", "posttooluse"],
            &in_cwd(&payload, t.path()).to_string(),
            t.path(),
        ),
        "dsh 绿门禁回执",
    );
    assert_silent_success(
        &feed(
            &["hook", "--host", "deepseek", "stop"],
            &in_cwd(&stop_payload(), t.path()).to_string(),
            t.path(),
        ),
        "stop",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 2);
    assert_eq!(evs[1]["state"], "passed", "无标记=退出 0,官方契约");
    assert_eq!(evs[1]["exit"], 0);
    assert_eq!(evs[1]["detail"], "test result: ok. 9 passed");
    assert_eq!(evs[1]["host"], "deepseek");
}

#[test]
fn dsh_string_failure_marker_folds_failed_with_real_exit() {
    let t = TempDir::new("dshfail");
    let payload = dsh_string_post(
        "cargo test",
        "error: could not find Cargo.toml\n[exit code: 101]",
        t.path(),
    );
    assert_silent_success(
        &feed(
            &["hook", "--host", "deepseek", "posttooluse"],
            &in_cwd(&payload, t.path()).to_string(),
            t.path(),
        ),
        "dsh 失败回执",
    );
    assert_silent_success(
        &feed(
            &["hook", "--host", "deepseek", "stop"],
            &in_cwd(&stop_payload(), t.path()).to_string(),
            t.path(),
        ),
        "stop",
    );
    let evs = read_events(t.path());
    assert_eq!(evs[1]["state"], "failed");
    assert_eq!(evs[1]["exit"], 101, "尾部标记即真退出码");
    assert_eq!(
        evs[1]["detail"], "error: could not find Cargo.toml",
        "detail 不含标记行"
    );
}

#[test]
fn dsh_string_killed_by_signal_maps_130() {
    let t = TempDir::new("dshkill");
    let payload = dsh_string_post("npm test", "\n[killed by signal: SIGKILL]", t.path());
    assert_silent_success(
        &feed(
            &["hook", "--host", "deepseek", "posttooluse"],
            &in_cwd(&payload, t.path()).to_string(),
            t.path(),
        ),
        "信号杀死回执",
    );
    assert_silent_success(
        &feed(
            &["hook", "--host", "deepseek", "stop"],
            &in_cwd(&stop_payload(), t.path()).to_string(),
            t.path(),
        ),
        "stop",
    );
    let evs = read_events(t.path());
    assert_eq!(evs[1]["state"], "failed");
    assert_eq!(evs[1]["exit"], 130, "信号杀死承 interrupted 的 130 约定");
}

// ------------------------------------------------------------ 用户自定义 gate(W10-003)

/// 预置 `.agentdash/config.json`(用户自定义 gate 词表,整文件覆写)。
fn seed_config(cwd: &Path, text: &str) {
    let dir = cwd.join(".agentdash");
    fs::create_dir_all(&dir).expect("mkdir .agentdash");
    fs::write(dir.join("config.json"), text).expect("write config.json");
}

/// `PostToolUse` · bash `pytest -q`(W10-003 用户词表演练命令,非内置门)。
fn pytest_post() -> Value {
    json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "pytest -q"},
        "tool_response": {"stdout": "9 passed in 0.31s\n", "status": 0}
    })
}

#[test]
fn custom_gate_config_hits_user_words_alongside_builtins() {
    let t = TempDir::new("usergate");
    seed_config(t.path(), r#"{"gates":[{"name":"ut","words":["pytest"]}]}"#);
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&pytest_post(), t.path()), t.path()),
        "用户词表回放",
    );
    assert_silent_success(
        &feed_payload(
            "posttooluse",
            &in_cwd(&cargo_test_post(), t.path()),
            t.path(),
        ),
        "内置门共存回放",
    );
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_payload(), t.path()), t.path()),
        "stop 折叠",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 4, "两 running + 两折叠");
    assert_eq!(evs[0]["kind"], "gate");
    assert_eq!(evs[0]["gate"], "ut", "用户 gate 名原样入账");
    assert_eq!(evs[0]["state"], "running");
    assert_eq!(evs[1]["gate"], "cargo-test", "内置门与用户表共存");
    assert_eq!(evs[2]["gate"], "ut");
    assert_eq!(evs[2]["state"], "passed", "用户 gate 折叠终态");
    assert_eq!(evs[2]["exit"], 0);
    assert_eq!(evs[3]["gate"], "cargo-test");
    assert_eq!(evs[3]["state"], "passed");
}

#[test]
fn custom_gate_shares_word_sequence_semantics() {
    // 自定义表复用既有词序列机(非正则、非子串):连续词 + 词边界,同内置语义
    let t = TempDir::new("userseq");
    seed_config(
        t.path(),
        r#"{"gates":[{"name":"make-check","words":["make","check"]}]}"#,
    );
    let cases = [
        ("make check", true),
        ("make  check", true),      // 词间多空白(\s+ 语义)
        ("make && check", true),    // && 分隔符
        ("make all check", false),  // 中间隔 token:不跨
        ("makefile check", false),  // 词边界:前缀粘连不匹配
        ("make checkstyle", false), // 词尾边界:check 粘连 style 不匹配
    ];
    for (command, _) in cases {
        let payload = json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": command},
            "tool_response": {"stdout": "ok\n", "status": 0}
        });
        assert_silent_success(
            &feed_payload("posttooluse", &in_cwd(&payload, t.path()), t.path()),
            "词序列回放",
        );
    }
    let evs = read_events(t.path());
    assert_eq!(evs.len(), cases.len());
    for (idx, (command, is_gate)) in cases.iter().enumerate() {
        assert_eq!(
            evs[idx]["kind"].as_str() == Some("gate"),
            *is_gate,
            "command={command:?} 落了 {} 行",
            evs[idx]["kind"]
        );
    }
}

#[test]
fn invalid_config_shapes_fall_back_to_builtins_whole_table() {
    // 任一形状错/越界 → 整表静默回退(表内合法条目同弃),hook 照常退 0
    // 且零 stderr;config 解析自身不产生任何事件(每例恰增一行)
    let entries_33 = {
        let mut gates: Vec<String> = (0..32)
            .map(|i| format!(r#"{{"name":"g{i}","words":["w{i}"]}}"#))
            .collect();
        gates.push(r#"{"name":"Overflow","words":["w32"]}"#.to_owned());
        format!(r#"{{"gates":[{}]}}"#, gates.join(","))
    };
    let cases: Vec<(&str, &str, &str)> = vec![
        ("损坏 JSON", "{{{ not json", "pytest -q"),
        (
            "顶层非对象",
            r#"[{"gates":[{"name":"ut","words":["pytest"]}]}]"#,
            "pytest -q",
        ),
        ("gates 键缺失", "{}", "pytest -q"),
        (
            "gates 非数组",
            r#"{"gates":{"name":"ut","words":["pytest"]}}"#,
            "pytest -q",
        ),
        ("条目非对象", r#"{"gates":["ut"]}"#, "pytest -q"),
        (
            "name 非字符串",
            r#"{"gates":[{"name":3,"words":["pytest"]}]}"#,
            "pytest -q",
        ),
        (
            "name 空串",
            r#"{"gates":[{"name":"","words":["pytest"]}]}"#,
            "pytest -q",
        ),
        (
            "name 字符集越界(合法条目同弃)",
            r#"{"gates":[{"name":"ut","words":["pytest"]},{"name":"Build","words":["pytest"]}]}"#,
            "pytest -q",
        ),
        ("words 缺失", r#"{"gates":[{"name":"ut"}]}"#, "pytest -q"),
        (
            "words 非数组",
            r#"{"gates":[{"name":"ut","words":"pytest"}]}"#,
            "pytest -q",
        ),
        (
            "words 空表",
            r#"{"gates":[{"name":"ut","words":[]}]}"#,
            "pytest -q",
        ),
        (
            "词含空串",
            r#"{"gates":[{"name":"ut","words":["pytest",""]}]}"#,
            "pytest -q",
        ),
        (
            "词非字符串",
            r#"{"gates":[{"name":"ut","words":["pytest",3]}]}"#,
            "pytest -q",
        ),
        (
            "words 超 8 词",
            r#"{"gates":[{"name":"ut","words":["w1","w2","w3","w4","w5","w6","w7","w8","w9"]}]}"#,
            "pytest -q",
        ),
        ("超 32 条(首条合法同弃)", &entries_33, "w0"),
    ];
    let t = TempDir::new("badshape");
    let mut lines = 0usize;
    for (label, config, command) in &cases {
        seed_config(t.path(), config);
        let payload = json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": command},
            "tool_response": {"stdout": "ok\n", "status": 0}
        });
        assert_silent_success(
            &feed_payload("posttooluse", &in_cwd(&payload, t.path()), t.path()),
            label,
        );
        lines += 1;
        let evs = read_events(t.path());
        assert_eq!(evs.len(), lines, "{label}: 恰增一行");
        assert_eq!(
            evs.last().unwrap()["kind"],
            "tool",
            "{label}: 整表回退,用户词不得命中"
        );
    }
    // 损坏 config 在场时内置门照常(显式配对钉死)
    seed_config(t.path(), "{{{ not json");
    assert_silent_success(
        &feed_payload(
            "posttooluse",
            &in_cwd(&cargo_test_post(), t.path()),
            t.path(),
        ),
        "损坏 config 内置门",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.last().unwrap()["gate"], "cargo-test");
}

#[test]
fn custom_gate_name_clipped_to_40_chars() {
    // name 超 40 字符按规格截断(长度是归一化,字符集才是裁断)
    let t = TempDir::new("nameclip");
    let long_name = "a".repeat(45);
    let config = format!(r#"{{"gates":[{{"name":"{long_name}","words":["pytest"]}}]}}"#);
    seed_config(t.path(), &config);
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&pytest_post(), t.path()), t.path()),
        "超长名回放",
    );
    let evs = read_events(t.path());
    assert_eq!(evs[0]["kind"], "gate");
    let gate = evs[0]["gate"].as_str().expect("gate 名");
    assert_eq!(gate.chars().count(), 40, "gate 名截 40: {gate}");
}

#[test]
fn user_table_same_name_overrides_builtin() {
    let t = TempDir::new("override");
    seed_config(
        t.path(),
        r#"{"gates":[{"name":"cargo-test","words":["cargo","check"]}]}"#,
    );
    // 用户表先行:`cargo check` 命中用户 cargo-test(内置词表无此门)
    let payload = json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "cargo check"},
        "tool_response": {"stdout": "ok\n", "status": 0}
    });
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&payload, t.path()), t.path()),
        "同名覆盖回放",
    );
    // 用户表未命中的词回落内置
    assert_silent_success(
        &feed_payload(
            "posttooluse",
            &in_cwd(&cargo_test_post(), t.path()),
            t.path(),
        ),
        "回落内置",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 2);
    assert_eq!(evs[0]["kind"], "gate");
    assert_eq!(evs[0]["gate"], "cargo-test", "同名覆盖:用户词表命中内置名");
    assert_eq!(evs[1]["gate"], "cargo-test", "用户未命中回落内置词表");
}

#[test]
fn empty_gates_array_is_valid_means_builtins_only() {
    let t = TempDir::new("emptygates");
    seed_config(t.path(), r#"{"gates":[]}"#);
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&pytest_post(), t.path()), t.path()),
        "空表回放",
    );
    assert_silent_success(
        &feed_payload(
            "posttooluse",
            &in_cwd(&cargo_test_post(), t.path()),
            t.path(),
        ),
        "内置门回放",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 2);
    assert_eq!(
        evs[0]["kind"], "tool",
        "空 gates 数组合法:仅内置,pytest 不命中"
    );
    assert_eq!(evs[1]["gate"], "cargo-test");
}

// ---------------------------------------------------------------- ADE attribution (feat/ade-attribution)

#[test]
fn session_id_persists_on_tool_gate_and_agent_lines() {
    let t = TempDir::new("ade-sid");
    assert_silent_success(
        &feed_payload(
            "posttooluse",
            &in_cwd(&cargo_test_post(), t.path()),
            t.path(),
        ),
        "gate running",
    );
    let edit = json!({
        "session_id": "s-w1-008",
        "tool_name": "Edit",
        "tool_input": {"file_path": "a.rs"},
        "tool_response": {}
    });
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&edit, t.path()), t.path()),
        "tool",
    );
    assert_silent_success(
        &feed_payload("stop", &in_cwd(&stop_payload(), t.path()), t.path()),
        "stop fold",
    );
    let evs = read_events(t.path());
    assert!(
        evs.iter()
            .any(|e| e["kind"] == "gate" && e["state"] == "running")
    );
    assert!(evs.iter().any(|e| e["kind"] == "tool"));
    assert!(
        evs.iter()
            .any(|e| e["kind"] == "gate" && e["state"] != "running")
    );
    for ev in &evs {
        assert_eq!(
            ev["session_id"], "s-w1-008",
            "session_id must persist on kind={}",
            ev["kind"]
        );
    }
}

#[test]
fn missing_session_id_stays_legacy_shape() {
    let t = TempDir::new("ade-nosid");
    let payload = json!({
        "tool_name": "Bash",
        "tool_input": {"command": "echo hi"},
        "tool_response": {"stdout": "hi"}
    });
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&payload, t.path()), t.path()),
        "legacy tool",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0]["kind"], "tool");
    assert!(
        evs[0].get("session_id").is_none(),
        "legacy payload must not grow a session_id field"
    );
}

#[test]
fn subagentstop_folds_pending_for_that_session() {
    let t = TempDir::new("ade-subfold");
    assert_silent_success(
        &feed_payload(
            "posttooluse",
            &in_cwd(&cargo_test_post(), t.path()),
            t.path(),
        ),
        "open gate",
    );
    let pending = fs::read_to_string(pending_path(t.path())).expect("pending exists");
    assert!(pending.contains("s-w1-008"), "slot keyed by session");
    let stop = json!({
        "session_id": "s-w1-008",
        "hook_event_name": "SubagentStop",
        "agent_name": "worker"
    });
    assert_silent_success(
        &feed_payload("subagentstop", &in_cwd(&stop, t.path()), t.path()),
        "subagentstop should fold",
    );
    assert!(
        !pending_path(t.path()).exists()
            || fs::read_to_string(pending_path(t.path())).unwrap_or_default() == "[]",
        "session slot must be folded away"
    );
    let evs = read_events(t.path());
    assert!(
        evs.iter()
            .any(|e| e["kind"] == "gate" && e["state"] != "running"),
        "folded gate terminal line"
    );
}

#[test]
fn opencode_metadata_exit_code() {
    let t = TempDir::new("ade-meta-exit");
    let payload = json!({
        "session_id": "s-oc",
        "tool_name": "bash",
        "tool_input": {"command": "false"},
        "tool_response": {
            "output": "boom",
            "metadata": {"exit": 2, "truncated": false}
        }
    });
    assert_silent_success(
        &feed_payload("posttooluse", &in_cwd(&payload, t.path()), t.path()),
        "metadata exit",
    );
    let evs = read_events(t.path());
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0]["kind"], "tool");
    assert_eq!(evs[0]["exit"], 2);
}

#[test]
fn opencode_kit_uses_session_status_not_idle() {
    let src = fs::read_to_string(Path::new(MANIFEST).join("kits/opencode/plugins/agentdash.js"))
        .expect("kit plugin");
    assert!(
        src.contains("\"session.status\""),
        "kit must hook session.status"
    );
    assert!(
        !src.contains("\"session.idle\""),
        "deprecated session.idle must not remain as a hook key"
    );
}

// ---------------------------------------------------------------- Antigravity (--host agy, W14-01)

/// 成功回放且校验 stdout 原文(Antigravity 要求每条出口返回单行 JSON)。
fn assert_agy_stdout(out: &Output, expected_stdout: &str, context: &str) {
    assert_silent_success(out, context);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        expected_stdout,
        "{context}: agy stdout JSON 不符"
    );
}

#[test]
fn agy_stdout_json_protocol_and_other_hosts_silent() {
    let t = TempDir::new("agy-stdout");
    let valid_obj = json!({
        "conversationId": "conv-stdout",
        "workspacePaths": [t.path().to_string_lossy()],
    })
    .to_string();

    // --host agy / --host Antigravity(大小写不敏感)在正常、空、损坏、非对象载荷下均输出合法单行 JSON
    for host_arg in ["agy", "Antigravity"] {
        for raw in [valid_obj.as_str(), "", "{{{corrupt", "[\"array\"]"] {
            assert_agy_stdout(
                &feed(&["hook", "--host", host_arg, "pretooluse"], raw, t.path()),
                "{\"decision\":\"allow\"}\n",
                "agy pretooluse stdout",
            );
            assert_agy_stdout(
                &feed(&["hook", "--host", host_arg, "posttooluse"], raw, t.path()),
                "{}\n",
                "agy posttooluse stdout",
            );
            assert_agy_stdout(
                &feed(
                    &["hook", "--host", host_arg, "posttoolusefailure"],
                    raw,
                    t.path(),
                ),
                "{}\n",
                "agy posttoolusefailure stdout",
            );
            assert_agy_stdout(
                &feed(&["hook", "--host", host_arg, "stop"], raw, t.path()),
                "{\"decision\":\"stop\"}\n",
                "agy stop stdout",
            );
            assert_agy_stdout(
                &feed(&["hook", "--host", host_arg, "subagentstop"], raw, t.path()),
                "{\"decision\":\"stop\"}\n",
                "agy subagentstop stdout",
            );
            assert_agy_stdout(
                &feed(&["hook", "--host", host_arg, "unknown-ev"], raw, t.path()),
                "{}\n",
                "agy unknown event stdout",
            );
        }
    }

    // 非 Antigravity 宿主必须继续保持 stdout 零字节
    for args in [
        &["hook", "pretooluse"][..],
        &["hook", "--host", "claude", "pretooluse"][..],
        &["hook", "--host", "claude", "posttooluse"][..],
        &["hook", "--host", "claude", "stop"][..],
        &["hook", "--host", "cursor", "stop"][..],
        &["hook", "--host", "opencode", "posttooluse"][..],
    ] {
        let out = feed(args, &valid_obj, t.path());
        assert_silent_success(&out, "non-agy host");
        assert!(
            out.stdout.is_empty(),
            "non-agy args={args:?} must emit 0 stdout bytes, got {:?}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

#[test]
fn agy_workspace_paths_prefers_root_with_agentdash() {
    let proc_cwd = TempDir::new("agy-ws-proc");
    let ws_plain = TempDir::new("agy-ws-plain");
    let ws_tracked = TempDir::new("agy-ws-tracked");
    fs::create_dir_all(ws_tracked.path().join(".agentdash")).expect("mkdir .agentdash");

    let payload = json!({
        "conversationId": "conv-ws",
        "workspacePaths": [
            ws_plain.path().to_string_lossy(),
            ws_tracked.path().to_string_lossy()
        ],
        "toolCall": {
            "name": "read_file",
            "args": {"Path": "src/hook.rs"}
        }
    });
    let out = feed(
        &["hook", "--host", "agy", "posttooluse"],
        &payload.to_string(),
        proc_cwd.path(),
    );
    assert_agy_stdout(&out, "{}\n", "workspacePaths multi-root");
    assert!(
        !events_path(ws_plain.path()).exists(),
        "无 .agentdash 的首根不应抢走事件"
    );
    assert!(
        !events_path(proc_cwd.path()).exists(),
        "workspacePaths 在场时不应落回进程 cwd"
    );
    let evs = read_events(ws_tracked.path());
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0]["summary"], "src/hook.rs");

    // 均无 .agentdash 时退取首个非空 workspacePaths
    let ws_first = TempDir::new("agy-ws-first");
    let ws_second = TempDir::new("agy-ws-second");
    let payload_fresh = json!({
        "conversationId": "conv-ws-2",
        "workspacePaths": [
            "   ",
            ws_first.path().to_string_lossy(),
            ws_second.path().to_string_lossy()
        ],
        "toolCall": {
            "name": "read_file",
            "args": {"Path": "README.md"}
        }
    });
    let out2 = feed(
        &["hook", "--host", "agy", "posttooluse"],
        &payload_fresh.to_string(),
        proc_cwd.path(),
    );
    assert_agy_stdout(&out2, "{}\n", "workspacePaths fresh fallback");
    let evs_first = read_events(ws_first.path());
    assert_eq!(evs_first.len(), 1);
    assert_eq!(evs_first[0]["summary"], "README.md");
}

#[test]
fn agy_run_command_gate_reads_transcript_passed() {
    // 按 stepIdx 精确命中 transcript.jsonl(忽略前序失败步)
    let repo_pass = TempDir::new("agy-gate-pass");
    let transcript_pass = repo_pass.path().join("transcript.jsonl");
    let lines_pass = [
        json!({
            "step_index": 3,
            "type": "GENERIC",
            "content": "Command exited with code 1.\n\nCommand output:\nearlier failure\n"
        })
        .to_string(),
        json!({
            "step_index": 4,
            "type": "GENERIC",
            "content": "Created At: 2026-10-28T11:00:00Z\nCommand exited with code 0.\n\nCommand output:\nrunning 12 tests\ntest result: ok. 12 passed; 0 failed\n"
        })
        .to_string(),
    ];
    fs::write(&transcript_pass, format!("{}\n", lines_pass.join("\n")))
        .expect("write transcript pass");

    let post_pass = json!({
        "conversationId": "conv-pass",
        "workspacePaths": [repo_pass.path().to_string_lossy()],
        "transcriptPath": transcript_pass.to_string_lossy(),
        "stepIdx": 4,
        "toolCall": {
            "name": "run_command",
            "args": {"CommandLine": "cargo test --all"}
        }
    });
    let stop_pass = json!({
        "conversationId": "conv-pass",
        "workspacePaths": [repo_pass.path().to_string_lossy()],
        "fullyIdle": true
    });
    assert_agy_stdout(
        &feed(
            &["hook", "--host", "agy", "posttooluse"],
            &post_pass.to_string(),
            repo_pass.path(),
        ),
        "{}\n",
        "agy gate pass post",
    );
    assert_agy_stdout(
        &feed(
            &["hook", "--host", "agy", "stop"],
            &stop_pass.to_string(),
            repo_pass.path(),
        ),
        "{\"decision\":\"stop\"}\n",
        "agy gate pass stop",
    );
    let evs = read_events(repo_pass.path());
    assert_eq!(evs.len(), 2);
    assert_eq!(evs[0]["gate"], "cargo-test");
    assert_eq!(evs[0]["state"], "running");
    assert_eq!(evs[0]["session_id"], "conv-pass");
    assert_eq!(evs[1]["state"], "passed");
    assert_eq!(evs[1]["exit"], 0);
    assert_eq!(evs[1]["detail"], "test result: ok. 12 passed; 0 failed");
    assert_eq!(evs[1]["session_id"], "conv-pass");
    assert_eq!(evs[1]["host"], "agy");
}

#[test]
fn agy_run_command_gate_reads_transcript_failed_bounded_tail() {
    // failed(101) + 超过 64KB 前缀的大 transcript 有界尾部扫描
    let repo_fail = TempDir::new("agy-gate-fail");
    let transcript_fail = repo_fail.path().join("transcript_big.jsonl");
    let mut big = "x".repeat(80 * 1024);
    big.push('\n');
    big.push_str(
        &json!({
            "step_index": 9,
            "type": "GENERIC",
            "content": "Command exited with code 101.\n\nCommand output:\ntest agy ... FAILED\nerror: test failed, to rerun pass `--test hook`\n"
        })
        .to_string(),
    );
    big.push('\n');
    fs::write(&transcript_fail, big).expect("write big transcript");

    let post_fail = json!({
        "conversationId": "conv-fail",
        "workspacePaths": [repo_fail.path().to_string_lossy()],
        "transcriptPath": transcript_fail.to_string_lossy(),
        "stepIdx": 9,
        "toolCall": {
            "name": "run_command",
            "args": {"CommandLine": "cargo test --test hook"}
        }
    });
    let stop_fail = json!({
        "conversationId": "conv-fail",
        "workspacePaths": [repo_fail.path().to_string_lossy()],
        "fullyIdle": true
    });
    assert_agy_stdout(
        &feed(
            &["hook", "--host", "agy", "posttooluse"],
            &post_fail.to_string(),
            repo_fail.path(),
        ),
        "{}\n",
        "agy gate fail post",
    );
    assert_agy_stdout(
        &feed(
            &["hook", "--host", "agy", "stop"],
            &stop_fail.to_string(),
            repo_fail.path(),
        ),
        "{\"decision\":\"stop\"}\n",
        "agy gate fail stop",
    );
    let evs_fail = read_events(repo_fail.path());
    assert_eq!(evs_fail.len(), 2);
    assert_eq!(evs_fail[1]["state"], "failed");
    assert_eq!(evs_fail[1]["exit"], 101);
    assert_eq!(
        evs_fail[1]["detail"],
        "error: test failed, to rerun pass `--test hook`"
    );
}

#[test]
fn agy_run_command_gate_deferred_transcript_rechecked_on_stop() {
    let repo_defer = TempDir::new("agy-gate-defer");
    let transcript_defer = repo_defer.path().join("transcript_defer.jsonl");
    let step11 = json!({
        "step_index": 11,
        "type": "GENERIC",
        "content": "Command exited with code 1.\n\nCommand output:\nold step\n"
    });
    fs::write(&transcript_defer, format!("{step11}\n")).expect("write step 11");

    let post_defer = json!({
        "conversationId": "conv-defer",
        "workspacePaths": [repo_defer.path().to_string_lossy()],
        "transcriptPath": transcript_defer.to_string_lossy(),
        "stepIdx": 12,
        "toolCall": {
            "name": "run_command",
            "args": {"CommandLine": "cargo test"}
        }
    });
    assert_agy_stdout(
        &feed(
            &["hook", "--host", "agy", "posttooluse"],
            &post_defer.to_string(),
            repo_defer.path(),
        ),
        "{}\n",
        "agy deferred post",
    );
    let slots = read_pending_slots(repo_defer.path());
    assert_eq!(slots.len(), 1);
    assert!(
        slots[0]["exit"].is_null(),
        "未落盘前不得误借旧步退出码或伪造 0"
    );
    assert_eq!(slots[0]["step_idx"], 12);
    assert_eq!(
        slots[0]["transcript"],
        transcript_defer.to_string_lossy().as_ref()
    );

    // Stop 之前 Antigravity 将 step 12 刷入 transcript.jsonl
    let step12 = json!({
        "step_index": 12,
        "type": "GENERIC",
        "content": "Command exited with code 0.\n\nCommand output:\ntest result: ok. 42 passed\n"
    });
    fs::write(&transcript_defer, format!("{step11}\n{step12}\n")).expect("flush step 12");

    let stop_defer = json!({
        "conversationId": "conv-defer",
        "workspacePaths": [repo_defer.path().to_string_lossy()],
        "fullyIdle": true
    });
    assert_agy_stdout(
        &feed(
            &["hook", "--host", "agy", "stop"],
            &stop_defer.to_string(),
            repo_defer.path(),
        ),
        "{\"decision\":\"stop\"}\n",
        "agy deferred stop",
    );
    let evs_defer = read_events(repo_defer.path());
    assert_eq!(evs_defer.len(), 2);
    assert_eq!(evs_defer[1]["state"], "passed");
    assert_eq!(evs_defer[1]["exit"], 0);
    assert_eq!(evs_defer[1]["detail"], "test result: ok. 42 passed");
}

#[test]
fn agy_run_command_without_transcript_or_error_stays_unknown_never_zero() {
    let repo_unk = TempDir::new("agy-unknown");
    let post = json!({
        "conversationId": "conv-unk",
        "workspacePaths": [repo_unk.path().to_string_lossy()],
        "transcriptPath": repo_unk.path().join("missing.jsonl").to_string_lossy(),
        "stepIdx": 5,
        "toolCall": {
            "name": "run_command",
            "args": {"CommandLine": "cargo clippy -- -D warnings"}
        }
    });
    let stop = json!({
        "conversationId": "conv-unk",
        "workspacePaths": [repo_unk.path().to_string_lossy()],
        "fullyIdle": true
    });
    assert_agy_stdout(
        &feed(
            &["hook", "--host", "agy", "posttooluse"],
            &post.to_string(),
            repo_unk.path(),
        ),
        "{}\n",
        "agy unknown post",
    );
    assert_agy_stdout(
        &feed(
            &["hook", "--host", "agy", "stop"],
            &stop.to_string(),
            repo_unk.path(),
        ),
        "{\"decision\":\"stop\"}\n",
        "agy unknown stop",
    );
    let evs = read_events(repo_unk.path());
    assert_eq!(evs.len(), 2);
    assert_eq!(evs[1]["gate"], "cargo-clippy");
    assert_eq!(evs[1]["state"], "failed", "缺失证据必须记 failed(unknown)");
    assert!(evs[1]["exit"].is_null(), "绝不伪造 exit 0: {}", evs[1]);
    assert_eq!(evs[1]["detail"], "(exit unknown)");
}

#[test]
fn agy_run_command_error_priority_and_non_gate_bash_tool() {
    // 顶层 error 字符串优先级最高:直接提取退出码或回退 1
    let repo_err = TempDir::new("agy-cmd-err");
    let post_err = json!({
        "conversationId": "conv-err",
        "workspacePaths": [repo_err.path().to_string_lossy()],
        "error": "Command exited with code 2.\ncompilation aborted",
        "toolCall": {
            "name": "run_command",
            "args": {"CommandLine": "cargo fmt --check"}
        }
    });
    let stop_err = json!({
        "conversationId": "conv-err",
        "workspacePaths": [repo_err.path().to_string_lossy()]
    });
    assert_agy_stdout(
        &feed(
            &["hook", "--host", "agy", "posttooluse"],
            &post_err.to_string(),
            repo_err.path(),
        ),
        "{}\n",
        "agy error post",
    );
    assert_agy_stdout(
        &feed(
            &["hook", "--host", "agy", "stop"],
            &stop_err.to_string(),
            repo_err.path(),
        ),
        "{\"decision\":\"stop\"}\n",
        "agy error stop",
    );
    let evs_err = read_events(repo_err.path());
    assert_eq!(evs_err[1]["state"], "failed");
    assert_eq!(evs_err[1]["exit"], 2);
    assert_eq!(evs_err[1]["detail"], "Command exited with code 2.");

    // 非 gate 的 run_command 归一为 bash tool 行且带真实退出码
    let repo_bash = TempDir::new("agy-cmd-bash");
    let transcript_bash = repo_bash.path().join("transcript.jsonl");
    let step1 = json!({
        "step_index": 1,
        "type": "GENERIC",
        "content": "Command exited with code 0.\n\nCommand output:\nOn branch main\n"
    });
    fs::write(&transcript_bash, format!("{step1}\n")).expect("write transcript");
    let post_bash = json!({
        "conversationId": "conv-bash",
        "workspacePaths": [repo_bash.path().to_string_lossy()],
        "transcriptPath": transcript_bash.to_string_lossy(),
        "toolCall": {
            "name": "run_command",
            "args": {"CommandLine": "git status"}
        }
    });
    assert_agy_stdout(
        &feed(
            &["hook", "--host", "agy", "posttooluse"],
            &post_bash.to_string(),
            repo_bash.path(),
        ),
        "{}\n",
        "agy non-gate run_command",
    );
    let evs_bash = read_events(repo_bash.path());
    assert_eq!(evs_bash.len(), 1);
    assert_eq!(evs_bash[0]["kind"], "tool");
    assert_eq!(evs_bash[0]["tool"], "bash");
    assert_eq!(evs_bash[0]["exit"], 0);
    assert_eq!(evs_bash[0]["summary"], "git status");
    assert_eq!(evs_bash[0]["session_id"], "conv-bash");
}

#[test]
fn agy_non_command_toolcall_emits_tool_with_conversation_id() {
    let repo = TempDir::new("agy-toolcall");
    let read_ok = json!({
        "conversationId": "conv-tool-42",
        "workspacePaths": [repo.path().to_string_lossy()],
        "toolCall": {
            "name": "read_file",
            "args": {"Path": "src/hook.rs", "StartLine": 1}
        }
    });
    let grep_err = json!({
        "conversationId": "conv-tool-42",
        "workspacePaths": [repo.path().to_string_lossy()],
        "error": "path does not exist",
        "toolCall": {
            "name": "grep_search",
            "args": {"Query": "agy_transcript_outcome"}
        }
    });
    assert_agy_stdout(
        &feed(
            &["hook", "--host", "agy", "posttooluse"],
            &read_ok.to_string(),
            repo.path(),
        ),
        "{}\n",
        "agy read_file",
    );
    assert_agy_stdout(
        &feed(
            &["hook", "--host", "agy", "posttooluse"],
            &grep_err.to_string(),
            repo.path(),
        ),
        "{}\n",
        "agy grep_search error",
    );
    let evs = read_events(repo.path());
    assert_eq!(evs.len(), 2);
    assert_eq!(evs[0]["kind"], "tool");
    assert_eq!(evs[0]["tool"], "read_file");
    assert_eq!(evs[0]["phase"], "end");
    assert!(evs[0]["exit"].is_null());
    assert_eq!(evs[0]["summary"], "src/hook.rs");
    assert_eq!(evs[0]["session_id"], "conv-tool-42");
    assert_eq!(evs[0]["host"], "agy");

    assert_eq!(evs[1]["tool"], "grep_search");
    assert_eq!(evs[1]["exit"], 1, "顶层 error 映射 exit=1");
    assert_eq!(evs[1]["summary"], "agy_transcript_outcome");
    assert_eq!(evs[1]["session_id"], "conv-tool-42");
}

#[test]
fn agy_subagent_dispatch_and_child_stop_completion_lifecycle() {
    let repo = TempDir::new("agy-subagent");
    let ws = [repo.path().to_string_lossy()];
    let pre_read = json!({
        "conversationId": "parent-1",
        "workspacePaths": ws,
        "toolCall": {"name": "read_file", "args": {"Path": "Cargo.toml"}}
    });
    assert_agy_stdout(
        &feed(
            &["hook", "--host", "agy", "pretooluse"],
            &pre_read.to_string(),
            repo.path(),
        ),
        "{\"decision\":\"allow\"}\n",
        "agy pre read_file",
    );
    assert!(!events_path(repo.path()).exists());

    let pre_dispatch = json!({
        "conversationId": "parent-1",
        "workspacePaths": ws,
        "toolCall": {
            "name": "invoke_subagent",
            "args": {
                "TypeName": "self",
                "Role": "Hook Adapter Dev",
                "Prompt": "Implement Antigravity hook adapter"
            }
        }
    });
    assert_agy_stdout(
        &feed(
            &["hook", "--host", "agy", "pretooluse"],
            &pre_dispatch.to_string(),
            repo.path(),
        ),
        "{\"decision\":\"allow\"}\n",
        "agy pre invoke_subagent",
    );
    let evs = read_events(repo.path());
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0]["event"], "dispatched");
    assert_eq!(evs[0]["who"], "Hook Adapter Dev");
    assert_eq!(evs[0]["task"], "Implement Antigravity hook adapter");
    assert_eq!(evs[0]["session_id"], "parent-1");

    // 父会话自身 Stop(is_dispatcher=true)及子会话 fullyIdle=false 不得发 completed
    for stop_early in [
        json!({"conversationId": "parent-1", "workspacePaths": ws, "fullyIdle": true}),
        json!({"conversationId": "child-1", "workspacePaths": ws, "fullyIdle": false}),
    ] {
        assert_agy_stdout(
            &feed(
                &["hook", "--host", "agy", "stop"],
                &stop_early.to_string(),
                repo.path(),
            ),
            "{\"decision\":\"stop\"}\n",
            "early stop",
        );
    }
    assert_eq!(read_events(repo.path()).len(), 1);

    // 子会话 fullyIdle=true 追加匿名 completed;重复 Stop 与无在途派发会话 Stop 均幂等不增行
    let child_stop = json!({"conversationId": "child-1", "workspacePaths": ws, "fullyIdle": true});
    let other_stop = json!({"conversationId": "other-2", "workspacePaths": ws, "fullyIdle": true});
    for payload in [&child_stop, &child_stop, &other_stop] {
        assert_agy_stdout(
            &feed(
                &["hook", "--host", "agy", "stop"],
                &payload.to_string(),
                repo.path(),
            ),
            "{\"decision\":\"stop\"}\n",
            "child/duplicate/unrelated stop",
        );
    }
    let evs = read_events(repo.path());
    assert_eq!(evs.len(), 2, "恰一条 dispatched + 一条幂等匿名 completed");
    assert_eq!(evs[1]["kind"], "agent");
    assert_eq!(evs[1]["event"], "completed");
    assert!(evs[1].get("who").is_none());
    assert_eq!(evs[1]["session_id"], "child-1");
    assert_eq!(evs[1]["host"], "agy");
}

#[test]
fn agy_step_none_session_isolation_and_subagent_who_fallbacks() {
    let repo = TempDir::new("agy-edges");
    let transcript = repo.path().join("transcript.jsonl");
    let lines = [
        json!({
            "step_index": 2,
            "type": "GENERIC",
            "content": "Command exited with code 137.\n\nCommand output:\n   \n"
        })
        .to_string(),
        json!({
            "step_index": 3,
            "type": "GENERIC",
            "content": "Showing lines 1-10 of 10\n1: fn main() {}"
        })
        .to_string(),
    ];
    fs::write(&transcript, format!("{}\n", lines.join("\n"))).expect("write transcript");

    let ws = [repo.path().to_string_lossy()];
    let post_a = json!({
        "conversationId": "conv-A",
        "workspacePaths": ws,
        "transcriptPath": transcript.to_string_lossy(),
        "toolCall": {"name": "run_command", "args": {"command": "cargo test"}}
    });
    let post_b = json!({
        "conversationId": "conv-B",
        "workspacePaths": ws,
        "tool_response": {"status": 0, "stdout": "all good\n"},
        "toolCall": {"name": "run_command", "args": {"CommandLine": "go test ./..."}}
    });
    for post in [&post_a, &post_b] {
        assert_agy_stdout(
            &feed(
                &["hook", "--host", "agy", "posttooluse"],
                &post.to_string(),
                repo.path(),
            ),
            "{}\n",
            "concurrent post",
        );
    }

    assert_agy_stdout(
        &feed(
            &["hook", "--host", "agy", "stop"],
            &json!({"conversationId": "conv-A", "workspacePaths": ws}).to_string(),
            repo.path(),
        ),
        "{\"decision\":\"stop\"}\n",
        "conv-A stop",
    );
    assert!(pending_path(repo.path()).exists(), "conv-B 槽仍保留");
    assert_agy_stdout(
        &feed(
            &["hook", "--host", "agy", "stop"],
            &json!({"conversationId": "conv-B", "workspacePaths": ws}).to_string(),
            repo.path(),
        ),
        "{\"decision\":\"stop\"}\n",
        "conv-B stop",
    );

    let cases = [
        (json!({"TypeName": "research"}), "research"),
        (json!({}), "agent"),
    ];
    for (args, _) in &cases {
        let pre = json!({
            "conversationId": "conv-A",
            "workspacePaths": ws,
            "toolCall": {"name": "invoke_subagent", "args": args}
        });
        assert_agy_stdout(
            &feed(
                &["hook", "--host", "agy", "pretooluse"],
                &pre.to_string(),
                repo.path(),
            ),
            "{\"decision\":\"allow\"}\n",
            "invoke_subagent fallback",
        );
    }

    let evs = read_events(repo.path());
    assert_eq!(evs.len(), 6);
    assert_eq!(evs[2]["state"], "failed");
    assert_eq!(evs[2]["exit"], 137);
    assert_eq!(evs[2]["detail"], "exit 137");
    assert_eq!(evs[3]["state"], "passed");
    assert_eq!(evs[3]["exit"], 0);
    assert_eq!(evs[4]["who"], cases[0].1);
    assert_eq!(evs[5]["who"], cases[1].1);
}
