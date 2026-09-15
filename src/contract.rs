//! 通用任务契约 `ledger.json` 解析与校验(spec §4.1,W1-002 车道)。
//!
//! 语义:损坏(非法 `JSON`、缺 `title`、类型不符、未知状态串)→ [`ContractError::Corrupt`];
//! 非致命偏差(未知 `$schema`、未声明 `profile` 的富态)→ 降级 + 记入 [`Ledger::warnings`];
//! 未知字段一律忽略(`JSON` `Schema` 是发布给集成包的严格校验面,见 `schema/agentdash.tasklog.v1.json`)。

use std::collections::HashMap;
use std::fmt;

use serde::de::{Deserializer, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

/// 台账 `schema` 标识(spec §4.1)。
pub const SCHEMA_ID: &str = "agentdash.tasklog.v1";

/// 解析失败:输入损坏,该源应降级为警告行而不拖垮整体渲染(AD-ERR-001)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractError {
    /// 携带人类可读原因:非法 `JSON`、必填缺失、类型不符或未知状态串。
    Corrupt(String),
}

impl fmt::Display for ContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Corrupt(why) => write!(f, "corrupt ledger.json: {why}"),
        }
    }
}

impl std::error::Error for ContractError {}

/// 任务状态机(§4.1 最小核 `pending → active → done`,终态 `blocked`;
/// `review` / `fix-round` 为可选富态,仅在 `profile` 声明后有效)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskState {
    /// 未开始。
    Pending,
    /// 进行中(富态降级的落点)。
    Active,
    /// 富态:复核中。
    Review,
    /// 富态:返修轮。
    FixRound,
    /// 完成(终态)。
    Done,
    /// 阻塞(终态)。
    Blocked,
}

impl TaskState {
    /// `schema`/警告用的规范小写串。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Review => "review",
            Self::FixRound => "fix-round",
            Self::Done => "done",
            Self::Blocked => "blocked",
        }
    }
}

/// 车道:命名的一组任务引用(数组顺序即展示顺序)。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Lane {
    /// 车道名(如 `A-impl`)。
    pub name: String,
    /// 本车道承担的任务 `id`(字符串或整数引用,统一收成字符串)。
    #[serde(default, deserialize_with = "de_task_ids")]
    pub tasks: Vec<String>,
}

/// 屏障:`after` 满足后放行 `unlocks`(一期只承载语义数据,不做调度校验)。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Barrier {
    /// 屏障 `id`(如 `B1`)。
    pub id: String,
    /// 前置任务 `id` 集合。
    #[serde(default, deserialize_with = "de_task_ids")]
    pub after: Vec<String>,
    /// 放行的任务 `id` 集合。
    #[serde(default, deserialize_with = "de_task_ids")]
    pub unlocks: Vec<String>,
}

/// 声明式里程碑(W3-004 加法,可选):显式把任务归入多个里程碑。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Milestone {
    /// 里程碑显示 `id`(如 `M1`)。
    pub id: String,
    /// 里程碑标题。
    pub title: String,
    /// 归入本里程碑的任务 `id` 引用(字符串或整数,统一收成字符串)。
    #[serde(default, deserialize_with = "de_task_ids")]
    pub tasks: Vec<String>,
}

/// 单条任务规格。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TaskSpec {
    /// 动词短语标签。
    pub label: String,
    /// 当前状态。
    pub state: TaskState,
    /// 附加说明(如 `fix round 2/5`)。
    #[serde(default)]
    pub note: Option<String>,
    /// 完成自报时刻(W5-001,可选):RFC 3339 串;写回 `d` 键自动盖章。
    /// 内核不作真伪判定,仅供渲染层与事件窗交叉核对(物证 `?` 标记)。
    #[serde(default)]
    pub done_at: Option<String>,
}

/// 任务台账(spec §4.1)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ledger {
    /// 波次编号(如 `W23`)。
    pub wave: Option<String>,
    /// 波次标题(必填)。
    pub title: String,
    /// 语义超集标识(如 `sdd`);声明后 `review` / `fix-round` 富态有效。
    pub profile: Option<String>,
    /// 车道列表。
    pub lanes: Vec<Lane>,
    /// 任务表:`id → 规格`。
    pub tasks: HashMap<String, TaskSpec>,
    /// 屏障列表。
    pub barriers: Vec<Barrier>,
    /// 声明式里程碑(W3-004,可选);缺省为空表,模型层据 `wave` + 全任务
    /// 合成单里程碑兜底。
    pub milestones: Vec<Milestone>,
    /// 非致命降级/提示(如富态无 `profile` 降级、未知 `$schema`)。
    pub warnings: Vec<String>,
}

/// `serde` 镜像:字段与 `JSON` 键一一对应(除 `$schema`),未知字段默认忽略。
#[derive(Deserialize)]
struct RawLedger {
    /// `schema` 标识声明;不识别 → 警告不报错(AD-ERR-003 的契约侧半边)。
    #[serde(rename = "$schema", default)]
    schema: Option<String>,
    /// 波次编号。
    #[serde(default)]
    wave: Option<String>,
    /// 波次标题(必填)。
    title: String,
    /// `profile` 声明。
    #[serde(default)]
    profile: Option<String>,
    /// 车道列表。
    #[serde(default)]
    lanes: Vec<Lane>,
    /// 任务表。
    #[serde(default)]
    tasks: HashMap<String, TaskSpec>,
    /// 屏障列表。
    #[serde(default)]
    barriers: Vec<Barrier>,
    /// 声明式里程碑:先按原样接住(`Value`),结构校验放后方降级——损坏时
    /// 只折损自身(警告 + 空表),不拖垮整账(降级铁律,W3-004)。
    #[serde(default)]
    milestones: Option<serde_json::Value>,
}

/// 台账里任务引用可为字符串或整数(`"tasks": [1, 2]`),统一收成字符串 `id`。
fn de_task_ids<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct TaskIds;

    impl<'de> Visitor<'de> for TaskIds {
        type Value = Vec<String>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a list of task ids (string or integer)")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut ids = Vec::new();
            while let Some(item) = seq.next_element::<serde_json::Value>()? {
                match item {
                    serde_json::Value::String(id) => ids.push(id),
                    serde_json::Value::Number(num) => ids.push(num.to_string()),
                    other => {
                        return Err(serde::de::Error::custom(format!(
                            "task id must be a string or integer, got {other}"
                        )));
                    }
                }
            }
            Ok(ids)
        }
    }

    deserializer.deserialize_seq(TaskIds)
}

/// 富态在未声明 `profile` 时应降级到的基础态;本就合法则 `None`。
const fn rich_without_profile(state: TaskState, profile: Option<&str>) -> Option<TaskState> {
    match (state, profile) {
        (TaskState::Review | TaskState::FixRound, None) => Some(TaskState::Active),
        _ => None,
    }
}

/// 解析并校验台账文本(spec §4.1)。
///
/// # Errors
/// 输入损坏(非法 `JSON`、缺 `title`、类型不符、未知状态串)时返回 [`ContractError::Corrupt`];
/// 非致命偏差不报错,降级并记入 [`Ledger::warnings`]。
pub fn parse_ledger(text: &str) -> Result<Ledger, ContractError> {
    let raw = serde_json::from_str::<RawLedger>(text)
        .map_err(|err| ContractError::Corrupt(err.to_string()))?;

    let mut warnings = Vec::new();
    if let Some(declared) = raw.schema.as_deref()
        && declared != SCHEMA_ID
    {
        warnings.push(format!(
            "unknown `$schema` `{declared}` (expected `{SCHEMA_ID}`); parsed as v1 core"
        ));
    }

    // 可选 `milestones` 降级校验(W3-004):结构损坏 → 警告 + 空表(缺省 /
    // `null` 视同未声明),其余字段照常——增强字段坏了不拖垮台账
    let milestones = match raw.milestones {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(raw_milestones) => serde_json::from_value::<Vec<Milestone>>(raw_milestones).unwrap_or_else(
            |err| {
                warnings.push(format!(
                    "malformed `milestones` ({err}); ignored, falling back to single-milestone synthesis"
                ));
                Vec::new()
            },
        ),
    };

    let mut tasks = raw.tasks;
    let mut degraded: Vec<(String, String)> = Vec::new();
    for (id, task) in &mut tasks {
        if let Some(base) = rich_without_profile(task.state, raw.profile.as_deref()) {
            degraded.push((
                id.clone(),
                format!(
                    "task {id}: rich state `{}` needs a declared `profile`; downgraded to `{}`",
                    task.state.as_str(),
                    base.as_str()
                ),
            ));
            task.state = base;
        }
    }
    // `HashMap` 遍历顺序不定;警告按任务 `id` 排序保证输出确定性。
    degraded.sort();
    warnings.extend(degraded.into_iter().map(|(_, message)| message));

    Ok(Ledger {
        wave: raw.wave,
        title: raw.title,
        profile: raw.profile,
        lanes: raw.lanes,
        tasks,
        barriers: raw.barriers,
        milestones,
        warnings,
    })
}
