//! 多源模型合并(W1-005):契约台账 > 事件流 > git 快照,按可信序折叠成渲染层模型。
//!
//! 降级矩阵(承 AD-ERR-001:降级不失败):
//! - 契约缺失或损坏 → 警告行 + git 伪任务兜底(`recent` 每条提交一个
//!   [`TaskView`] 单链,恒 `pending`);事件层不受影响,照常合并。
//! - 契约合法 → 台账警告加 `ledger:` 源前缀透传;lane 未声明的任务尾接
//!   (按 id 字典序,保证确定性)。
//! - 三源全无 → 空态 + 引导文案进 [`Dashboard::warnings`]。
//!
//! 挂载约定:本模块经顶层路径(`crate::contract` / `crate::events` /
//! `crate::sources::git`)消费兄弟模块,必须挂在 crate 根(main.rs `mod model;`
//! 或测试 `#[path]` 根挂载),不得嵌套在子模块里。

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
    /// 修复轮次:从 [`Self::note`] 解析 `fix round N/M`(全/半角空格与
    /// 大小写容忍);note 缺失或解析不了保持 [`None`]。
    pub fix_round: Option<(u32, u32)>,
    /// 任务时刻的 RFC 3339 串:契约任务 = `ledger.json` 文件 mtime
    /// (台账整体最后一次落盘时刻);git 伪任务不设([`None`],无逐任务时刻)。
    pub since: Option<String>,
}

/// 屏障边视图(W1-007 起随模型携带;与 `render::graph` 的图侧类型同构,
/// 渲染/布局入口各自换形消费)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BarrierEdges {
    /// 前置任务 id 集(after)。
    pub after: Vec<String>,
    /// 放行任务 id 集(unlocks)。
    pub unlocks: Vec<String>,
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

/// 在跑子代理视图(事件层活跃表投影;渲染层按需取字段)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentView {
    /// agent 身份(事件 `who`)。
    pub who: String,
    /// 任务注记(事件未携带时为 [`None`])。
    pub task: Option<String>,
    /// 首次 `dispatched` 的 `ts` 原串(缺省为空串)。
    pub since: String,
}

/// 验证门视图(后到覆盖先到折尽后的终态快照)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateView {
    /// 门名(事件 `gate`)。
    pub name: String,
    /// 终态,取事件词表:`running` / `passed` / `failed`。
    pub state: String,
    /// 终态 detail 原样携带(`running` 恒为空串)。
    pub detail: String,
}

/// 仪表盘完整模型(渲染层的唯一输入面)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dashboard {
    /// 任务视图列表(台账车道序在前,伪任务单链兜底)。
    pub tasks: Vec<TaskView>,
    /// 里程碑列表(只来自台账;无契约时为空)。
    pub milestones: Vec<MilestoneView>,
    /// 台账屏障边(after → unlocks;无契约时为空)。
    pub barriers: Vec<BarrierEdges>,
    /// 三层收集的全部警告(契约降级/事件残缺行/引导文案)。
    pub warnings: Vec<String>,
    /// 在跑 agent 视图(事件活跃表投影;按 `who` 字典序,输出确定)。
    pub agents: Vec<AgentView>,
    /// 验证门终态视图(取自 [`events::EventModel`],后到覆盖先到;
    /// 按门名字典序,输出确定)。
    pub gates: Vec<GateView>,
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
    merge_with_git(repo, git::snapshot(repo))
}

/// [`merge`] 的分级刷新变体(watch 30s 节流档,W1-007):git 快照由调用方
/// 注入,使最重的 git 探测能压到 30s 边界、其余源照常按 interval 档重建;
/// 合并语义与 [`merge`] 完全一致。
#[must_use]
pub fn merge_with_git(repo: &Path, git: GitFacts) -> Dashboard {
    let mut warnings = Vec::new();

    // 契约层(可信序最高):合法则任务/里程碑出自台账;损坏降级为警告行
    let mut contract_ok = false;
    let mut tasks = Vec::new();
    let mut milestones = Vec::new();
    let mut barriers = Vec::new();
    let ledger_path = repo.join(".agentdash").join("ledger.json");
    if let Some(text) = read_source(&ledger_path, "ledger.json", &mut warnings) {
        match contract::parse_ledger(&text) {
            Ok(ledger) => {
                contract_ok = true;
                // 台账自带警告加源前缀透传,与事件层/损坏降级行可区分
                warnings.extend(
                    ledger
                        .warnings
                        .iter()
                        .map(|warning| format!("ledger: {warning}")),
                );
                // 契约任务 since 统一取台账文件 mtime(台账整体落盘时刻)
                let since = file_mtime_iso(&ledger_path);
                tasks = contract_tasks(&ledger, since.as_deref());
                milestones.push(milestone_of(&ledger));
                barriers = ledger
                    .barriers
                    .iter()
                    .map(|barrier| BarrierEdges {
                        after: barrier.after.clone(),
                        unlocks: barrier.unlocks.clone(),
                    })
                    .collect();
            }
            // `Display` 已带 `corrupt ledger.json:` 前缀,作警告行直接透传
            Err(err) => warnings.push(err.to_string()),
        }
    }

    // 事件层:永远照常合并(契约缺失或损坏都不影响)
    let mut agents = Vec::new();
    let mut gates = Vec::new();
    let events_path = repo.join(".agentdash").join("events.jsonl");
    if let Some(text) = read_source(&events_path, "events.jsonl", &mut warnings) {
        let model = events::replay(text.lines().map(str::to_owned));
        // 投影时就地排序,渲染层免排序即可拿到确定性输出
        agents = model
            .agents
            .into_iter()
            .map(|agent| AgentView {
                who: agent.who,
                task: agent.task,
                since: agent.first_seen,
            })
            .collect();
        agents.sort_by(|a, b| a.who.cmp(&b.who));
        gates = model
            .gates
            .into_iter()
            .map(|(name, state)| {
                let (state, detail) = match state {
                    GateState::Running => ("running", String::new()),
                    GateState::Passed { detail } => ("passed", detail),
                    GateState::Failed { detail } => ("failed", detail),
                };
                GateView {
                    name,
                    state: state.to_owned(),
                    detail,
                }
            })
            .collect();
        gates.sort_by(|a, b| a.name.cmp(&b.name));
        warnings.extend(model.warnings);
    }

    // git 兜底:无契约时用最近提交伪任务单链(recent 每条一个,恒 pending)
    if !contract_ok && git.present {
        tasks = git_tasks(&git);
    }

    // 全无空态 → 引导文案(空任务 + 无 gate、无 agent + 非 git 仓)
    if tasks.is_empty() && gates.is_empty() && agents.is_empty() && !git.present {
        warnings.push(EMPTY_GUIDANCE.to_owned());
    }

    Dashboard {
        tasks,
        milestones,
        barriers,
        warnings,
        agents,
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
/// `since` 逐任务化:契约任务一律携带台账文件 mtime。
fn contract_tasks(ledger: &contract::Ledger, since: Option<&str>) -> Vec<TaskView> {
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
                    fix_round: spec.note.as_deref().and_then(parse_fix_round),
                    since: since.map(str::to_owned),
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
            fix_round: spec.note.as_deref().and_then(parse_fix_round),
            since: since.map(str::to_owned),
        });
    }
    tasks
}

/// 从 note 解析修复轮次 `fix round N/M`:全/半角空格(含连续混排)与
/// 大小写容忍;N/M 须为非负整数且 M>0,其余形态(缺词、非数字、零分母)
/// 一概 [`None`]——解析不了不臆造。
fn parse_fix_round(note: &str) -> Option<(u32, u32)> {
    // 全角空格(U+3000)归一为半角,交给 split_whitespace 吃掉任意空白
    let normalized: String = note
        .chars()
        .map(|ch| if ch == '\u{3000}' { ' ' } else { ch })
        .collect();
    let mut words = normalized.split_whitespace();
    let fix = words.next()?;
    let round = words.next()?;
    let fraction = words.next()?;
    if !fix.eq_ignore_ascii_case("fix") || !round.eq_ignore_ascii_case("round") {
        return None;
    }
    let (done, total) = fraction.split_once('/')?;
    let done = done.parse::<u32>().ok()?;
    let total = total.parse::<u32>().ok()?;
    (total > 0).then_some((done, total))
}

/// 文件 mtime → RFC 3339 UTC 串;不可读、mtime 早于纪元等失败路径一律
/// [`None`](降级为无时刻,绝不失败)。
fn file_mtime_iso(path: &Path) -> Option<String> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let secs = modified.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(utc_timestamp(secs))
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
                fix_round: None,
                since: None,
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
