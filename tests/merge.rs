//! W1-005 模型合并集成测试:三源齐全 / 仅 git / 全无 / 契约损坏 四组。
//!
//! agentdash 当前是纯二进制 crate(无 lib 目标),集成测试按 `#[path]` 在 crate
//! 根挂载 `src` 模块树,使 model.rs 内部的 `crate::contract` / `crate::events` /
//! `crate::sources::git` 顶层路径照常解析(mod 声明序经 rustfmt 字母序重排,
//! 与解析无关)。

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
use model::{AgentView, GateView, TaskView};
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

    // git 层始终采集快照;generated_at 为 RFC 3339 UTC 串
    assert!(dash.git.present);
    assert_eq!(dash.git.recent.len(), 2);
    assert!(!dash.generated_at.is_empty());
    assert!(dash.generated_at.ends_with('Z'));
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
        },
        "who 字典序:alice 压过派发更早的 bob"
    );
    assert_eq!(
        dash.agents[1],
        AgentView {
            who: "bob".to_owned(),
            task: Some("改写 render".to_owned()),
            since: "2026-09-13T09:00:00Z".to_owned(),
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
