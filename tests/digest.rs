//! W10-002 离场摘要集成测试:`render digest [--strict] [PATH]`。
//!
//! agentdash 是纯二进制 crate,集成测试按 `#[path]` 在 crate 根挂载模块树
//! (与 `tests/render_panel.rs` 同约定)。渲染面手工构造 `Dashboard` 精确钉
//! 黄金样;退出码面(--strict 两态 / 多 PATH 用法错)子进程回放真二进制,
//! fixture 走纯临时目录 + 台账(非 git 仓即可,merge 链路无 git 依赖)。

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

use contract::TaskState;
use model::{AgentView, Dashboard, GateView, MilestoneView, TaskView};
use render::{digest_needs_attention, render_digest};
use sources::git::GitFacts;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

/// 被测二进制(cargo 注入的绝对路径,bin-only crate 走子进程回放)。
const EXE: &str = env!("CARGO_BIN_EXE_agentdash");

// ---------------------------------------------------------------- 渲染面钉样

fn task(id: &str, label: &str, state: TaskState, done_at: Option<&str>) -> TaskView {
    TaskView {
        id: id.into(),
        label: label.into(),
        state,
        lane: None,
        note: None,
        fix_round: None,
        since: None,
        done_at: done_at.map(str::to_owned),
    }
}

fn agent(who: &str, task: Option<&str>, since: &str, host: Option<&str>) -> AgentView {
    AgentView {
        who: who.into(),
        task: task.map(str::to_owned),
        since: since.into(),
        host: host.map(str::to_owned),
    }
}

fn gate(name: &str, state: &str, detail: &str) -> GateView {
    GateView {
        name: name.into(),
        state: state.into(),
        detail: detail.into(),
    }
}

/// 钉固 git 事实:仅带 `root`(项目名来源),其余字段全空——手工构造的
/// `Dashboard` 不走 git 源,断言与测试 cwd 名解耦(承 `render_panel` 约定)。
fn pinned_git() -> GitFacts {
    GitFacts {
        root: Some("agentdash".to_owned()),
        ..GitFacts::absent()
    }
}

/// 全要素样例:活跃里程碑 + 5 任务(2 done / 1 active / 1 blocked / 1
/// pending)+ 2 在跑 agent + 3 门(passed/failed/running)+ 1 警告。
fn busy_dash() -> Dashboard {
    Dashboard {
        tasks: vec![
            task("T1", "fiber join 即回收", TaskState::Done, None),
            task("T2", "scope by_id 索引", TaskState::Active, None),
            task("T3", "orphan 上界", TaskState::Blocked, None),
            task(
                "T4",
                "集成收尾",
                TaskState::Done,
                Some("2026-09-13T09:00:00Z"),
            ),
            task("T5", "velocity 收口", TaskState::Pending, None),
        ],
        milestones: vec![MilestoneView {
            wave: Some("W25".into()),
            title: "并行车道冲刺".into(),
            done: 2,
            total: 5,
        }],
        warnings: vec!["ledger: 未知字段".to_owned()],
        agents: vec![
            agent(
                "alice",
                Some("冲 W10"),
                "2026-09-13T08:30:00Z",
                Some("codex"),
            ),
            agent("bob", None, "", None),
        ],
        gates: vec![
            gate("build", "passed", ""),
            gate("test", "failed", "2 failed"),
            gate("lint", "running", ""),
        ],
        barriers: Vec::new(),
        event_tail: Vec::new(),
        events_present: false,
        last_gate_passed: None,
        event_span_secs: None,
        git: pinned_git(),
        remote: None,
        project: "agentdash".into(),
        generated_at: "2026-09-13T08:30:00Z".into(),
    }
}

/// 空仓底座:单 pending 任务、无 agent/门/警告(零残留断言用)。
fn quiet_dash() -> Dashboard {
    Dashboard {
        tasks: vec![task("T1", "唯一任务", TaskState::Pending, None)],
        milestones: Vec::new(),
        warnings: Vec::new(),
        agents: Vec::new(),
        gates: Vec::new(),
        barriers: Vec::new(),
        event_tail: Vec::new(),
        events_present: false,
        last_gate_passed: None,
        event_span_secs: None,
        git: pinned_git(),
        remote: None,
        project: "agentdash".into(),
        generated_at: "2026-09-13T08:30:00Z".into(),
    }
}

/// W10-002 主钉:全要素摘要——节序 = 页眉(项目 · 活跃里程碑)→ 统计行 →
/// 失败门(✗ 名 · detail)→ 在跑 agent → blocked(⊘)→ done(✓,带
/// `done_at` 者附时刻切片)→ ⚠;**零 ANSI**;passed/running 门与非 done 非
/// blocked 任务零残留(缺项零残留,不打空标题/空态)。
#[test]
fn digest_is_plain_text_and_pins_all_sections_in_order() {
    let out = render_digest(&busy_dash());
    assert!(!out.contains('\x1b'), "离场摘要零 ANSI: {out:?}");
    let expected = [
        "agentdash · W25 并行车道冲刺",
        "✓2 ▶1 ·2 ⚑0 ⊘1 · 2 agents · 09-13T08:30",
        "✗ test · 2 failed",
        "▶ alice [codex] · 冲 W10 · 09-13T08:30",
        "▶ bob",
        "⊘ T3 orphan 上界",
        "✓ T1 fiber join 即回收",
        "✓ T4 集成收尾 · 09-13T09:00",
        "⚠ ledger: 未知字段",
    ];
    assert_eq!(out, expected.join("\n"), "全要素摘要逐行黄金(节序与格式)");
    let lines: Vec<&str> = out.lines().collect();
    let pos = |mark: &str| {
        lines
            .iter()
            .position(|line| line.starts_with(mark))
            .unwrap_or_else(|| panic!("节标记 {mark} 应上板: {out}"))
    };
    assert!(pos("✗ ") < pos("▶ "), "失败门节先于在跑 agent 节");
    assert!(pos("▶ ") < pos("⊘ "), "在跑 agent 节先于 blocked 节");
    assert!(pos("⊘ ") < pos("✓ "), "blocked 节先于 done 节");
    assert!(pos("✓ ") < pos("⚠ "), "done 节先于警告节");
    // 非本视图内容零残留:passed/running 门、active/pending 任务、分隔线
    assert!(!out.contains("build"), "passed 门不上摘要: {out}");
    assert!(!out.contains("lint"), "running 门不上摘要: {out}");
    assert!(!out.contains("T2"), "active 任务不上摘要: {out}");
    assert!(!out.contains("T5"), "pending 任务不上摘要: {out}");
    assert!(!out.contains('═') && !out.contains('─'), "无区块分隔线");
}

/// 缺项零残留:无失败门/在跑 agent/blocked/done/警告的仓,摘要只出页眉 +
/// 统计两行,不打任何空态占位或区块标题。
#[test]
fn digest_missing_sections_leave_zero_residue() {
    let out = render_digest(&quiet_dash());
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec!["agentdash", "✓0 ▶0 ·1 ⚑0 ⊘0 · 0 agents · 09-13T08:30",],
        "空仓仅页眉+统计两行: {out}"
    );
    assert!(!out.contains('\x1b'), "空态输出同样零 ANSI");
}

/// done 节细节:带 `done_at` 者附时刻切片(承 agent 行的 MM-DDTHH:MM 口径),
/// 无 `done_at` 者仅标签;物证怀疑 `?` 沿 [`render::unattested_done`] 语义
/// (自报晚于最近通过门 → `?`,有物证/无事件窗不打)。
#[test]
fn digest_done_section_carries_done_at_and_question_marker() {
    let mut dash = quiet_dash();
    dash.tasks = vec![
        task("TA", "甲", TaskState::Done, Some("2026-09-13T12:00:00Z")),
        task("TB", "乙", TaskState::Done, Some("2026-09-13T08:00:00Z")),
        task("TC", "丙", TaskState::Done, None),
    ];
    dash.events_present = true;
    dash.last_gate_passed = Some("2026-09-13T09:00:00Z".into());
    let out = render_digest(&dash);
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines.contains(&"✓ TA 甲 · 09-13T12:00 ?"),
        "自报晚于通过门 → done_at 切片 + ?: {out}"
    );
    assert!(
        lines.contains(&"✓ TB 乙 · 09-13T08:00"),
        "有物证:只附时刻,不打 ?"
    );
    assert!(!out.contains("TB 乙 · 09-13T08:00 ?"));
    assert!(
        lines.contains(&"✓ TC 丙"),
        "无 done_at 仍列入(仅标签),不打 ?"
    );
    assert!(!out.contains("TC 丙 ?"));
    assert!(out.contains("✓3 "), "统计行 done 计数接真值: {out}");
    // 无事件窗(纯契约)不怀疑:同 fixture 关掉 events 后 `?` 消失
    dash.events_present = false;
    let out = render_digest(&dash);
    assert!(!out.contains(" ?"), "无事件窗不作怀疑: {out}");
}

/// --strict 判据(本仓首个内容性退出码的数据面):failed gate **或**
/// blocked 任务在场 → `true`;两者皆无 → `false`(passed 门/done 任务不算)。
#[test]
fn needs_attention_matches_failed_gate_or_blocked_only() {
    let mut dash = quiet_dash();
    assert!(!digest_needs_attention(&dash), "无失败门无 blocked → false");
    dash.gates = vec![gate("build", "passed", "")];
    assert!(!digest_needs_attention(&dash), "passed 门不触发 --strict");
    dash.gates = vec![gate("lint", "failed", "boom")];
    assert!(digest_needs_attention(&dash), "failed gate → true");
    dash.gates = Vec::new();
    dash.tasks = vec![task("T1", "阻塞任务", TaskState::Blocked, None)];
    assert!(digest_needs_attention(&dash), "blocked 任务 → true");
}

// ---------------------------------------------------------------- 退出码面

/// 子进程回放:无 stdin 输入,捕获 stdout/stderr。
fn run(args: &[&str]) -> Output {
    Command::new(EXE)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn agentdash")
}

/// 自清理临时目录(Drop 即删,二进制 fixture 用,非 git 仓即可)。
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "agentdash-digest-{tag}-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir_all(dir.join(".agentdash")).expect("create fixture dir");
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

/// 干净台账:单 pending 任务。
const CLEAN_LEDGER: &str = r#"{
  "$schema": "agentdash.tasklog.v1",
  "title": "digest 样例",
  "tasks": {"T1": {"label": "唯一任务", "state": "pending"}}
}"#;

/// blocked 台账:单 blocked 任务(--strict 的 blocked 判据源)。
const BLOCKED_LEDGER: &str = r#"{
  "$schema": "agentdash.tasklog.v1",
  "title": "digest 样例",
  "tasks": {"T1": {"label": "阻塞任务", "state": "blocked"}}
}"#;

/// 失败门事件流(--strict 的 failed gate 判据源)。
const FAILED_GATE_EVENTS: &str =
    r#"{"kind":"gate","gate":"review","state":"failed","detail":"boom"}"#;

/// --strict 两态钉死:failed gate 或 blocked 任务在场退 1,干净仓退 0;
/// 不带 --strict 恒退 0(内容照常上板)——内容性退出码只随旗标生效。
#[test]
fn cli_strict_exits_one_on_failed_gate_or_blocked_and_zero_when_clean() {
    let blocked = TempDir::new("blocked");
    fs::write(
        blocked.path().join(".agentdash/ledger.json"),
        BLOCKED_LEDGER,
    )
    .expect("write ledger");
    let path = blocked.path().to_str().expect("utf8 path");

    let out = run(&["render", "digest", "--strict", path]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "blocked 在场 --strict 应退 1,stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains('\x1b'), "退 1 时输出仍零 ANSI: {stdout:?}");
    assert!(stdout.contains("⊘ T1"), "内容照常上板: {stdout}");

    // 无 --strict:同一脏仓恒退 0,摘要照打(旗标驱动退出码,内容无差别)
    let out = run(&["render", "digest", path]);
    assert_eq!(out.status.code(), Some(0), "无 --strict 恒退 0");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("⊘ T1"),
        "无 --strict 内容照常"
    );

    let clean = TempDir::new("clean");
    fs::write(clean.path().join(".agentdash/ledger.json"), CLEAN_LEDGER).expect("write ledger");
    let out = run(&[
        "render",
        "digest",
        "--strict",
        clean.path().to_str().expect("utf8 path"),
    ]);
    assert_eq!(out.status.code(), Some(0), "干净仓 --strict 应退 0");

    let gated = TempDir::new("gate");
    fs::write(gated.path().join(".agentdash/ledger.json"), CLEAN_LEDGER).expect("write ledger");
    fs::write(
        gated.path().join(".agentdash/events.jsonl"),
        FAILED_GATE_EVENTS,
    )
    .expect("write events");
    let out = run(&[
        "render",
        "digest",
        "--strict",
        gated.path().to_str().expect("utf8 path"),
    ]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "failed gate 在场 --strict 应退 1: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// 用法错一律退 2:多 PATH(digest 仅单仓)、--format(svg 是图视图专属,
/// digest 不收格式)、未知旗标;stderr 必须带错误说明。
#[test]
fn cli_digest_usage_errors_exit_two() {
    let repo = TempDir::new("usage");
    fs::write(repo.path().join(".agentdash/ledger.json"), CLEAN_LEDGER).expect("write ledger");
    let path = repo.path().to_str().expect("utf8 path");
    let cases: &[&[&str]] = &[
        &["render", "digest", path, path],
        &["render", "digest", "--format", "svg", path],
        &["render", "digest", "--format", "ansi", path],
        &["render", "digest", "--bogus"],
    ];
    for args in cases {
        let out = run(args);
        assert_eq!(out.status.code(), Some(2), "args={args:?} 应退 2");
        assert!(!out.stderr.is_empty(), "args={args:?} 应有错误说明");
    }
}
