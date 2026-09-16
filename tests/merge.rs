//! W1-005 模型合并集成测试:三源齐全 / 仅 git / 全无 / 契约损坏 四组。
//!
//! agentdash 当前是纯二进制 crate(无 lib 目标),集成测试按 `#[path]` 在 crate
//! 根挂载 `src` 模块树,使 model.rs 内部的 `crate::contract` / `crate::events` /
//! `crate::sources::git` 顶层路径照常解析(mod 声明序经 rustfmt 字母序重排,
//! 与解析无关)。本组用例只触达合并投影,挂载树的其余 pub 项(如
//! `events::replay` / `merge_with_git` 的直呼面)属死代码,按文件级 allow
//! 放行(同 `tests/events.rs` 约定;W11-003 起 merge 改经 `merge_opts` →
//! `build` 折叠核,不再传递消费上述两项)。

#![allow(dead_code)]

#[path = "../src/contract.rs"]
mod contract;
#[path = "../src/events.rs"]
mod events;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/sources/mod.rs"]
mod sources;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use contract::TaskState;
use model::{AgentView, GateView, MilestoneView, TaskView};
use sources::git::GitFacts;

/// 三源齐全组的合法台账:两个车道任务 + 一个未入车道任务,1/3 done。
const LEDGER: &str = r#"{
  "$schema": "agentdash.tasklog.v1",
  "wave": "W1",
  "title": "wave one",
  "lanes": [{"name": "A-impl", "tasks": ["1", "2"]}],
  "tasks": {
    "1": {"label": "implement contract", "state": "active", "note": "fix round 2/5"},
    "2": {"label": "implement events", "state": "done"},
    "3": {"label": "implement git snapshot", "state": "pending"}
  }
}"#;

/// 三源齐全组的事件流:第 2 行故意残缺,验证事件警告穿透;第 3 行覆盖第 1 行 gate。
const EVENTS: &str = r#"{"kind":"gate","gate":"test","state":"running"}
not json at all
{"kind":"gate","gate":"test","state":"passed","detail":"3 passed"}
"#;

/// agent/gate 投影组的事件流:bob 先派、alice 后派(断言 who 字典序重排),
/// bob 同 who 再派刷新 task 注记(首见 ts 保留),一个 passed gate。
const EVENTS_AGENTS: &str = r#"{"kind":"agent","event":"dispatched","who":"bob","task":"写 render","ts":"2026-09-13T09:00:00Z"}
{"kind":"agent","event":"dispatched","who":"alice","ts":"2026-09-13T08:30:00Z"}
{"kind":"gate","gate":"test","state":"passed","detail":"5 passed"}
{"kind":"agent","event":"dispatched","who":"bob","task":"改写 render"}
"#;

fn run_git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .expect("git should be on PATH for tests");
    assert!(
        status.success(),
        "git {args:?} failed in {}",
        repo.display()
    );
}

fn next_dir(name: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "agentdash-w1-005-{name}-{}-{serial}",
        std::process::id()
    ))
}

fn cleanup(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

/// 建一个受控 fixture 仓库:git init + 本地身份 + 固定分支名 main + 2 次提交。
fn fixture_repo(name: &str) -> PathBuf {
    let repo = next_dir(name);
    fs::create_dir_all(&repo).expect("create fixture dir");
    run_git(&repo, &["init"]);
    run_git(&repo, &["config", "user.name", "agentdash-test"]);
    run_git(
        &repo,
        &["config", "user.email", "agentdash-test@example.com"],
    );
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    run_git(&repo, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    fs::write(repo.join("a.txt"), "first\n").expect("write a.txt");
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "one"]);
    fs::write(repo.join("b.txt"), "second\n").expect("write b.txt");
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "two"]);
    repo
}

/// 第一组:契约 + 事件 + git 三源齐全。
#[test]
fn three_sources_merge_into_dashboard() {
    let repo = fixture_repo("three");
    let dir = repo.join(".agentdash");
    fs::create_dir_all(&dir).expect("create .agentdash");
    fs::write(dir.join("ledger.json"), LEDGER).expect("write ledger.json");
    fs::write(dir.join("events.jsonl"), EVENTS).expect("write events.jsonl");

    let dash = model::merge(&repo);

    // 契约层可信序最高:任务来自台账(车道序在前),而非 git 伪任务
    assert_eq!(dash.tasks.len(), 3);
    // 契约任务 since = ledger.json 文件 mtime(RFC 3339 UTC;三任务同源同戳)
    let mtime = fs::metadata(dir.join("ledger.json"))
        .expect("stat ledger.json")
        .modified()
        .expect("mtime");
    let expected_since = model::utc_timestamp(
        mtime
            .duration_since(std::time::UNIX_EPOCH)
            .expect("post-epoch mtime")
            .as_secs(),
    );
    assert_eq!(
        dash.tasks[0],
        TaskView {
            id: "1".to_owned(),
            label: "implement contract".to_owned(),
            state: TaskState::Active,
            lane: Some("A-impl".to_owned()),
            note: Some("fix round 2/5".to_owned()),
            fix_round: Some((2, 5)),
            since: Some(expected_since.clone()),
            done_at: None,
        },
        "note `fix round 2/5` 解析为 fix_round;since 取台账 mtime"
    );
    assert_eq!(dash.tasks[1].id, "2");
    assert_eq!(dash.tasks[1].state, TaskState::Done);
    assert_eq!(dash.tasks[1].lane.as_deref(), Some("A-impl"));
    assert_eq!(dash.tasks[1].note, None);
    assert_eq!(dash.tasks[1].fix_round, None, "无 note 不得臆造 fix_round");
    assert_eq!(
        dash.tasks[1].since.as_deref(),
        Some(expected_since.as_str()),
        "契约任务一律带台账 mtime since"
    );
    assert_eq!(
        dash.tasks[2].lane, None,
        "未入任何车道的任务尾接,车道为 None"
    );
    assert_eq!(dash.tasks[2].label, "implement git snapshot");
    assert_eq!(
        dash.tasks[2].since.as_deref(),
        Some(expected_since.as_str()),
        "未入车道任务同为契约任务,since 同源"
    );

    // milestone 由 ledger wave/title 聚合:1/3 done
    assert_eq!(dash.milestones.len(), 1);
    let milestone = &dash.milestones[0];
    assert_eq!(milestone.wave.as_deref(), Some("W1"));
    assert_eq!(milestone.title, "wave one");
    assert_eq!(milestone.done, 1);
    assert_eq!(milestone.total, 3);
    assert!(!milestone.is_complete());

    // 事件层照常合并:gate 后到覆盖先到;残缺行警告穿透
    assert_eq!(
        dash.gates,
        vec![GateView {
            name: "test".to_owned(),
            state: "passed".to_owned(),
            detail: "3 passed".to_owned(),
        }],
        "gate 视图按名排序、state 取事件词表"
    );
    assert!(dash.agents.is_empty(), "本组无 agent 事件,活跃表必须为空");
    assert!(
        dash.warnings
            .iter()
            .any(|w| w.contains("line 2: invalid JSON")),
        "事件警告必须穿透到模型层: {:?}",
        dash.warnings
    );
    assert!(
        !dash.warnings.iter().any(|w| w.contains("corrupt")),
        "合法台账不得产生 corrupt 警告: {:?}",
        dash.warnings
    );

    // git 层始终采集快照;generated_at 与事件 ts 同用本地时区偏移格式
    // (`±HH:MM` 收尾,发现 9;W2 时代的 UTC `Z` 断言随本批退役)
    assert!(dash.git.present);
    assert_eq!(dash.git.recent.len(), 2);
    assert!(!dash.generated_at.is_empty());
    let tz = dash.generated_at.as_bytes();
    assert!(
        dash.generated_at.len() == 25 && (tz[19] == b'+' || tz[19] == b'-') && tz[22] == b':',
        "generated_at 须为本地偏移 RFC 3339(YYYY-MM-DDTHH:MM:SS±HH:MM): {}",
        dash.generated_at
    );
    cleanup(&repo);
}

/// 第二组:仅 git(无契约、无事件)→ git 伪任务单链兜底 + 契约缺失警告行
/// (W2-3b/AD-ERR-001:git 属"其余在场源",契约缺失不再静默;W1 的
/// "仅 git 不告警"断言随本批收紧,空态引导仅限三源全无)。
#[test]
fn git_only_repo_builds_pseudo_task_chain() {
    let repo = fixture_repo("git-only");

    let dash = model::merge(&repo);

    assert!(dash.milestones.is_empty(), "milestone 只来自台账");
    assert_eq!(dash.tasks.len(), 2, "recent 每条提交一个伪任务");
    assert_eq!(dash.tasks[0].label, "two", "新提交在前(recent 序)");
    assert_eq!(dash.tasks[1].label, "one");
    assert_eq!(
        dash.tasks[0].id,
        dash.git.recent[0].split(' ').next().expect("sha token"),
        "伪任务 id 取提交行的短 SHA token"
    );
    let mut ids: Vec<&str> = dash.tasks.iter().map(|t| t.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 2, "伪任务 id 不得重复");
    for task in &dash.tasks {
        assert_eq!(task.state, TaskState::Pending, "伪任务恒为 pending");
        assert_eq!(task.lane, None);
        assert_eq!(task.note, None);
        assert_eq!(task.fix_round, None, "伪任务无 note 何来 fix_round");
        assert_eq!(task.since, None, "git 伪任务不设 since(无逐任务时刻)");
    }
    assert!(
        dash.warnings
            .iter()
            .any(|w| w.starts_with("missing ledger.json")),
        "git 在场而契约缺失必须出警告行(AD-ERR-001): {:?}",
        dash.warnings
    );
    assert!(dash.gates.is_empty());
    assert!(dash.agents.is_empty());
    assert!(dash.git.present);
    cleanup(&repo);
}

/// 第三组:全无(无台账、无事件、非 git 仓)→ 空态 + 引导文案。
#[test]
fn nothing_at_all_yields_guidance() {
    let dir = next_dir("empty");
    fs::create_dir_all(&dir).expect("create plain dir");

    let dash = model::merge(&dir);

    assert_eq!(dash.tasks, Vec::<TaskView>::new());
    assert!(dash.milestones.is_empty());
    assert!(dash.gates.is_empty());
    assert!(dash.agents.is_empty());
    assert_eq!(dash.git, GitFacts::absent());
    assert!(
        dash.warnings
            .iter()
            .any(|w| w.contains(".agentdash/ledger.json") && w.contains("events.jsonl")),
        "全无空态必须携带引导文案: {:?}",
        dash.warnings
    );
    assert!(
        !dash
            .warnings
            .iter()
            .any(|w| w.starts_with("missing ledger.json")),
        "三源全无只出空态引导,不叠加契约缺失警告: {:?}",
        dash.warnings
    );
    assert!(!dash.generated_at.is_empty());
    cleanup(&dir);
}

/// 第四组:契约损坏 → 警告行 + 事件层照常 + git 伪任务兜底。
#[test]
fn corrupt_ledger_warns_and_event_layer_still_merges() {
    let repo = fixture_repo("corrupt");
    let dir = repo.join(".agentdash");
    fs::create_dir_all(&dir).expect("create .agentdash");
    fs::write(dir.join("ledger.json"), "{ not json").expect("write broken ledger");
    fs::write(
        dir.join("events.jsonl"),
        r#"{"kind":"gate","gate":"review","state":"passed","detail":"ok"}"#,
    )
    .expect("write events.jsonl");

    let dash = model::merge(&repo);

    assert!(
        dash.warnings
            .iter()
            .any(|w| w.starts_with("corrupt ledger.json")),
        "损坏契约必须降级为警告行: {:?}",
        dash.warnings
    );
    assert_eq!(
        dash.gates,
        vec![GateView {
            name: "review".to_owned(),
            state: "passed".to_owned(),
            detail: "ok".to_owned(),
        }],
        "事件层不受契约损坏影响,照常合并"
    );
    assert_eq!(dash.tasks.len(), 2, "git 伪任务兜底");
    assert!(dash.tasks.iter().all(|t| t.state == TaskState::Pending));
    assert!(
        dash.tasks
            .iter()
            .all(|t| t.since.is_none() && t.fix_round.is_none()),
        "兜底伪任务不带 since/fix_round"
    );
    assert!(dash.milestones.is_empty());
    assert!(
        !dash.warnings.iter().any(|w| w.contains("no data sources")),
        "git 在场时不得出现全无引导: {:?}",
        dash.warnings
    );
    cleanup(&repo);
}

/// 第五组(W2-001):活跃 agent 表与 gate 终态投影进 Dashboard——
/// who 字典序重排(派发序 bob 在前)、同 who 再派刷新 task 且保留首见 ts。
#[test]
fn agents_and_gates_project_into_dashboard_sorted() {
    let repo = fixture_repo("agents");
    let dir = repo.join(".agentdash");
    fs::create_dir_all(&dir).expect("create .agentdash");
    fs::write(dir.join("ledger.json"), LEDGER).expect("write ledger.json");
    fs::write(dir.join("events.jsonl"), EVENTS_AGENTS).expect("write events.jsonl");

    let dash = model::merge(&repo);

    assert_eq!(dash.agents.len(), 2, "两活跃 agent 入模");
    assert_eq!(
        dash.agents[0],
        AgentView {
            who: "alice".to_owned(),
            task: None,
            since: "2026-09-13T08:30:00Z".to_owned(),
            host: None,
            inferred: false,
        },
        "who 字典序:alice 压过派发更早的 bob"
    );
    assert_eq!(
        dash.agents[1],
        AgentView {
            who: "bob".to_owned(),
            task: Some("改写 render".to_owned()),
            since: "2026-09-13T09:00:00Z".to_owned(),
            host: None,
            inferred: false,
        },
        "同 who 再派刷新 task 注记,first_seen 保留首见"
    );
    assert_eq!(
        dash.gates,
        vec![GateView {
            name: "test".to_owned(),
            state: "passed".to_owned(),
            detail: "5 passed".to_owned(),
        }],
        "gate 由重放终态映射,detail 原样携带"
    );
    assert!(
        dash.warnings.is_empty(),
        "合法事件流不得携带警告: {:?}",
        dash.warnings
    );
    cleanup(&repo);
}

/// 附加:RFC 3339 纯函数钉死(闰日、纪元零点)。
#[test]
fn utc_timestamp_formats_known_epochs() {
    assert_eq!(model::utc_timestamp(0), "1970-01-01T00:00:00Z");
    assert_eq!(model::utc_timestamp(1_700_000_000), "2023-11-14T22:13:20Z");
    assert_eq!(model::utc_timestamp(951_782_400), "2000-02-29T00:00:00Z");
}

/// 第六组(W2-002):note → `fix_round` 解析矩阵——全角/半角/混排空格容忍,
/// 大小写容忍;非数字、零分母、缺词一概保持 `None`(解析不了不臆造)。
#[test]
fn fix_round_parses_tolerantly_or_stays_none() {
    const LEDGER_NOTES: &str = r#"{
      "$schema": "agentdash.tasklog.v1",
      "title": "fix note parsing",
      "tasks": {
        "a": {"label": "全角空格", "state": "active", "note": "fix　round　3/7"},
        "b": {"label": "连续半角空格", "state": "active", "note": "fix  round  2/5"},
        "c": {"label": "大写容忍", "state": "active", "note": "FIX ROUND 4/6"},
        "d": {"label": "非数字", "state": "active", "note": "fix round x/y"},
        "e": {"label": "零分母", "state": "active", "note": "fix round 1/0"},
        "f": {"label": "缺 fix 词", "state": "active", "note": "round 2/5"},
        "g": {"label": "分数缺一", "state": "active", "note": "fix round 2"}
      }
    }"#;
    let repo = fixture_repo("fix-notes");
    let dir = repo.join(".agentdash");
    fs::create_dir_all(&dir).expect("create .agentdash");
    fs::write(dir.join("ledger.json"), LEDGER_NOTES).expect("write ledger.json");

    let dash = model::merge(&repo);

    let round_of = |id: &str| {
        dash.tasks
            .iter()
            .find(|task| task.id == id)
            .unwrap_or_else(|| panic!("task {id} missing"))
            .fix_round
    };
    assert_eq!(round_of("a"), Some((3, 7)), "全角空格(U+3000)容忍");
    assert_eq!(round_of("b"), Some((2, 5)), "连续半角空格容忍");
    assert_eq!(round_of("c"), Some((4, 6)), "大小写容忍");
    assert_eq!(round_of("d"), None, "非数字解析不了保持 None");
    assert_eq!(round_of("e"), None, "零分母不是合法轮次");
    assert_eq!(round_of("f"), None, "缺 `fix` 词不解析");
    assert_eq!(round_of("g"), None, "缺 `/M` 不解析");
    // 全部为契约任务:since 一律取台账 mtime
    assert!(
        dash.tasks.iter().all(|task| task.since.is_some()),
        "契约任务一律携带台账 mtime since: {:?}",
        dash.tasks
    );
    cleanup(&repo);
}

/// 第七组(W2-002):台账警告贯通 Dashboard 时加 `ledger:` 源前缀,
/// 与事件层警告(`events.jsonl …`)和损坏降级行(`corrupt ledger.json: …`)可区分。
#[test]
fn contract_warnings_carry_ledger_source_prefix() {
    const LEDGER_OLD_SCHEMA: &str = r#"{
      "$schema": "agentdash.tasklog.v0",
      "title": "prefixed warnings",
      "tasks": {"1": {"label": "合法任务", "state": "pending"}}
    }"#;
    let repo = fixture_repo("warn-prefix");
    let dir = repo.join(".agentdash");
    fs::create_dir_all(&dir).expect("create .agentdash");
    fs::write(dir.join("ledger.json"), LEDGER_OLD_SCHEMA).expect("write ledger.json");

    let dash = model::merge(&repo);

    assert!(
        dash.warnings
            .iter()
            .any(|w| w.starts_with("ledger: unknown `$schema`")),
        "台账警告必须带 `ledger:` 源前缀透传: {:?}",
        dash.warnings
    );
    assert!(
        !dash.warnings.iter().any(|w| w.contains("corrupt")),
        "合法(仅警告)台账不得混入 corrupt 降级行: {:?}",
        dash.warnings
    );
    assert_eq!(dash.tasks.len(), 1, "警告不降级任务本身");
    cleanup(&repo);
}

/// 第九组(W3-001 D2 空态收紧):空 events 文件 = 源在场(文件级)→ 出
/// 缺失台账警告,不再叠加"无数据源"全无引导(消两行文案互扰)。
#[test]
fn empty_events_file_counts_as_present_source() {
    let dir = next_dir("empty-events");
    let dot = dir.join(".agentdash");
    fs::create_dir_all(&dot).expect("create .agentdash");
    fs::write(dot.join("events.jsonl"), "").expect("write empty events.jsonl");

    let dash = model::merge(&dir);

    assert!(
        dash.warnings
            .iter()
            .any(|w| w.starts_with("missing ledger.json")),
        "events 文件在场而台账缺失必须出缺失警告: {:?}",
        dash.warnings
    );
    assert!(
        !dash.warnings.iter().any(|w| w.contains("no data sources")),
        "events 文件在场(哪怕空文件)不得再出全无引导(D2 收紧): {:?}",
        dash.warnings
    );
    cleanup(&dir);
}

/// 第八组(W2-3b F1,AD-ERR-001):契约**缺失**(有 events 无 ledger.json)
/// 同样降级为警告行——与损坏路径同风格(`missing ledger.json: …`),事件层
/// 照常合并、git 伪任务兜底不受影响。
#[test]
fn missing_ledger_warns_and_other_sources_still_merge() {
    let repo = fixture_repo("missing-ledger");
    let dir = repo.join(".agentdash");
    fs::create_dir_all(&dir).expect("create .agentdash");
    fs::write(
        dir.join("events.jsonl"),
        r#"{"kind":"gate","gate":"review","state":"passed","detail":"ok"}"#,
    )
    .expect("write events.jsonl");

    let dash = model::merge(&repo);

    assert!(
        dash.warnings
            .iter()
            .any(|w| w.starts_with("missing ledger.json")),
        "契约缺失必须降级为警告行(AD-ERR-001): {:?}",
        dash.warnings
    );
    assert_eq!(
        dash.gates,
        vec![GateView {
            name: "review".to_owned(),
            state: "passed".to_owned(),
            detail: "ok".to_owned(),
        }],
        "事件层不受契约缺失影响,照常合并"
    );
    assert_eq!(dash.tasks.len(), 2, "git 伪任务兜底照常");
    assert!(
        !dash.warnings.iter().any(|w| w.contains("no data sources")),
        "其余源在场不得出现全无引导: {:?}",
        dash.warnings
    );
    cleanup(&repo);
}

/// F1 负路径①(W3-001):有台账、无 events → 台账在场,不出
/// `missing ledger.json` 警告(缺失警告只针对台账;events 缺失不告警)。
#[test]
fn ledger_without_events_does_not_warn_missing() {
    const LEDGER_ONLY: &str = r#"{
      "$schema": "agentdash.tasklog.v1",
      "title": "ledger without events",
      "tasks": {"1": {"label": "only task", "state": "active"}}
    }"#;
    let repo = fixture_repo("ledger-no-events");
    let dir = repo.join(".agentdash");
    fs::create_dir_all(&dir).expect("create .agentdash");
    fs::write(dir.join("ledger.json"), LEDGER_ONLY).expect("write ledger.json");

    let dash = model::merge(&repo);

    assert!(
        !dash
            .warnings
            .iter()
            .any(|w| w.starts_with("missing ledger.json")),
        "台账在场不得出缺失警告: {:?}",
        dash.warnings
    );
    assert!(
        dash.warnings.is_empty(),
        "合法台账 + 无 events 不产任何警告: {:?}",
        dash.warnings
    );
    assert_eq!(dash.tasks.len(), 1, "台账任务照常合并");
    cleanup(&repo);
}

/// F1 负路径②(W3-001):台账存在但不可读 → 恰一条 `unreadable` 警告,
/// 无 `missing`(文件在场 ≠ 缺失)。用同名目录顶替文件制造"存在但读不了"
/// (目录 `read_to_string` 必报非 `NotFound` 错),git 伪任务兜底照常。
#[test]
fn unreadable_ledger_warns_only_unreadable() {
    let repo = fixture_repo("unreadable-ledger");
    fs::create_dir_all(repo.join(".agentdash").join("ledger.json"))
        .expect("mkdir ledger.json(同名目录)");

    let dash = model::merge(&repo);

    let unreadable = dash
        .warnings
        .iter()
        .filter(|w| w.contains("ledger.json unreadable"))
        .count();
    assert_eq!(
        unreadable, 1,
        "不可读台账恰一条 unreadable 警告: {:?}",
        dash.warnings
    );
    assert!(
        !dash
            .warnings
            .iter()
            .any(|w| w.starts_with("missing ledger.json")),
        "文件在场(哪怕读不了)不算缺失: {:?}",
        dash.warnings
    );
    assert_eq!(dash.tasks.len(), 2, "git 伪任务兜底照常");
    cleanup(&repo);
}

// ---------- W3-004 多里程碑分组 + 速度线 + project 上模型 + 时区统一 ----------

/// W3-004 D1:声明式里程碑分组——被引用任务入组;重复引用以首见为准并记
/// 警告;未被引用的任务进「未分组」尾组;未知 id 沿车道先例静默跳过。
#[test]
fn declared_milestones_group_tasks_first_wins_with_ungrouped_tail() {
    const LEDGER_MS: &str = r#"{
      "$schema": "agentdash.tasklog.v1",
      "wave": "W3",
      "title": "波标题(声明式时不再上里程碑)",
      "milestones": [
        {"id": "M1", "title": "契约扩展", "tasks": ["01", "02", "99"]},
        {"id": "M2", "title": "渲染接线", "tasks": ["02", "03"]}
      ],
      "tasks": {
        "01": {"label": "任务一", "state": "done"},
        "02": {"label": "任务二", "state": "done"},
        "03": {"label": "任务三", "state": "pending"},
        "04": {"label": "任务四", "state": "done"}
      }
    }"#;
    let repo = fixture_repo("ms-group");
    let dir = repo.join(".agentdash");
    fs::create_dir_all(&dir).expect("create .agentdash");
    fs::write(dir.join("ledger.json"), LEDGER_MS).expect("write ledger.json");

    let dash = model::merge(&repo);

    assert_eq!(
        dash.milestones,
        vec![
            MilestoneView {
                wave: Some("M1".to_owned()),
                title: "契约扩展".to_owned(),
                done: 2,
                total: 2,
            },
            MilestoneView {
                wave: Some("M2".to_owned()),
                title: "渲染接线".to_owned(),
                done: 0,
                total: 1,
            },
            MilestoneView {
                wave: None,
                title: "未分组".to_owned(),
                done: 1,
                total: 1,
            },
        ],
        "M1(01,02;未知 99 静默跳过)→ M2(02 重复以首见为准,只收 03)→ 未分组尾组(04)"
    );
    assert!(
        dash.warnings.iter().any(|w| w.contains("ledger:")
            && w.contains("M2")
            && w.contains("02")
            && w.contains("M1")),
        "重复引用必须带 `ledger:` 前缀警告并点名 M2/02/M1: {:?}",
        dash.warnings
    );
    assert_eq!(dash.tasks.len(), 4, "分组不改任务视图本身");
    cleanup(&repo);
}

/// W3-004 降级铁律(模型侧):milestones 结构损坏 → 台账警告(带 `ledger:`
/// 前缀)+ **单里程碑合成兜底**(wave + 全任务),任务/屏障照常合并。
#[test]
fn malformed_milestones_fall_back_to_single_synthesis() {
    const LEDGER_BROKEN_MS: &str = r#"{
      "$schema": "agentdash.tasklog.v1",
      "wave": "W3",
      "title": "坏里程碑兜底",
      "milestones": "not-a-list",
      "tasks": {
        "01": {"label": "任务一", "state": "done"},
        "02": {"label": "任务二", "state": "pending"}
      }
    }"#;
    let repo = fixture_repo("ms-broken");
    let dir = repo.join(".agentdash");
    fs::create_dir_all(&dir).expect("create .agentdash");
    fs::write(dir.join("ledger.json"), LEDGER_BROKEN_MS).expect("write ledger.json");

    let dash = model::merge(&repo);

    assert_eq!(
        dash.milestones.len(),
        1,
        "损坏 milestones 必须退到单里程碑合成: {:?}",
        dash.milestones
    );
    let synthesis = &dash.milestones[0];
    assert_eq!(synthesis.wave.as_deref(), Some("W3"), "合成取台账 wave");
    assert_eq!(synthesis.title, "坏里程碑兜底", "合成取台账 title");
    assert_eq!((synthesis.done, synthesis.total), (1, 2), "全任务口径");
    assert!(
        dash.warnings
            .iter()
            .any(|w| w.contains("ledger:") && w.contains("milestones") && w.contains("synthesis")),
        "损坏 milestones 必须出点名警告: {:?}",
        dash.warnings
    );
    assert!(
        !dash.warnings.iter().any(|w| w.contains("corrupt")),
        "降级不是损坏,不得混入 corrupt 行: {:?}",
        dash.warnings
    );
    assert_eq!(dash.tasks.len(), 2, "任务视图不受影响");
    cleanup(&repo);
}

/// W3-004 速度线模型口径(纯函数,W3-006 起为事件窗缺失时的回退半边):
/// ≥2 里程碑**且**任务 since 跨度 > 0 → done 总数 / 跨度小时(跨度 =
/// max(since) − min(since));其余一律 `None`。事件窗实参传 `None` =
/// 无合格活动窗,正好钉死回退口径。
#[test]
fn velocity_needs_two_milestones_and_positive_span() {
    let tv = |since: Option<&str>| TaskView {
        id: String::new(),
        label: String::new(),
        state: TaskState::Done,
        lane: None,
        note: None,
        fix_round: None,
        since: since.map(str::to_owned),
        done_at: None,
    };
    let ms = |done: usize, total: usize| MilestoneView {
        wave: None,
        title: String::new(),
        done,
        total,
    };
    // 双里程碑 + 2h 跨度 + 3 done → 1.5 tasks/h(中间戳不影响 max/min)
    let tasks = vec![
        tv(Some("2026-09-13T06:00:00Z")),
        tv(Some("2026-09-13T08:00:00Z")),
        tv(Some("2026-09-13T07:00:00Z")),
    ];
    let milestones = vec![ms(2, 4), ms(1, 3)];
    let rate = model::velocity(&tasks, &milestones, None).expect("双里程碑 + 正跨度必有速度");
    assert!((rate - 1.5).abs() < 1e-9, "3 done / 2h = 1.5, got {rate}");

    assert!(
        model::velocity(&tasks, &milestones[..1], None).is_none(),
        "单里程碑不打速度线"
    );
    let same = vec![
        tv(Some("2026-09-13T08:00:00Z")),
        tv(Some("2026-09-13T08:00:00Z")),
    ];
    assert!(
        model::velocity(&same, &milestones, None).is_none(),
        "任务 since 全同戳 → 跨度 0,不虚报速度"
    );
    assert!(
        model::velocity(&[tv(None)], &milestones, None).is_none(),
        "无任何可解析时间戳 → 不打"
    );
    assert!(
        model::velocity(&[tv(Some("not-a-time"))], &milestones, None).is_none(),
        "坏戳不参与跨度,全坏则不打"
    );
    // 偏移格式与 `Z` 混排按绝对时刻折算(发现 9 后两种形态并存)
    let mixed = vec![
        tv(Some("2026-09-13T14:00:00+08:00")), // = 06:00Z
        tv(Some("2026-09-13T08:00:00Z")),
        tv(Some("2026-09-13T06:30:00-02:00")), // = 08:30Z
    ];
    let rate = model::velocity(&mixed, &milestones, None).expect("混排跨度 2.5h");
    assert!((rate - 1.2).abs() < 1e-9, "3 done / 2.5h = 1.2, got {rate}");
}

/// W3-006 事件窗速度线(纯函数半边):合格事件活动窗(`Some` 且 > 0)优先
/// 于任务 since 跨度——即便后者更大也不采信;`Some(0)`(全同刻窗)不合格,
/// 回退任务 since 跨度。分子与 ≥2 里程碑门槛不变。
#[test]
fn velocity_prefers_qualifying_event_window() {
    let tv = |since: Option<&str>| TaskView {
        id: String::new(),
        label: String::new(),
        state: TaskState::Done,
        lane: None,
        note: None,
        fix_round: None,
        since: since.map(str::to_owned),
        done_at: None,
    };
    let ms = |done: usize, total: usize| MilestoneView {
        wave: None,
        title: String::new(),
        done,
        total,
    };
    // 任务 since 跨度 10h:若误用会得 3/10 = 0.3 而非事件窗口径
    let tasks = vec![
        tv(Some("2026-09-13T06:00:00Z")),
        tv(Some("2026-09-13T16:00:00Z")),
    ];
    let milestones = vec![ms(2, 4), ms(1, 3)];
    let rate = model::velocity(&tasks, &milestones, Some(3_600)).expect("合格活动窗(1h)必有速度");
    assert!(
        (rate - 3.0).abs() < 1e-9,
        "3 done / 1h 事件窗 = 3.0, got {rate}"
    );

    // 全同刻窗 `Some(0)` 不合格 → 回退任务 since 跨度 10h → 0.3
    let rate = model::velocity(&tasks, &milestones, Some(0)).expect("退化窗回退 since 跨度");
    assert!((rate - 0.3).abs() < 1e-9, "3 done / 10h = 0.3, got {rate}");

    // 事件窗在场也过不了单里程碑门槛
    assert!(
        model::velocity(&tasks, &milestones[..1], Some(3_600)).is_none(),
        "单里程碑不打速度线(事件窗不豁免门槛)"
    );
}

/// W3-006 事件窗速度线 fixture:双里程碑台账(3 任务 2 done,完成 2 个入
/// M1、进行中 1 个入 M2)。契约任务 since 恒取台账 mtime,天然零跨度——
/// 与真实 dogfood 同形,活动窗是唯一可用时间基。
const LEDGER_MS2: &str = r#"{
  "$schema": "agentdash.tasklog.v1",
  "wave": "W3",
  "title": "事件窗速度线",
  "milestones": [
    {"id": "M1", "title": "第一批", "tasks": ["1", "2"]},
    {"id": "M2", "title": "第二批", "tasks": ["3"]}
  ],
  "lanes": [{"name": "A-impl", "tasks": ["1", "2", "3"]}],
  "tasks": {
    "1": {"label": "implement contract", "state": "done"},
    "2": {"label": "implement events", "state": "done"},
    "3": {"label": "implement git snapshot", "state": "pending"}
  }
}"#;

/// 本仓 dogfood `events.jsonl` 同款:两条 gate 事件,ts 相隔 6 秒。
const EVENTS_WINDOW: &str = concat!(
    r#"{"gate":"cargo-fmt","kind":"gate","state":"running","ts":"2026-09-14T13:13:32+08:00"}"#,
    "\n",
    r#"{"detail":"rustfmt 1.9.0-stable (8bab26f4f6 2026-07-14)","exit":0,"gate":"cargo-fmt","kind":"gate","state":"passed","ts":"2026-09-14T13:13:38+08:00"}"#,
    "\n",
);

/// W3-006(合并层正断言):双里程碑台账 + dogfood 同款 6 秒事件窗 →
/// `event_span_secs = Some(6)`,velocity 据此点亮;任务 since 跨度恒 0,
/// 活动窗是唯一可用时间基。
#[test]
fn event_window_feeds_velocity_in_merge() {
    let repo = fixture_repo("ms-window");
    let dir = repo.join(".agentdash");
    fs::create_dir_all(&dir).expect("create .agentdash");
    fs::write(dir.join("ledger.json"), LEDGER_MS2).expect("write ledger.json");
    fs::write(dir.join("events.jsonl"), EVENTS_WINDOW).expect("write events.jsonl");

    let dash = model::merge(&repo);

    assert_eq!(
        dash.event_span_secs,
        Some(6),
        "两条事件 ts 相隔 6 秒 → 活动窗 6s: {:?}",
        dash.event_span_secs
    );
    assert_eq!(dash.milestones.len(), 2, "双里程碑在场");
    let rate = model::velocity(&dash.tasks, &dash.milestones, dash.event_span_secs)
        .expect("双里程碑 + 合格活动窗必有速度");
    assert!(
        (rate - 1_200.0).abs() < 1e-9,
        "2 done / (6s = 1/600 h) = 1200 tasks/h, got {rate}"
    );
    cleanup(&repo);
}

/// W3-006(合并层负断言):无 events / 单条 event / 全同刻三态,活动窗
/// 均不合格(`event_span_secs` 为 `None`);契约任务共享台账 mtime,回退
/// 的 since 跨度同为 0 → velocity 一律 `None`,速度行不虚报。
#[test]
fn velocity_hidden_without_qualifying_event_window() {
    const SINGLE: &str = concat!(
        r#"{"gate":"cargo-fmt","kind":"gate","state":"passed","ts":"2026-09-14T13:13:32+08:00"}"#,
        "\n",
    );
    const SAME_TS: &str = concat!(
        r#"{"gate":"cargo-fmt","kind":"gate","state":"running","ts":"2026-09-14T13:13:32+08:00"}"#,
        "\n",
        r#"{"gate":"cargo-fmt","kind":"gate","state":"passed","ts":"2026-09-14T13:13:32+08:00"}"#,
        "\n",
    );
    let cases: [(&str, Option<&str>); 3] = [
        ("no-events", None),
        ("single-event", Some(SINGLE)),
        ("same-ts", Some(SAME_TS)),
    ];
    for (name, events) in cases {
        let repo = fixture_repo(name);
        let dir = repo.join(".agentdash");
        fs::create_dir_all(&dir).expect("create .agentdash");
        fs::write(dir.join("ledger.json"), LEDGER_MS2).expect("write ledger.json");
        if let Some(text) = events {
            fs::write(dir.join("events.jsonl"), text).expect("write events.jsonl");
        }

        let dash = model::merge(&repo);

        assert_eq!(
            dash.milestones.len(),
            2,
            "{name}: 双里程碑在场(负断言只针对事件窗)"
        );
        assert!(
            dash.event_span_secs.is_none(),
            "{name}: 活动窗必须不合格: {:?}",
            dash.event_span_secs
        );
        assert!(
            model::velocity(&dash.tasks, &dash.milestones, dash.event_span_secs).is_none(),
            "{name}: 无合格活动窗且任务同 mtime → 不打速度"
        );
        cleanup(&repo);
    }
}

/// W3-004 发现 9:`generated_at` 与事件 ts 同用本地时区偏移格式(`±HH:MM`,
/// hook `ts_now` 同款),不再是 UTC `Z`;且模型 RFC 3339 解析兼容两种形态。
#[test]
fn generated_at_uses_local_offset_format() {
    let repo = fixture_repo("tz-format");
    let dash = model::merge(&repo);
    let text = dash.generated_at.clone();
    let bytes = text.as_bytes();
    assert!(
        text.len() == 25
            && bytes[4] == b'-'
            && bytes[10] == b'T'
            && bytes[13] == b':'
            && (bytes[19] == b'+' || bytes[19] == b'-')
            && bytes[22] == b':',
        "generated_at 须为 YYYY-MM-DDTHH:MM:SS±HH:MM: {text}"
    );
    assert!(
        model::rfc3339_to_secs(&text).is_some(),
        "模型解析器必须吃自己产出的格式: {text}"
    );
    cleanup(&repo);

    // 偏移语义纯函数钉死:同一瞬时两种写法解析相等
    assert_eq!(
        model::rfc3339_to_secs("2026-09-13T14:00:00+08:00"),
        model::rfc3339_to_secs("2026-09-13T06:00:00Z"),
        "+08:00 偏移按绝对时刻折算"
    );
    assert_eq!(
        model::rfc3339_to_secs("2026-09-13T06:00:00-03:00"),
        model::rfc3339_to_secs("2026-09-13T09:00:00Z"),
        "负偏移同样折算"
    );
    assert_eq!(
        model::rfc3339_to_secs("2026-09-13T06:00:00+99:00"),
        None,
        "越界偏移拒解析"
    );
}

/// W3-004 D3:project 上模型——git 仓根目录名优先;非 git 目录回退 cwd
/// 目录名(cargo test 的 cwd = crate 根,名 `agentdash`);渲染层不再自带
/// 回退链,页眉/图题/oneline 一律读本字段。
#[test]
fn project_follows_git_root_then_cwd_fallback() {
    let repo = fixture_repo("proj-model");
    let expected = repo
        .file_name()
        .expect("fixture dir name")
        .to_string_lossy()
        .into_owned();
    let dash = model::merge(&repo);
    assert_eq!(dash.project, expected, "D3:git 仓根目录名上模型");
    cleanup(&repo);

    let plain = next_dir("plain-proj");
    fs::create_dir_all(&plain).expect("create plain dir");
    let dash = model::merge(&plain);
    assert_eq!(
        dash.project, "agentdash",
        "非 git 仓回退 cwd 目录名(测试进程 cwd = crate 根)"
    );
    cleanup(&plain);
}

// ------------------------------------------------------------ 事件尾投影(W4-002)

#[test]
fn event_tail_projects_agent_and_gate_in_arrival_order() {
    let repo = fixture_repo("tail");
    let dir = repo.join(".agentdash");
    fs::create_dir_all(&dir).expect("create .agentdash");
    fs::write(dir.join("events.jsonl"), EVENTS_AGENTS).expect("write events.jsonl");

    let dash = model::merge(&repo);
    let tail: Vec<_> = dash
        .event_tail
        .iter()
        .map(|e| (e.kind.as_str(), e.name.as_str(), e.state.as_str()))
        .collect();
    assert_eq!(
        tail,
        [
            ("agent", "bob", "dispatched"),
            ("agent", "alice", "dispatched"),
            ("gate", "test", "passed"),
            ("agent", "bob", "dispatched"),
        ],
        "直投影保持到达序(agents/gates 投影的字典序不适用于尾)"
    );
    assert_eq!(dash.event_tail[0].ts, "2026-09-13T09:00:00Z", "ts 原串保留");
    cleanup(&repo);
}

// ------------------------------------------------------------ 物证事实字段(W5-001)

#[test]
fn event_fact_fields_project_for_attestation() {
    let repo = fixture_repo("anchor");
    let dir = repo.join(".agentdash");
    fs::create_dir_all(&dir).expect("create .agentdash");
    fs::write(dir.join("events.jsonl"), EVENTS_WINDOW).expect("write events.jsonl");
    let dash = model::merge(&repo);
    assert!(dash.events_present, "events 文件在场");
    assert_eq!(
        dash.last_gate_passed.as_deref(),
        Some("2026-09-14T13:13:38+08:00"),
        "对账锚 = 最近 passed gate 的 ts 原串"
    );
    cleanup(&repo);

    let repo = fixture_repo("noanchor");
    let dash = model::merge(&repo);
    assert!(!dash.events_present, "无 events 文件 = 源不在场");
    assert_eq!(dash.last_gate_passed, None);
    cleanup(&repo);
}

// ------------------------------------------------------------ ±HHMM 容忍(W6-001)

#[test]
fn rfc3339_accepts_basic_offset_format() {
    let with_colon = model::rfc3339_to_secs("2026-09-15T20:25:56+08:00").unwrap();
    assert_eq!(
        model::rfc3339_to_secs("2026-09-15T20:25:56+0800"),
        Some(with_colon),
        "±HHMM 与 ±HH:MM 同折算"
    );
    assert_eq!(
        model::rfc3339_to_secs("2026-09-15T20:25:56-0530"),
        Some(with_colon + 13 * 3_600 + 30 * 60),
        "西半球负偏移"
    );
    assert_eq!(
        model::rfc3339_to_secs("2026-09-15T20:25:56+080"),
        None,
        "长度不符拒解析"
    );
    assert_eq!(
        model::rfc3339_to_secs("2026-09-15T20:25:56+08000"),
        None,
        "六字节既非 ±HH:MM 也非 ±HHMM,拒"
    );
    assert_eq!(
        model::rfc3339_to_secs("2026-09-15T20:25:56+2400"),
        None,
        "时越界拒解析"
    );
    assert_eq!(
        model::rfc3339_to_secs("2026-09-15T20:25:56+0860"),
        None,
        "分越界拒解析"
    );
}

/// W11-003:无 who completed 配对启发上模——配对行投影 `inferred`(完成侧,
/// 不占在跑计数),警告降频一条汇总;`merge_opts(.., false)`(--no-infer)
/// 回严格丢弃 + 逐行警告,与 W11 前逐字节一致。
#[test]
fn anonymous_completed_projects_inferred_and_no_infer_is_strict() {
    const EVENTS: &str = concat!(
        r#"{"kind":"agent","event":"dispatched","who":"alice","ts":"2026-09-16T09:00:00Z"}"#,
        "\n",
        r#"{"kind":"agent","event":"dispatched","who":"bob","ts":"2026-09-16T09:01:00Z"}"#,
        "\n",
        r#"{"kind":"agent","event":"completed","ts":"2026-09-16T09:02:00Z"}"#,
        "\n",
    );
    let repo = fixture_repo("infer");
    let dir = repo.join(".agentdash");
    fs::create_dir_all(&dir).expect("create .agentdash");
    fs::write(dir.join("ledger.json"), LEDGER).expect("write ledger.json");
    fs::write(dir.join("events.jsonl"), EVENTS).expect("write events.jsonl");

    let dash = model::merge(&repo);
    assert_eq!(
        dash.warnings,
        vec!["1 个无 who completed 已推断配对".to_owned()],
        "缺省开启发:逐行警告降频为一条汇总"
    );
    let alice = dash
        .agents
        .iter()
        .find(|agent| agent.who == "alice")
        .unwrap();
    let bob = dash.agents.iter().find(|agent| agent.who == "bob").unwrap();
    assert!(alice.inferred, "最老在跑 alice 被推断配对(完成侧)");
    assert!(!bob.inferred, "bob 照常在跑");
    assert_eq!(
        dash.running_agents(),
        1,
        "在跑计数只算未推断行:配对行不占在跑"
    );

    let strict = model::merge_opts(&repo, false);
    assert_eq!(
        strict.warnings,
        vec!["line 3: agent event with missing `who`, line dropped".to_owned()],
        "--no-infer:严格丢弃 + 逐行警告,无汇总"
    );
    assert!(
        strict.agents.iter().all(|agent| !agent.inferred),
        "严格路径无推断行"
    );
    assert_eq!(strict.running_agents(), 2);
    cleanup(&repo);
}
