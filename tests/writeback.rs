//! W2-006 写回集成测试:`apply` 的成功落位(含未知字段/未触及任务保留)、
//! 幂等命中(已是目标 → Err 且文件原样)、无台账/无任务/损坏台账的失败面、
//! 原子性(写后 JSON 合法且无 tmp 残留)、锁内并发(两线程改不同任务双双
//! 存活,证明读-改-写整体在临界区内,无丢失更新),以及 tui 键位接线纯映射
//! (`d`/`b`/`m`、备注行编辑态、help 键位同步)。
//!
//! agentdash 是纯二进制 crate,集成测试按 `#[path]` 挂载模块树(同
//! `tests/tui.rs` 约定);挂载源中本测试未触达的 pub 项按文件级 allow 放行。

#![allow(dead_code)]

#[path = "../src/contract.rs"]
mod contract;
#[path = "../src/events.rs"]
mod events;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/render/mod.rs"]
mod render;
#[path = "../src/sources/mod.rs"]
mod sources;
#[path = "../src/tui.rs"]
mod tui;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;

use tui::{Action as KeyAction, InputMode};
use tui::writeback::Action as WriteAction;

// ---------------------------------------------------------------- helpers

/// 自清理临时目录(测试工作仓:`.agentdash/ledger.json` 落这里)。
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("agentdash-writeback-{tag}-{pid}-{n}"));
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

/// 样例台账:T2 active 带备注(验证改 state 时 note 保留),T1 已 done
/// (验证幂等命中),外加未知顶层字段(验证写回不丢字段)。
const LEDGER: &str = r#"{
  "$schema": "agentdash.tasklog.v1",
  "wave": "W2",
  "title": "W2 交互跃迁",
  "lanes": [ { "name": "A-impl", "tasks": ["T1", "T2"] } ],
  "tasks": {
    "T1": { "label": "任务一", "state": "done" },
    "T2": { "label": "任务二", "state": "active", "note": "fix round 1/3" }
  },
  "barriers": [],
  "extra_unknown": { "keep": true }
}
"#;

/// 临时仓 + 预置 `.agentdash/ledger.json`。
fn repo_with_ledger(tag: &str) -> TempDir {
    let t = TempDir::new(tag);
    let dir = t.path().join(".agentdash");
    fs::create_dir_all(&dir).expect("create .agentdash");
    fs::write(dir.join("ledger.json"), LEDGER).expect("write ledger");
    t
}

/// 写回后的台账(此处解析失败即原子性破坏,直接炸测试)。
fn ledger_value(t: &TempDir) -> Value {
    let text =
        fs::read_to_string(t.path().join(".agentdash").join("ledger.json")).expect("read ledger");
    serde_json::from_str(&text).expect("ledger.json 必须始终是合法 JSON")
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

// ---------------------------------------------------------------- apply

#[test]
fn mark_done_writes_state_and_preserves_rest() {
    let t = repo_with_ledger("done");
    let msg = tui::writeback::apply(t.path(), "T2", WriteAction::MarkDone).expect("apply ok");
    assert_eq!(msg, "已写回 T2:done");
    let ledger = ledger_value(&t);
    assert_eq!(ledger["tasks"]["T2"]["state"], "done");
    assert_eq!(
        ledger["tasks"]["T2"]["note"], "fix round 1/3",
        "改 state 不动 note"
    );
    assert_eq!(ledger["tasks"]["T1"]["state"], "done", "未触及任务原样");
    assert_eq!(ledger["title"], "W2 交互跃迁", "标题原样");
    assert_eq!(ledger["$schema"], "agentdash.tasklog.v1", "$schema 原样");
    assert_eq!(
        ledger["extra_unknown"]["keep"], true,
        "未知顶层字段写回后保留"
    );
}

#[test]
fn mark_blocked_writes_state() {
    let t = repo_with_ledger("blocked");
    let msg = tui::writeback::apply(t.path(), "T2", WriteAction::MarkBlocked).expect("apply ok");
    assert_eq!(msg, "已写回 T2:blocked");
    assert_eq!(ledger_value(&t)["tasks"]["T2"]["state"], "blocked");
}

#[test]
fn set_note_writes_note_including_cjk() {
    let t = repo_with_ledger("note");
    let msg = tui::writeback::apply(
        t.path(),
        "T2",
        WriteAction::SetNote(String::from("等 gate 放行")),
    )
    .expect("apply ok");
    assert_eq!(msg, "已写回 T2:note");
    let ledger = ledger_value(&t);
    assert_eq!(ledger["tasks"]["T2"]["note"], "等 gate 放行");
    assert_eq!(
        ledger["tasks"]["T2"]["state"], "active",
        "改 note 不动 state"
    );
}

#[test]
fn set_note_empty_clears_note() {
    let t = repo_with_ledger("clear");
    tui::writeback::apply(t.path(), "T2", WriteAction::SetNote(String::new())).expect("apply ok");
    assert!(
        ledger_value(&t)["tasks"]["T2"].get("note").is_none(),
        "空备注 = 清除 note 键"
    );
}

#[test]
fn apply_without_ledger_is_err() {
    let t = TempDir::new("nol");
    let err = tui::writeback::apply(t.path(), "T2", WriteAction::MarkDone).unwrap_err();
    assert!(err.contains("无台账"), "err={err}");
}

#[test]
fn apply_unknown_task_is_err_and_file_untouched() {
    let t = repo_with_ledger("notask");
    let before = fs::read_to_string(t.path().join(".agentdash").join("ledger.json")).unwrap();
    let err = tui::writeback::apply(t.path(), "T9", WriteAction::MarkDone).unwrap_err();
    assert!(err.contains("T9"), "错误消息可显示任务 id:err={err}");
    let after = fs::read_to_string(t.path().join(".agentdash").join("ledger.json")).unwrap();
    assert_eq!(before, after, "失败写回不得动文件");
}

#[test]
fn apply_idempotent_done_is_err_and_file_untouched() {
    let t = repo_with_ledger("idem");
    let before = fs::read_to_string(t.path().join(".agentdash").join("ledger.json")).unwrap();
    let err = tui::writeback::apply(t.path(), "T1", WriteAction::MarkDone).unwrap_err();
    assert!(err.contains("已是 done"), "幂等命中给原因:err={err}");
    let after = fs::read_to_string(t.path().join(".agentdash").join("ledger.json")).unwrap();
    assert_eq!(before, after, "幂等命中不得重写文件");
}

#[test]
fn set_note_unchanged_is_err() {
    let t = repo_with_ledger("noteidem");
    let err = tui::writeback::apply(
        t.path(),
        "T2",
        WriteAction::SetNote(String::from("fix round 1/3")),
    )
    .unwrap_err();
    assert!(err.contains("备注未变化"), "err={err}");
}

#[test]
fn apply_corrupt_ledger_is_err() {
    let t = TempDir::new("corrupt");
    let dir = t.path().join(".agentdash");
    fs::create_dir_all(&dir).expect("create .agentdash");
    fs::write(dir.join("ledger.json"), "NOT JSON {").expect("write corrupt");
    let err = tui::writeback::apply(t.path(), "T2", WriteAction::MarkDone).unwrap_err();
    assert!(err.contains("损坏"), "err={err}");
}

// ---------------------------------------------------------------- 原子性 / 锁

#[test]
fn apply_is_atomic_no_tmp_residue_and_valid_json() {
    let t = repo_with_ledger("atomic");
    tui::writeback::apply(t.path(), "T2", WriteAction::MarkDone).expect("apply ok");
    let entries: Vec<String> = fs::read_dir(t.path().join(".agentdash"))
        .expect("read dir")
        .map(|entry| {
            entry
                .expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let residue: Vec<&String> = entries
        .iter()
        .filter(|name| std::path::Path::new(name).extension().is_some_and(|e| e.eq_ignore_ascii_case("tmp")))
        .collect();
    assert!(residue.is_empty(), "tmp 残留:{residue:?}");
    // ledger_value 内部断言 JSON 合法;此处再验落位
    assert_eq!(ledger_value(&t)["tasks"]["T2"]["state"], "done");
}

#[test]
fn concurrent_writes_both_survive() {
    let t = repo_with_ledger("lock");
    let repo_a = t.path().to_path_buf();
    let repo_b = t.path().to_path_buf();
    let a = std::thread::spawn(move || tui::writeback::apply(&repo_a, "T2", WriteAction::MarkDone));
    let b = std::thread::spawn(move || {
        tui::writeback::apply(
            &repo_b,
            "T1",
            WriteAction::SetNote(String::from("并发备注")),
        )
    });
    a.join().expect("thread a").expect("a ok");
    b.join().expect("thread b").expect("b ok");
    let ledger = ledger_value(&t);
    assert_eq!(ledger["tasks"]["T2"]["state"], "done", "T2 改动存活");
    assert_eq!(
        ledger["tasks"]["T1"]["note"], "并发备注",
        "T1 改动存活:读-改-写在锁内,无丢失更新"
    );
}

// ---------------------------------------------------------------- tui 键位接线

#[test]
fn writeback_key_map() {
    let none = KeyModifiers::NONE;
    assert_eq!(
        tui::key_action(InputMode::Normal, key(KeyCode::Char('d'), none)),
        KeyAction::MarkDone,
        "d 归写回(W2-006 裁定:详情改 ⏎/Esc 独占)"
    );
    assert_eq!(
        tui::key_action(InputMode::Normal, key(KeyCode::Char('b'), none)),
        KeyAction::MarkBlocked
    );
    assert_eq!(
        tui::key_action(InputMode::Normal, key(KeyCode::Char('m'), none)),
        KeyAction::StartNote
    );
    assert_eq!(
        tui::key_action(InputMode::Normal, key(KeyCode::Enter, none)),
        KeyAction::EnterDetail,
        "⏎ 仍是详情独占入口"
    );
    assert_eq!(
        tui::key_action(InputMode::Normal, key(KeyCode::Esc, none)),
        KeyAction::Back,
        "Esc 返回(详情态关闭)不变"
    );
}

#[test]
fn note_mode_line_edit_semantics() {
    let none = KeyModifiers::NONE;
    assert_eq!(
        tui::key_action(InputMode::Note, key(KeyCode::Char('备'), none)),
        KeyAction::Input('备'),
        "备注态按字符输入(CJK 安全)"
    );
    assert_eq!(
        tui::key_action(InputMode::Note, key(KeyCode::Backspace, none)),
        KeyAction::Erase
    );
    assert_eq!(
        tui::key_action(InputMode::Note, key(KeyCode::Enter, none)),
        KeyAction::Submit,
        "⏎ 提交写回"
    );
    assert_eq!(
        tui::key_action(InputMode::Note, key(KeyCode::Esc, none)),
        KeyAction::Cancel
    );
    assert_eq!(
        tui::key_action(
            InputMode::Note,
            key(KeyCode::Char('c'), KeyModifiers::CONTROL)
        ),
        KeyAction::Quit,
        "备注态 Ctrl-C 仍退出"
    );
}

#[test]
fn help_lists_writeback_keys() {
    let help = tui::help_lines().join("\n");
    assert!(
        help.contains("d      聚焦任务标记 done"),
        "help 缺 d 写回:{help}"
    );
    assert!(
        help.contains("b      聚焦任务标记 blocked"),
        "help 缺 b 写回"
    );
    assert!(help.contains("m      聚焦任务写备注"), "help 缺 m 备注");
    assert!(
        help.contains("⏎      打开聚焦任务详情"),
        "详情入口标注仍在(⏎ 独占)"
    );
}
