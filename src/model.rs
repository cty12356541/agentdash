//! 多源模型合并(W1-005):契约台账 > 事件流 > git 快照,按可信序折叠成渲染层模型。
//!
//! 降级矩阵(承 AD-ERR-001:降级不失败):
//! - 契约缺失或损坏 → 警告行 + git 伪任务兜底(`recent` 每条提交一个
//!   [`TaskView`] 单链,恒 `pending`);事件层不受影响,照常合并。
//! - 契约合法 → 台账警告透传;lane 未声明的任务尾接(按 id 字典序,保证确定性)。
//! - 三源全无 → 空态 + 引导文案进 [`Dashboard::warnings`]。
//!
//! 挂载约定:本模块经顶层路径(`crate::contract` / `crate::events` /
//! `crate::sources::git`)消费兄弟模块,必须挂在 crate 根(main.rs `mod model;`
//! 或测试 `#[path]` 根挂载),不得嵌套在子模块里。

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::contract::{self, TaskState};
use crate::events::{self, GateState};
use crate::sources::git::{self, GitFacts};

/// 全无空态的引导文案(指向三个约定数据源,不猜路径、不报错)。
const EMPTY_GUIDANCE: &str = concat!(
    "no data sources found: expected `.agentdash/ledger.json`, `.agentdash/events.jsonl`, ",
    "or a git repository; run `agentdash hook` or add a ledger to start tracking"
);

/// 单任务显示视图(模型层最小面,渲染车道按需取字段)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskView {
    /// 任务 id(台账任务 id,或 git 伪任务的短 SHA token)。
    pub id: String,
    /// 动词短语标签(git 伪任务为提交 subject)。
    pub label: String,
    /// 当前状态(git 伪任务恒 `pending`)。
    pub state: TaskState,
    /// 所属车道名;未入车道或 git 伪任务为 [`None`]。
    pub lane: Option<String>,
    /// 附加说明(如 `fix round 2/5`)。
    pub note: Option<String>,
}

/// 里程碑视图:按台账 `wave` / `title` 聚合的任务完成度。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MilestoneView {
    /// 波次编号(台账未声明时为 [`None`])。
    pub wave: Option<String>,
    /// 波次标题。
    pub title: String,
    /// `done` 任务数。
    pub done: usize,
    /// 任务总数。
    pub total: usize,
}

impl MilestoneView {
    /// 完成口径承 claude-dash:`4/4 done` 即全部任务到达终态 `done`。
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.total > 0 && self.done == self.total
    }
}

/// 仪表盘完整模型(渲染层的唯一输入面)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dashboard {
    /// 任务视图列表(台账车道序在前,伪任务单链兜底)。
    pub tasks: Vec<TaskView>,
    /// 里程碑列表(只来自台账;无契约时为空)。
    pub milestones: Vec<MilestoneView>,
    /// 三层收集的全部警告(契约降级/事件残缺行/引导文案)。
    pub warnings: Vec<String>,
    /// 验证门终态视图(取自 [`events::EventModel`],后到覆盖先到)。
    pub gates: HashMap<String, GateState>,
    /// git 快照事实(始终采集,与契约存在与否无关)。
    pub git: GitFacts,
    /// 合并时刻的 UTC 时间(RFC 3339 串,如 `2026-09-13T08:30:00Z`)。
    pub generated_at: String,
}

/// 对 `repo` 目录做多源合并(读 `<repo>/.agentdash/ledger.json` 与
/// `events.jsonl`,并对目录做 git 快照),按 契约 > 事件 > git 可信序折叠。
///
/// 任何单源损坏都不 `panic`、不失败:降级为警告行,其余源照常。
#[must_use]
pub fn merge(repo: &Path) -> Dashboard {
    let mut warnings = Vec::new();
    let git = git::snapshot(repo);

    // 契约层(可信序最高):合法则任务/里程碑出自台账;损坏降级为警告行
    let mut contract_ok = false;
    let mut tasks = Vec::new();
    let mut milestones = Vec::new();
    let ledger_path = repo.join(".agentdash").join("ledger.json");
    if let Some(text) = read_source(&ledger_path, "ledger.json", &mut warnings) {
        match contract::parse_ledger(&text) {
            Ok(ledger) => {
                contract_ok = true;
                warnings.extend(ledger.warnings.iter().cloned());
                tasks = contract_tasks(&ledger);
                milestones.push(milestone_of(&ledger));
            }
            // `Display` 已带 `corrupt ledger.json:` 前缀,作警告行直接透传
            Err(err) => warnings.push(err.to_string()),
        }
    }

    // 事件层:永远照常合并(契约缺失或损坏都不影响)
    let mut gates = HashMap::new();
    let events_path = repo.join(".agentdash").join("events.jsonl");
    if let Some(text) = read_source(&events_path, "events.jsonl", &mut warnings) {
        let model = events::replay(text.lines().map(str::to_owned));
        gates = model.gates;
        warnings.extend(model.warnings);
    }

    // git 兜底:无契约时用最近提交伪任务单链(recent 每条一个,恒 pending)
    if !contract_ok && git.present {
        tasks = git_tasks(&git);
    }

    // 全无空态 → 引导文案(空任务 + 无 gate + 非 git 仓)
    if tasks.is_empty() && gates.is_empty() && !git.present {
        warnings.push(EMPTY_GUIDANCE.to_owned());
    }

    Dashboard {
        tasks,
        milestones,
        warnings,
        gates,
        git,
        generated_at: now_rfc3339(),
    }
}

/// 读一个源文件;不存在静默返回 [`None`],其他 IO 错误降级为警告行。
fn read_source(path: &Path, label: &str, warnings: &mut Vec<String>) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(err) if err.kind() == ErrorKind::NotFound => None,
        Err(err) => {
            warnings.push(format!("{label} unreadable: {err}"));
            None
        }
    }
}

/// 台账 → 任务视图:车道声明序在前(任务在多车道出现取首见),
/// 未入车道的任务尾接并按 id 字典序排列,保证输出确定性。
fn contract_tasks(ledger: &contract::Ledger) -> Vec<TaskView> {
    let mut tasks = Vec::with_capacity(ledger.tasks.len());
    let mut placed: Vec<&str> = Vec::with_capacity(ledger.tasks.len());
    for lane in &ledger.lanes {
        for id in &lane.tasks {
            if placed.contains(&id.as_str()) {
                continue;
            }
            if let Some(spec) = ledger.tasks.get(id) {
                placed.push(id.as_str());
                tasks.push(TaskView {
                    id: id.clone(),
                    label: spec.label.clone(),
                    state: spec.state,
                    lane: Some(lane.name.clone()),
                    note: spec.note.clone(),
                });
            }
        }
    }
    let mut rest: Vec<&String> = ledger
        .tasks
        .keys()
        .filter(|id| !placed.contains(&id.as_str()))
        .collect();
    rest.sort_unstable();
    for id in rest {
        let spec = &ledger.tasks[id];
        tasks.push(TaskView {
            id: id.clone(),
            label: spec.label.clone(),
            state: spec.state,
            lane: None,
            note: spec.note.clone(),
        });
    }
    tasks
}

/// 台账 → 里程碑:`wave` / `title` 原样携带,`done` 数由任务状态聚合。
fn milestone_of(ledger: &contract::Ledger) -> MilestoneView {
    let done = ledger
        .tasks
        .values()
        .filter(|spec| spec.state == TaskState::Done)
        .count();
    MilestoneView {
        wave: ledger.wave.clone(),
        title: ledger.title.clone(),
        done,
        total: ledger.tasks.len(),
    }
}

/// git 快照 → 伪任务单链:每条 `recent` 提交行一个 [`TaskView`],
/// id 取行首短 SHA token,label 取提交 subject,恒 `pending`。
fn git_tasks(git: &GitFacts) -> Vec<TaskView> {
    git.recent
        .iter()
        .map(|line| {
            let (id, label) = match line.split_once(' ') {
                Some((sha, subject)) => (sha.to_owned(), subject.trim().to_owned()),
                None => (line.clone(), line.clone()),
            };
            TaskView {
                id,
                label,
                state: TaskState::Pending,
                lane: None,
                note: None,
            }
        })
        .collect()
}

/// 当前 UTC 时刻的 RFC 3339 串;系统时钟早于纪元时退化为
/// `1970-01-01T00:00:00Z`(绝不失败)。
fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |age| age.as_secs());
    utc_timestamp(secs)
}

/// 纯函数:Unix 纪元秒 → RFC 3339 UTC 串(`YYYY-MM-DDTHH:MM:SSZ`)。
/// 天数→日期用 Hinnant civil 算法;输入恒非负,无失败路径。
#[must_use]
pub fn utc_timestamp(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(i64::MAX);
    let rem = secs % 86_400;
    let (hour, minute, second) = (rem / 3_600, (rem % 3_600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}
