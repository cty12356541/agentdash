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

// ------------------------------ W14-07:started_at 真周转时长 / CJK 列宽对齐 / 无事件有台账 / --host 大小写

/// 测试侧东亚显示宽计算(bin-only crate 子进程集成测试核验列宽对齐)。
fn col_width(text: &str) -> usize {
    text.chars()
        .map(|ch| {
            if matches!(
                u32::from(ch),
                0x1100..=0x115F
                    | 0x2E80..=0x303E
                    | 0x3041..=0x33FF
                    | 0x3400..=0x4DBF
                    | 0x4E00..=0x9FFF
                    | 0xA000..=0xA4CF
                    | 0xA960..=0xA97F
                    | 0xAC00..=0xD7A3
                    | 0xF900..=0xFAFF
                    | 0xFE10..=0xFE19
                    | 0xFE30..=0xFE6F
                    | 0xFF01..=0xFF60
                    | 0xFFE0..=0xFFE6
                    | 0x1F300..=0x1F64F
                    | 0x1F900..=0x1F9FF
                    | 0x20000..=0x2FFFD
                    | 0x30000..=0x3FFFD
            ) {
                2
            } else {
                1
            }
        })
        .sum()
}

/// 全量带 `started_at` + `done_at` 的 done 任务:跨 `Z` / `+08:00` / `+0800`
/// 时区折算真周转时长,表头输出中位 / 最大 / 均值 / 样本数,并覆盖偶数样本中位数
/// 与 `<60s` / `<3600s` / `>=3600s` 三档 `fmt_duration` 格式。
#[test]
fn stats_turnover_timed_durations_median_max_mean() {
    // 3 个 done 样本:60s(1m) / 180s(3m,跨 +08:00 与 Z) / 600s(10m,+0800 基本偏移)
    // → 中位 3m,最大 10m,均值 (60+180+600)/3 = 280s = 4m40s
    let timed_ledger = r#"{
  "$schema": "agentdash.tasklog.v1",
  "wave": "W14",
  "title": "真周转样例",
  "tasks": {
    "T1": {
      "label": "甲任务",
      "state": "done",
      "started_at": "2026-09-13T08:00:00Z",
      "done_at": "2026-09-13T08:01:00Z"
    },
    "T2": {
      "label": "乙任务",
      "state": "done",
      "started_at": "2026-09-13T16:10:00+08:00",
      "done_at": "2026-09-13T08:13:00Z"
    },
    "T3": {
      "label": "丙任务",
      "state": "done",
      "started_at": "2026-09-13T17:00:00+0800",
      "done_at": "2026-09-13T17:10:00+08:00"
    },
    "T4": {
      "label": "在跑任务",
      "state": "active",
      "started_at": "2026-09-13T18:00:00Z"
    }
  }
}"#;
    let repo = TempDir::new("timed-odd");
    let path = repo.write_dash("events.jsonl", MULTI_HOST_EVENTS);
    repo.write_dash("ledger.json", timed_ledger);

    let out = run(&["stats", &path]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let expected_tail = "\
任务周转: 中位 3m · 最大 10m · 均值 4m40s (样本 3)
  T1 甲任务 · 1m (2026-09-13T08:00:00Z → 2026-09-13T08:01:00Z)
  T2 乙任务 · 3m (2026-09-13T16:10:00+08:00 → 2026-09-13T08:13:00Z)
  T3 丙任务 · 10m (2026-09-13T17:00:00+0800 → 2026-09-13T17:10:00+08:00)";
    assert!(
        stdout.trim_end_matches('\n').ends_with(expected_tail),
        "三样本真周转表不符:\n{stdout}"
    );
    assert!(
        !stdout.contains("N/A(无 done_at)"),
        "unstamped==0 不出 N/A 行: {stdout}"
    );

    // 偶数样本(45s + 3665s → 中位/均值 1855s = 30m55s,最大 1h01m05s)+ `--host` 过滤注记
    let even_ledger = r#"{
  "$schema": "agentdash.tasklog.v1",
  "title": "偶数样本与跨小时格式",
  "tasks": {
    "T1": {
      "label": "秒级任务",
      "state": "done",
      "started_at": "2026-09-13T08:00:00Z",
      "done_at": "2026-09-13T08:00:45Z"
    },
    "T2": {
      "label": "跨时任务",
      "state": "done",
      "started_at": "2026-09-13T09:00:00Z",
      "done_at": "2026-09-13T10:01:05Z"
    }
  }
}"#;
    let repo_even = TempDir::new("timed-even");
    let path_even = repo_even.write_dash("events.jsonl", MULTI_HOST_EVENTS);
    repo_even.write_dash("ledger.json", even_ledger);

    let out_even = run(&["stats", &path_even, "--host", "claude"]);
    assert_eq!(out_even.status.code(), Some(0));
    let stdout_even = String::from_utf8_lossy(&out_even.stdout);
    let expected_even_tail = "\
任务周转: 中位 30m55s · 最大 1h01m05s · 均值 30m55s (样本 2)
  (台账任务无宿主字段,本表不随 --host 过滤)
  T1 秒级任务 · 45s (2026-09-13T08:00:00Z → 2026-09-13T08:00:45Z)
  T2 跨时任务 · 1h01m05s (2026-09-13T09:00:00Z → 2026-09-13T10:01:05Z)";
    assert!(
        stdout_even
            .trim_end_matches('\n')
            .ends_with(expected_even_tail),
        "偶数样本 + >=3600s 格式 + --host 注记不符:\n{stdout_even}"
    );
}

/// 混合台账:2 个合法计时任务 + 1 个仅 `done_at` 任务 + 1 个时钟倒拨
/// (`done_at < started_at`,绝不下溢 panic,降级按无有效 `started_at` 列行)+
/// 1 个无 `done_at` 的 done 任务(`N/A` 汇总)。
#[test]
fn stats_turnover_mixed_timed_untimed_clock_skew_and_unstamped() {
    let mixed_ledger = r#"{
  "$schema": "agentdash.tasklog.v1",
  "wave": "W14",
  "title": "混合周转样例",
  "tasks": {
    "T1": {
      "label": "瞬时完成",
      "state": "done",
      "started_at": "2026-09-13T09:00:00Z",
      "done_at": "2026-09-13T09:00:00Z"
    },
    "T2": {
      "label": "两分完成",
      "state": "done",
      "started_at": "2026-09-13T09:00:00Z",
      "done_at": "2026-09-13T09:02:00Z"
    },
    "T3": {
      "label": "缺起始戳",
      "state": "done",
      "done_at": "2026-09-13T10:30:00+08:00"
    },
    "T4": {
      "label": "时钟倒拨",
      "state": "done",
      "started_at": "2026-09-13T12:00:00Z",
      "done_at": "2026-09-13T11:55:00Z"
    },
    "T5": {
      "label": "缺完成戳",
      "state": "done",
      "started_at": "2026-09-13T08:00:00Z"
    }
  }
}"#;
    let repo = TempDir::new("mixed");
    let path = repo.write_dash("events.jsonl", MULTI_HOST_EVENTS);
    repo.write_dash("ledger.json", mixed_ledger);

    let out = run(&["stats", &path]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let expected_tail = "\
任务周转: 中位 1m · 最大 2m · 均值 1m (样本 2)
  T1 瞬时完成 · 0s (2026-09-13T09:00:00Z → 2026-09-13T09:00:00Z)
  T2 两分完成 · 2m (2026-09-13T09:00:00Z → 2026-09-13T09:02:00Z)
  T3 缺起始戳 · 2026-09-13T10:30:00+08:00 (无有效 started_at)
  T4 时钟倒拨 · 2026-09-13T11:55:00Z (无有效 started_at)
  N/A(无 done_at): 1";
    assert!(
        stdout.trim_end_matches('\n').ends_with(expected_tail),
        "混合台账周转表(含倒拨防护与 N/A)不符:\n{stdout}"
    );
}

/// W12-delta B②:无生效事件但 `ledger.json` 在场时,首行输出无事件提示(或
/// `--host` 未知宿主说明),紧跟渲染 ③ 任务周转表,恒退 0。
#[test]
fn stats_ledger_without_events_renders_notice_and_turnover_table() {
    let repo = TempDir::new("ledger-only");
    let path = repo.write_dash("ledger.json", LEDGER);

    let out = run(&["stats", &path]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let expected = "\
stats: 无生效事件(events.jsonl 缺失、为空或全为残缺行)
任务周转: 契约无 started_at,仅列完成时刻(不虚构时长)
  T1 甲任务 · 2026-09-13T09:00:00Z
  T2 乙任务 · 2026-09-13T10:30:00+08:00
  N/A(无 done_at): 1";
    assert_eq!(
        stdout.trim_end_matches('\n'),
        expected,
        "无 events 有 ledger 必须出空事件说明 + ③ 周转表:\n{stdout}"
    );

    // 残缺 events.jsonl + `--host agy` + ledger 在场:首行宿主无记录说明 + ③ 周转表
    repo.write_dash("events.jsonl", "broken line\n");
    let out_host = run(&["stats", &path, "--host", "agy"]);
    assert_eq!(out_host.status.code(), Some(0));
    let stdout_host = String::from_utf8_lossy(&out_host.stdout);
    let expected_host = "\
宿主 agy: 无任何事件记录
任务周转: 契约无 started_at,仅列完成时刻(不虚构时长)
  (台账任务无宿主字段,本表不随 --host 过滤)
  T1 甲任务 · 2026-09-13T09:00:00Z
  T2 乙任务 · 2026-09-13T10:30:00+08:00
  N/A(无 done_at): 1";
    assert_eq!(
        stdout_host.trim_end_matches('\n'),
        expected_host,
        "残缺 events + --host + ledger 在场:\n{stdout_host}"
    );
}

/// W12-delta A①:CJK 门名(`"门禁构建"`,4 字符 = 8 显示列)+ 3 字符短宿主
/// (`"agy"`,短于 4 列 `"host"` 表头)+ CJK 宿主名,逐行校验终端显示列宽对齐。
#[test]
fn stats_cjk_gate_and_short_host_align_by_display_width() {
    let events = concat!(
        r#"{"kind":"gate","gate":"门禁构建","state":"passed","exit":0,"detail":"ok","host":"agy","ts":"2026-09-13T08:00:00Z"}"#,
        "\n",
        r#"{"kind":"gate","gate":"pytest","state":"failed","exit":1,"detail":"fail","host":"智谱","ts":"2026-09-13T08:01:00Z"}"#,
        "\n",
        r#"{"kind":"tool","tool":"bash","phase":"end","exit":0,"summary":"ls","host":"agy","ts":"2026-09-13T08:02:00Z"}"#,
        "\n",
    );
    let repo = TempDir::new("cjk-align");
    let path = repo.write_dash("events.jsonl", events);

    let out = run(&["stats", &path]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let expected = "\
宿主使用率(生效事件按 host 戳计数;无戳归 unknown)
  agy   2
  智谱  1
gate 通过率(终态折叠计数;unknown = exit 不可知折叠)
  gate      host  passed  failed  unknown
  pytest    智谱       0       1        0
  门禁构建  agy        1       0        0";
    assert_eq!(
        stdout.trim_end_matches('\n'),
        expected,
        "CJK 门名/宿主与 3 字符短宿主列宽对齐:\n{stdout}"
    );

    let lines: Vec<&str> = stdout.lines().collect();
    // ① 表两数据行显示宽完全相等(`  agy   2` == 9 列,`  智谱  1` == 9 列)
    assert_eq!(col_width(lines[1]), col_width(lines[2]));
    // ② 表表头与所有数据行显示宽逐行严格相等
    let header_w = col_width(lines[4]);
    for row in &lines[5..] {
        assert_eq!(
            col_width(row),
            header_w,
            "② 表行显示宽应与表头({header_w})一致: {row:?}"
        );
    }
}

/// W12-delta A②:`--host` 匹配大小写不敏感(`--host CLAUDE` / `--host agy` /
/// `--host AGY` 均能命中对应宿主桶)。
#[test]
fn stats_host_filter_case_insensitive_matching() {
    let events = concat!(
        r#"{"kind":"gate","gate":"cargo-test","state":"passed","exit":0,"detail":"ok","host":"claude","ts":"2026-09-13T08:00:00Z"}"#,
        "\n",
        r#"{"kind":"gate","gate":"pytest","state":"passed","exit":0,"detail":"ok","host":"agy","ts":"2026-09-13T08:01:00Z"}"#,
        "\n",
    );
    let repo = TempDir::new("host-case");
    let path = repo.write_dash("events.jsonl", events);

    let lower = run(&["stats", &path, "--host", "claude"]);
    let upper = run(&["stats", &path, "--host", "CLAUDE"]);
    assert_eq!(lower.status.code(), Some(0));
    assert_eq!(upper.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&upper.stdout),
        String::from_utf8_lossy(&lower.stdout),
        "`--host CLAUDE` 与 `--host claude` 输出应逐字节一致"
    );

    let agy_lower = run(&["stats", &path, "--host", "agy"]);
    let agy_upper = run(&["stats", &path, "--host", "AGY"]);
    let agy_stdout = String::from_utf8_lossy(&agy_upper.stdout);
    assert_eq!(
        agy_stdout,
        String::from_utf8_lossy(&agy_lower.stdout),
        "`--host AGY` 与 `--host agy` 输出应逐字节一致"
    );
    assert!(
        agy_stdout.contains("  agy  1") && agy_stdout.contains("  pytest  agy        1"),
        "`--host AGY` 应命中 agy 宿主行并保底 host 列宽 4:\n{agy_stdout}"
    );
}
