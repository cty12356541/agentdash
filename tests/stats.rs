//! W11-004 agentstats 观测面集成测试:`stats [PATH] [--host <name>]` 三表。
//!
//! bin-only crate:统计面子进程回放真二进制(fixture 纯临时目录,与
//! tests/digest.rs 退出码面同约定)。钉:三表黄金、`--host` 过滤、未知宿主
//! 空表 + 说明行、无事件空态恒退 0、零 ANSI、损坏台账降级、用法错退 2;
//! 并钉 W10 终审 Important-1 的顺手清——panel N==1(含展开后)也随行出
//! 无匹配 ⚠ 行。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

/// 被测二进制(cargo 注入的绝对路径)。
const EXE: &str = env!("CARGO_BIN_EXE_agentdash");

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

/// 自清理临时目录(Drop 即删;`.agentdash/` 预建,非 git 仓即可)。
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "agentdash-stats-{tag}-{}-{serial}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".agentdash")).expect("create fixture dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// `.agentdash/` 下落一个文件,返回仓路径字符串(直接作 CLI 位置参数)。
    fn write_dash(&self, name: &str, text: &str) -> String {
        fs::write(self.0.join(".agentdash").join(name), text).expect("write fixture");
        self.0.to_str().expect("utf8 path").to_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.0));
    }
}

/// 多宿主全要素事件流:agent(claude/codex 戳)+ tool(含无戳行)+ 门三态
/// (running 在途 / passed 有码 / failed 有码 / failed 无码 = exit 不可知)+
/// 无 who completed(推断配对,host 戳仍归 claude)。
const MULTI_HOST_EVENTS: &str = concat!(
    r#"{"kind":"agent","event":"dispatched","who":"alice","host":"claude","ts":"2026-09-13T08:00:00Z"}"#,
    "\n",
    r#"{"kind":"agent","event":"completed","who":"alice","host":"claude","ts":"2026-09-13T08:05:00Z"}"#,
    "\n",
    r#"{"kind":"agent","event":"dispatched","who":"bob","host":"codex","ts":"2026-09-13T08:01:00Z"}"#,
    "\n",
    r#"{"kind":"tool","tool":"bash","phase":"end","exit":0,"summary":"ls","host":"claude","ts":"2026-09-13T08:02:00Z"}"#,
    "\n",
    r#"{"kind":"tool","tool":"read","phase":"end","exit":null,"summary":"","host":"codex","ts":"2026-09-13T08:03:00Z"}"#,
    "\n",
    r#"{"kind":"tool","tool":"grep","phase":"end","exit":0,"summary":"x","ts":"2026-09-13T08:04:00Z"}"#,
    "\n",
    r#"{"kind":"gate","gate":"cargo-test","state":"running","host":"claude","ts":"2026-09-13T08:06:00Z"}"#,
    "\n",
    r#"{"kind":"gate","gate":"cargo-test","state":"passed","exit":0,"detail":"ok","host":"claude","ts":"2026-09-13T08:07:00Z"}"#,
    "\n",
    r#"{"kind":"gate","gate":"cargo-test","state":"running","host":"codex","ts":"2026-09-13T08:08:00Z"}"#,
    "\n",
    r#"{"kind":"gate","gate":"cargo-test","state":"failed","exit":2,"detail":"boom","host":"codex","ts":"2026-09-13T08:09:00Z"}"#,
    "\n",
    r#"{"kind":"gate","gate":"cargo-fmt","state":"failed","exit":null,"detail":"(exit unknown)","host":"claude","ts":"2026-09-13T08:10:00Z"}"#,
    "\n",
    r#"{"kind":"gate","gate":"npm-test","state":"failed","exit":null,"detail":"(exit unknown)","ts":"2026-09-13T08:11:00Z"}"#,
    "\n",
    r#"{"kind":"agent","event":"dispatched","who":"carol","host":"claude","ts":"2026-09-13T08:12:00Z"}"#,
    "\n",
    r#"{"kind":"agent","event":"completed","host":"claude","ts":"2026-09-13T08:13:00Z"}"#,
    "\n",
);

/// 台账:2 done 带 `done_at`(异构时区原串)+ 1 done 无 `done_at`(N/A 汇总)+
/// 1 active(不入周转表)。
const LEDGER: &str = r#"{
  "$schema": "agentdash.tasklog.v1",
  "wave": "W11",
  "title": "stats 样例",
  "tasks": {
    "T1": {"label": "甲任务", "state": "done", "done_at": "2026-09-13T09:00:00Z"},
    "T2": {"label": "乙任务", "state": "done", "done_at": "2026-09-13T10:30:00+08:00"},
    "T3": {"label": "丙任务", "state": "done"},
    "T4": {"label": "丁任务", "state": "active"}
  }
}"#;

/// 三表黄金(契约无 `started_at`:周转表只列完成时刻,口径局限入表头)。
const GOLDEN_THREE_TABLES: &str = "\
宿主使用率(生效事件按 host 戳计数;无戳归 unknown)
  claude   8
  codex    4
  unknown  2
gate 通过率(终态折叠计数;unknown = exit 不可知折叠)
  gate        host     passed  failed  unknown
  cargo-fmt   claude        0       0        1
  cargo-test  claude        1       0        0
  cargo-test  codex         0       1        0
  npm-test    unknown       0       0        1
任务周转: 契约无 started_at,仅列完成时刻(不虚构时长)
  T1 甲任务 · 2026-09-13T09:00:00Z
  T2 乙任务 · 2026-09-13T10:30:00+08:00
  N/A(无 done_at): 1";

/// 主钉:三表黄金——多宿主事件 + 门三态(含 exit 不可知折叠)+ 台账
/// `done_at`;输出逐字节、零 ANSI、恒退 0、stderr 净。
#[test]
fn stats_three_tables_golden_fixture() {
    let repo = TempDir::new("golden");
    let path = repo.write_dash("events.jsonl", MULTI_HOST_EVENTS);
    repo.write_dash("ledger.json", LEDGER);

    let out = run(&["stats", &path]);
    assert_eq!(out.status.code(), Some(0), "stats 恒退 0(内容永不影响)");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains('\x1b'), "零 ANSI: {stdout:?}");
    assert!(
        out.stderr.is_empty(),
        "stderr 应净: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        stdout.trim_end_matches('\n'),
        GOLDEN_THREE_TABLES,
        "三表黄金(宿主使用率 / gate 通过率 / 任务周转)"
    );
}

/// `--host claude`:①② 只剩该宿主(计数不变),③ 台账任务无宿主字段、
/// 行不随过滤(表内注明);零 ANSI,恒退 0。
#[test]
fn stats_host_filter_narrows_tables_but_ledger_rows_stay() {
    let repo = TempDir::new("filter");
    let path = repo.write_dash("events.jsonl", MULTI_HOST_EVENTS);
    repo.write_dash("ledger.json", LEDGER);

    let out = run(&["stats", &path, "--host", "claude"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains('\x1b'), "零 ANSI: {stdout:?}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(
        lines.contains(&"  claude  8"),
        "① 只剩 claude 行,计数不变: {stdout}"
    );
    assert!(
        !lines.iter().any(|line| line.starts_with("  codex")),
        "其余宿主行不现: {stdout}"
    );
    assert!(
        !lines.iter().any(|line| line.starts_with("  unknown")),
        "unknown 桶行同样被滤: {stdout}"
    );
    assert!(
        stdout.contains("cargo-test") && stdout.contains("claude"),
        "② 只剩 claude 的折叠行: {stdout}"
    );
    assert!(
        !stdout.contains("npm-test") && !stdout.contains("codex"),
        "非该宿主折叠行(npm-test·unknown / cargo-test·codex)不现: {stdout}"
    );
    assert!(
        stdout.contains("(台账任务无宿主字段,本表不随 --host 过滤)"),
        "③ 表内注明不随过滤: {stdout}"
    );
    assert!(stdout.contains("T1 甲任务"), "③ 台账行保留(无宿主维度可滤)");
}

/// `--host ghost`(无任何事件的宿主):①② 空表零残留,单行说明 + ③ 照常,
/// 恒退 0。
#[test]
fn stats_unknown_host_note_with_empty_tables_and_turnover_stays() {
    let repo = TempDir::new("ghost");
    let path = repo.write_dash("events.jsonl", MULTI_HOST_EVENTS);
    repo.write_dash("ledger.json", LEDGER);

    let out = run(&["stats", &path, "--host", "ghost"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.first().copied(),
        Some("宿主 ghost: 无任何事件记录"),
        "未知宿主首行说明: {stdout}"
    );
    assert!(!stdout.contains("claude"), "①② 空表:他宿主零残留: {stdout}");
    assert!(stdout.contains("任务周转"), "③ 照常上板: {stdout}");
    assert!(
        stdout.contains("T1 甲任务"),
        "③ 行不因宿主过滤消失: {stdout}"
    );
}

/// 无事件(events.jsonl 缺失)= 单行空态,恒退 0。
#[test]
fn stats_without_events_prints_single_line_and_exits_zero() {
    let repo = TempDir::new("empty");
    let path = repo.path().to_str().expect("utf8 path").to_owned();
    let out = run(&["stats", &path]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.trim_end_matches('\n'),
        "stats: 无生效事件(events.jsonl 缺失、为空或全为残缺行)",
        "无事件 = 单行空态(台账在场也不出表): {stdout:?}"
    );
}

/// 事件全残缺(非法 JSON + 未知 kind)= 同一空态,不虚报零值表。
#[test]
fn stats_all_invalid_lines_fold_to_empty_state() {
    let repo = TempDir::new("invalid");
    let path = repo.write_dash("events.jsonl", "not json\n{\"kind\":\"mystery\"}\n");
    let out = run(&["stats", &path]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.trim_end_matches('\n'),
        "stats: 无生效事件(events.jsonl 缺失、为空或全为残缺行)",
        "全残缺同空态: {stdout:?}"
    );
}

/// 台账损坏 → ③ 降级为说明行(events 照常);台账缺失 → ③ 整节零残留;
/// 两者都恒退 0。
#[test]
fn stats_ledger_corrupt_degrades_and_missing_leaves_no_section() {
    let corrupt = TempDir::new("corrupt");
    let path = corrupt.write_dash("events.jsonl", MULTI_HOST_EVENTS);
    corrupt.write_dash("ledger.json", "{broken");
    let out = run(&["stats", &path]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("周转: ledger.json 损坏,无法统计"),
        "损坏台账降级说明行: {stdout}"
    );
    assert!(
        stdout.contains("宿主使用率"),
        "事件表不受台账损坏牵连: {stdout}"
    );

    let missing = TempDir::new("no-ledger");
    let path = missing.write_dash("events.jsonl", MULTI_HOST_EVENTS);
    let out = run(&["stats", &path]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("任务周转"),
        "无台账:③ 整节零残留: {stdout}"
    );
    assert!(stdout.contains("宿主使用率"), "事件表照常: {stdout}");
}

/// 用法错一律退 2:未知旗标 / 多 PATH / `--host` 缺参 / `--host=` 空值;
/// stderr 必须带错误说明。
#[test]
fn stats_usage_errors_exit_two() {
    let repo = TempDir::new("usage");
    let path = repo.path().to_str().expect("utf8 path").to_owned();
    let cases: &[&[&str]] = &[
        &["stats", "--bogus"],
        &["stats", &path, &path],
        &["stats", "--host"],
        &["stats", "--host="],
    ];
    for args in cases {
        let out = run(args);
        assert_eq!(out.status.code(), Some(2), "args={args:?} 应退 2");
        assert!(!out.stderr.is_empty(), "args={args:?} 应有错误说明");
    }
}

// ------------------------------ W10 终审 Important-1 顺手清:panel N==1 无匹配 ⚠

/// 恰 1 仓(含展开后)也随行补 ⚠:pattern 无匹配不再被单仓路径吞掉
/// (全面板与 ⚠ 同屏,非替换);N>1 孪生断言护既有行为不回归。
#[test]
fn panel_single_repo_with_nomatch_pattern_still_warns_w10_important1() {
    let root = TempDir::new("n1");
    let repo = root.0.join("only-repo");
    fs::create_dir_all(&repo).expect("create repo dir");
    let pattern = root.0.join("zz-nope-*");
    let pattern = pattern.to_str().expect("utf8 pattern").to_owned();

    let out = run(&[
        "render",
        "panel",
        repo.to_str().expect("utf8 repo"),
        &pattern,
    ]);
    assert_eq!(out.status.code(), Some(0), "恒退 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("no match"),
        "N==1 必须随行补 ⚠ 行: {stdout}"
    );
    assert!(stdout.contains(&pattern), "⚠ 行携带 pattern 原串: {stdout}");
    assert!(
        stdout.lines().count() > 1,
        "全面板与 ⚠ 同屏(修复不是替换成单行): {stdout}"
    );

    // N>1 孪生:既有 ⚠ 行为不回归
    let repo2 = root.0.join("second-repo");
    fs::create_dir_all(&repo2).expect("create repo dir");
    let out = run(&[
        "render",
        "panel",
        repo.to_str().expect("utf8 repo"),
        repo2.to_str().expect("utf8 repo2"),
        &pattern,
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("no match"), "N>1 ⚠ 既有行为: {stdout}");
}
