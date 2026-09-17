//! 多源模型合并(W1-005):契约台账 > 事件流 > git 快照,按可信序折叠成渲染层模型。
//!
//! 降级矩阵(承 AD-ERR-001:降级不失败):
//! - 契约缺失或损坏 → 警告行(缺失仅当其余源在场才告警)+ git 伪任务兜底
//!   (`recent` 每条提交一个 [`TaskView`] 单链,恒 `pending`);事件层不受
//!   影响,照常合并。
//! - 契约合法 → 台账警告加 `ledger:` 源前缀透传;lane 未声明的任务尾接
//!   (按 id 字典序,保证确定性)。
//! - 三源全无(文件级,D2)→ 空态 + 引导文案进 [`Dashboard::warnings`]。
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
use crate::sources::remote::{self, RemoteFacts};

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
    /// 完成自报时刻(W5-001,可选):契约任务的 `done_at` 原样穿透;git 伪
    /// 任务与折叠伪任务为 [`None`]。真伪不作判定,仅供物证交叉核对。
    pub done_at: Option<String>,
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
    /// 里程碑显示 id:v1 合成时为台账 `wave`(未声明 [`None`]);声明式
    /// 里程碑(W3-004)为其 `id`。
    pub wave: Option<String>,
    /// 里程碑标题(v1 合成 = 台账 `title`;声明式 = 其 `title`;
    /// 未引用任务尾组 = `未分组`)。
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
    /// 宿主归属(W7-001;首次 dispatched 的 `--host` 盖章,缺省 [`None`])。
    pub host: Option<String>,
    /// W11-003:该行已被无 `who` 的 completed **推断配对**为完成——显示层
    /// 标 `▶⇢✓ … (inferred)`,计数走完成侧(不占在跑);推断非实测,严格
    /// 路径(`--no-infer`)恒 `false`。
    pub inferred: bool,
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
    /// W12-009:`failed` 且折叠无退出码证据(exit=null)为真——渲染第三态
    /// `? (unknown)`,与有码真失败 ✗ 视觉可辨;passed/running 恒假。
    pub unknown: bool,
}

/// 详情面板事件尾条目视图(W4-002;[`events::EventModel::tail`] 直投影:
/// 已生效 agent/gate 事件紧凑行,到达序,最近 10 条)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventTailView {
    /// 事件类:`agent` / `gate`。
    pub kind: String,
    /// 主体名:agent 为 `who`,gate 为门名。
    pub name: String,
    /// 事件词:gate 取 state,agent 取 event(dispatched/completed)。
    pub state: String,
    /// `ts` 原串(缺省为空串)。
    pub ts: String,
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
    /// 详情面板事件尾(W4-002;取自 [`events::EventModel::tail`] 直投影,
    /// 到达序,最近 10 条,agent+gate 生效行)。
    pub event_tail: Vec<EventTailView>,
    /// 详情面板事件尾(W5-001 起 `events.jsonl` 文件在场与否的事实字段):
    /// `true` = events.jsonl 存在(哪怕空文件)。
    pub events_present: bool,
    /// 对账锚(W5-001):事件层最近 passed gate 的 ts 原串;无事件/从未有
    /// passed → [`None`]。物证交叉核对的数据源(全窗有效,非 tail 视窗)。
    pub last_gate_passed: Option<String>,
    /// 事件流活动窗跨度(秒,W3-006):events.jsonl ≥2 条不同 `ts` 时取
    /// `max(ts) − min(ts)`(全部 gate/agent/tool 事件按绝对时刻折算);
    /// 无事件 / 单条 / 全同刻为 [`None`]。速度线 span 首选数据源——
    /// [`velocity`] 据此优先,不合格回退任务 `since` 跨度(契约任务恒同
    /// 台账 mtime,跨度恒 0,故真实数据全靠本字段点亮,W2-002 承诺兑现)。
    pub event_span_secs: Option<u64>,
    /// git 快照事实(始终采集,与契约存在与否无关)。
    pub git: GitFacts,
    /// 远程 PR 事实(W2-007;增强非依赖):gh 探测成功且有 PR 时有值,
    /// gh 缺失/非 git 仓/无 PR/超时/断网一律为 `None`——渲染层以此隐藏
    /// PR 区块(降级矩阵"无远程 → 隐藏区块")。
    pub remote: Option<RemoteFacts>,
    /// 项目名(D3 上模型,W3-004):git 仓根目录名 → cwd 目录名 →
    /// `"agentdash"`;页眉/图题/oneline 一律读本字段,渲染层不再自带回退链。
    pub project: String,
    /// 合并时刻的本地时区时间(RFC 3339 带偏移串,如
    /// `2026-09-14T13:13:32+08:00`;与事件 `ts` 同格式——发现 9,W3-004 起
    /// 弃用 `Z` UTC 形态)。
    pub generated_at: String,
}

/// 对 `repo` 目录做多源合并(读 `<repo>/.agentdash/ledger.json` 与
/// `events.jsonl`,并对目录做 git 快照),按 契约 > 事件 > git 可信序折叠。
///
/// 任何单源损坏都不 `panic`、不失败:降级为警告行,其余源照常。
#[must_use]
pub fn merge(repo: &Path) -> Dashboard {
    merge_opts(repo, true)
}

/// [`merge`] 的旗标变体(W11-003):`infer = false` 关闭无 who completed
/// 配对启发(`--no-infer`,严格丢弃 + 逐行警告),`true` 为缺省开启;
/// 其余语义与 [`merge`] 完全一致。
#[must_use]
pub fn merge_opts(repo: &Path, infer: bool) -> Dashboard {
    build(repo, git::snapshot(repo), infer)
}

impl Dashboard {
    /// 在跑 agent 计数(W11-003):只算未推断行——配对行已完成(完成侧
    /// 语义),不占在跑。panel/精要/摘要/oneline 四处计数同源。
    #[must_use]
    pub fn running_agents(&self) -> usize {
        self.agents.iter().filter(|agent| !agent.inferred).count()
    }
}

/// [`merge`] 的分级刷新变体(watch 30s 节流档,W1-007):git 快照由调用方
/// 注入,使最重的 git 探测能压到 30s 边界、其余源照常按 interval 档重建;
/// 合并语义与 [`merge`] 完全一致(启发缺省开启)。
#[must_use]
pub fn merge_with_git(repo: &Path, git: GitFacts) -> Dashboard {
    build(repo, git, true)
}

/// 三入口共用折叠核(W11-003 起):`infer` 只透传事件重放(契约/git 源
/// 与配对启发无关)。
fn build(repo: &Path, git: GitFacts, infer: bool) -> Dashboard {
    let mut warnings = Vec::new();

    // 契约层(可信序最高):合法则任务/里程碑出自台账;损坏降级为警告行
    let mut contract_ok = false;
    let mut tasks = Vec::new();
    let mut milestones = Vec::new();
    let mut barriers = Vec::new();
    let ledger_path = repo.join(".agentdash").join("ledger.json");
    let (ledger_text, ledger_present) = read_source(&ledger_path, "ledger.json", &mut warnings);
    if let Some(text) = ledger_text {
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
                // 里程碑:声明式分组优先(D1);缺省(或声明损坏降级为空)时
                // 由 wave + 全任务合成单里程碑(v1 行为,W3-004)
                let (views, group_warnings) = milestone_views(&ledger);
                milestones = views;
                warnings.extend(
                    group_warnings
                        .iter()
                        .map(|warning| format!("ledger: {warning}")),
                );
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

    // 事件层:永远照常合并(契约缺失或损坏都不影响);投影细节收口
    // [`event_views`](agents/gates 字典序,尾到达序,W4-002)
    let mut event_span_secs = None;
    let events_path = repo.join(".agentdash").join("events.jsonl");
    let (events_text, events_present) = read_source(&events_path, "events.jsonl", &mut warnings);
    let (agents, gates, event_tail, last_gate_passed) = match events_text {
        Some(text) => {
            let model = events::replay_infer(text.lines().map(str::to_owned), infer);
            event_span_secs = event_span_of(&model);
            let (agents, gates, tail, anchor, model_warnings) = event_views(model);
            warnings.extend(model_warnings);
            (agents, gates, tail, anchor)
        }
        None => (Vec::new(), Vec::new(), Vec::new(), None),
    };

    // AD-ERR-001:契约**缺失**(文件不存在,非损坏)同样降级为警告行,措辞
    // 对齐损坏路径;但仅当其余源(事件文件 / git 仓)在场——三源全无走下方
    // 空态引导,不叠加缺失告警(README 空态行为,W2-3b)
    if !ledger_present && (events_present || git.present) {
        warnings.push("missing ledger.json: expected .agentdash/ledger.json".to_owned());
    }

    // git 兜底:无契约时用最近提交伪任务单链(recent 每条一个,恒 pending)
    if !contract_ok && git.present {
        tasks = git_tasks(&git);
    }

    // 全无空态 → 引导文案。D2 收紧(W3-001):仅当三源**文件级**皆无——
    // events 文件存在(哪怕空文件)即源在场,由上方缺失警告行接管,不再
    // 叠加"无数据源"引导(消两行文案互扰)
    if !ledger_present && !events_present && !git.present {
        warnings.push(EMPTY_GUIDANCE.to_owned());
    }

    // D3(W3-004):项目名上模型,git 仓根名 → cwd 目录名 → 兜底;
    // 渲染层 cwd 回退就此退役
    let project = project_of(&git);
    Dashboard {
        tasks,
        milestones,
        barriers,
        warnings,
        agents,
        gates,
        event_tail,
        // 物证交叉核对事实字段(W5-001):events.jsonl 在场与否 + 最近通过门
        events_present,
        last_gate_passed,
        event_span_secs,
        git,
        // 远程层(120s 档):失败静默为 None——远程是增强不是依赖,
        // 绝不因 gh 缺失/断网拖垮合并
        remote: remote::fetch(repo),
        project,
        // 发现 9(W3-004):与事件 ts 同用本地时区偏移格式,页眉不再 UTC/本地并存
        generated_at: ts_now(),
    }
}

/// 事件流活动窗(W3-006):重放模型 → 合格活动窗跨度(秒)。极值折叠在
/// [`events::replay`](<`crate::events`> 侧只存事实),此处只做合格判定:
/// ≥2 个可解析 ts 且极差严格大于 0;零宽窗(单条/全同刻)与无事件同为
/// [`None`],交由 [`velocity`] 回退任务 `since` 跨度。
fn event_span_of(model: &events::EventModel) -> Option<u64> {
    match (model.ts_min, model.ts_max) {
        (Some(min), Some(max)) if max > min => Some(max - min),
        _ => None,
    }
}

/// 事件层投影五元组(agents / gates / 事件尾 / 对账锚 / 重放警告)。
type EventViews = (
    Vec<AgentView>,
    Vec<GateView>,
    Vec<EventTailView>,
    Option<String>,
    Vec<String>,
);

/// 事件重放模型 → 渲染视图五元组:agents 按 `who` 字典序、gates 按门名字典序
/// (投影时就地排序,渲染层免排序即得确定性输出);事件尾保持到达序不排序
/// (W4-002:重放序即叙事序);`last_gate_passed` 对账锚随行(W5-001);
/// 重放警告随行透传。
fn event_views(model: events::EventModel) -> EventViews {
    let mut agents: Vec<AgentView> = model
        .agents
        .into_iter()
        .map(|agent| AgentView {
            who: agent.who,
            task: agent.task,
            since: agent.first_seen,
            host: agent.host,
            inferred: agent.inferred,
        })
        .collect();
    agents.sort_by(|a, b| a.who.cmp(&b.who));
    let mut gates: Vec<GateView> = model
        .gates
        .into_iter()
        .map(|(name, state)| {
            let (state, detail, unknown) = match state {
                GateState::Running => ("running", String::new(), false),
                GateState::Passed { detail } => ("passed", detail, false),
                GateState::Failed { detail, unknown } => ("failed", detail, unknown),
            };
            GateView {
                name,
                state: state.to_owned(),
                detail,
                unknown,
            }
        })
        .collect();
    gates.sort_by(|a, b| a.name.cmp(&b.name));
    let tail = model
        .tail
        .into_iter()
        .map(|entry| EventTailView {
            kind: entry.kind,
            name: entry.name,
            state: entry.state,
            ts: entry.ts,
        })
        .collect();
    (agents, gates, tail, model.last_gate_passed, model.warnings)
}

/// 读一个源文件,返回 `(内容, 文件是否存在)`:不存在(NotFound)静默返回
/// `(None, false)`——存在与否是 AD-ERR-001 缺失警告行的判定输入,不与
/// 其他 IO 错误混同(存在但读不了降级为警告行,返回 `(None, true)`)。
fn read_source(path: &Path, label: &str, warnings: &mut Vec<String>) -> (Option<String>, bool) {
    match fs::read_to_string(path) {
        Ok(text) => (Some(text), true),
        Err(err) if err.kind() == ErrorKind::NotFound => (None, false),
        Err(err) => {
            warnings.push(format!("{label} unreadable: {err}"));
            (None, true)
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
                    fix_round: spec
                        .note
                        .as_deref()
                        .and_then(parse_fix_round)
                        .map(|(round, _)| round),
                    since: since.map(str::to_owned),
                    done_at: spec.done_at.clone(),
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
            fix_round: spec
                .note
                .as_deref()
                .and_then(parse_fix_round)
                .map(|(round, _)| round),
            since: since.map(str::to_owned),
            done_at: spec.done_at.clone(),
        });
    }
    tasks
}

/// 从 note 解析修复轮次前缀 `fix round N/M`:返回 `(轮次, 匹配前缀之后的
/// 残余 note)`(残余去首尾空白,空残余为空串)。全/半角空格(含连续混排)
/// 与大小写容忍;N/M 须为非负整数且 M>0,其余形态(缺词、非数字、零分母)
/// 一概 [`None`]——解析不了不臆造。W3-001 起携带匹配区间信息(第 3 词词尾
/// 即前缀止点):面板只抑匹配前缀,残余 note 照常上板,渲染层复用本解析。
pub(crate) fn parse_fix_round(note: &str) -> Option<((u32, u32), String)> {
    // 全角空格(U+3000)归一为半角再切词;归一化逐字符 1:1,字符下标在
    // 原文中不变,残余可按字符位安全回切
    let normalized: Vec<char> = note
        .chars()
        .map(|ch| if ch == '\u{3000}' { ' ' } else { ch })
        .collect();
    let mut spans: Vec<(usize, usize)> = Vec::with_capacity(3);
    let mut idx = 0;
    while idx < normalized.len() {
        if normalized[idx] == ' ' {
            idx += 1;
            continue;
        }
        let start = idx;
        while idx < normalized.len() && normalized[idx] != ' ' {
            idx += 1;
        }
        spans.push((start, idx));
    }
    if spans.len() < 3 {
        return None;
    }
    let word = |span: (usize, usize)| normalized[span.0..span.1].iter().collect::<String>();
    if !word(spans[0]).eq_ignore_ascii_case("fix") || !word(spans[1]).eq_ignore_ascii_case("round")
    {
        return None;
    }
    let fraction = word(spans[2]);
    let (done, total) = fraction.split_once('/')?;
    let done = done.parse::<u32>().ok()?;
    let total = total.parse::<u32>().ok()?;
    if total == 0 {
        return None;
    }
    // 匹配前缀止于第 3 词词尾;其后残余按字符位回切(保留词间空白,全角
    // 空格已归一为半角)再去首尾空白
    let residual: String = normalized[spans[2].1..].iter().collect();
    Some(((done, total), residual.trim().to_owned()))
}

/// 文件 mtime → RFC 3339 UTC 串;不可读、mtime 早于纪元等失败路径一律
/// [`None`](降级为无时刻,绝不失败)。
fn file_mtime_iso(path: &Path) -> Option<String> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let secs = modified.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(utc_timestamp(secs))
}

/// 台账 → 里程碑视图列表(D1,W3-004):
/// - 无声明(或声明在契约层损坏降级为空)→ v1 单里程碑合成([`milestone_of`]),
///   输出与 W3 前逐字节一致;
/// - 有声明 → 按声明序逐个分组:被引用且存在的任务入组,**重复引用以首见
///   为准**并记警告(带首见里程碑 id),未知 id 沿车道先例静默跳过;未被
///   任何里程碑引用的任务进「未分组」尾组(非空才出)。
///
/// 返回 `(视图, 警告)`;警告由调用方加 `ledger:` 源前缀。
fn milestone_views(ledger: &contract::Ledger) -> (Vec<MilestoneView>, Vec<String>) {
    if ledger.milestones.is_empty() {
        return (vec![milestone_of(ledger)], Vec::new());
    }
    let mut views = Vec::with_capacity(ledger.milestones.len());
    let mut warnings = Vec::new();
    // 已认领任务 → 首见里程碑 id(重复引用判定与警告措辞用)
    let mut claimed: Vec<(&str, &str)> = Vec::with_capacity(ledger.tasks.len());
    for milestone in &ledger.milestones {
        let (mut done, mut total) = (0usize, 0usize);
        for id in &milestone.tasks {
            let Some(spec) = ledger.tasks.get(id) else {
                continue; // 未知 id 沿车道先例静默跳过
            };
            if let Some((_, owner)) = claimed.iter().find(|(task, _)| *task == id.as_str()) {
                warnings.push(format!(
                    "milestone `{}` re-references task `{id}` (already in `{owner}`); first wins",
                    milestone.id
                ));
                continue;
            }
            claimed.push((id.as_str(), milestone.id.as_str()));
            total += 1;
            if spec.state == TaskState::Done {
                done += 1;
            }
        }
        views.push(MilestoneView {
            wave: Some(milestone.id.clone()),
            title: milestone.title.clone(),
            done,
            total,
        });
    }
    let ungrouped: Vec<&String> = ledger
        .tasks
        .keys()
        .filter(|id| !claimed.iter().any(|(task, _)| *task == id.as_str()))
        .collect();
    if !ungrouped.is_empty() {
        let done = ungrouped
            .iter()
            .filter(|id| ledger.tasks[id.as_str()].state == TaskState::Done)
            .count();
        views.push(MilestoneView {
            wave: None,
            title: "未分组".to_owned(),
            done,
            total: ungrouped.len(),
        });
    }
    (views, warnings)
}

/// 台账 → 里程碑(v1 合成):`wave` / `title` 原样携带,`done` 数由任务状态聚合。
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

/// 速度线吞吐(W3-004 入模,W3-006 改 span 数据源):done 任务数 /
/// 活动窗小时数。span 取值:事件活动窗优先(`event_span_secs` 为 `Some`
/// 且 > 0,即 events.jsonl ≥2 条不同 ts 的 max − min,含 gate/agent/tool
/// 全部事件);否则回退任务 `since` 跨度(max − min,不可解析/缺失的
/// 时间戳不参与,全不可解析则无跨度)。生效条件:≥2 里程碑在场**且**
/// 所选跨度严格大于 0;不满足返回 [`None`]——渲染层据此隐藏速度行
/// (不虚报)。分子恒为里程碑 done 合计(含未分组尾)。
#[must_use]
pub fn velocity(
    tasks: &[TaskView],
    milestones: &[MilestoneView],
    event_span_secs: Option<u64>,
) -> Option<f64> {
    if milestones.len() < 2 {
        return None;
    }
    // W3-006:span 数据源选择——合格事件活动窗优先;缺失/零宽(单条、
    // 全同刻)回退任务 `since` 跨度(原口径,`None` 语义不变)
    let span_secs = match event_span_secs {
        Some(secs) if secs > 0 => secs,
        _ => {
            let stamps: Vec<u64> = tasks
                .iter()
                .filter_map(|task| task.since.as_deref())
                .filter_map(rfc3339_to_secs)
                .collect();
            let min = stamps.iter().copied().min()?;
            let max = stamps.iter().copied().max()?;
            max.saturating_sub(min)
        }
    };
    if span_secs == 0 {
        return None;
    }
    let done: usize = milestones.iter().map(|milestone| milestone.done).sum();
    let hours = f64::from(u32::try_from(span_secs).ok()?) / 3_600.0;
    // done 经 u32 折算入 f64(里程碑计数在 u32 域内,f64 精度足够展示)
    let done = f64::from(u32::try_from(done).unwrap_or(u32::MAX));
    (hours > 0.0).then(|| done / hours)
}

/// 项目名(D3,W3-004):git 仓根目录名 → cwd 目录名 → `"agentdash"`。
/// 原 `render::project_label` 的回退链上收模型,渲染层只读 [`Dashboard::project`]。
fn project_of(git: &GitFacts) -> String {
    git.root
        .as_deref()
        .and_then(dir_name)
        .or_else(|| std::env::current_dir().ok().and_then(|cwd| dir_name(&cwd)))
        .unwrap_or_else(|| "agentdash".to_owned())
}

/// 路径末段目录名;无末段(根路径等)为 [`None`],非 UTF-8 lossy 降级。
fn dir_name(path: impl AsRef<Path>) -> Option<String> {
    path.as_ref()
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
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
                done_at: None,
            }
        })
        .collect()
}

/// 本地 ISO8601 秒级 ts(带时区偏移,如 `2026-09-13T21:00:00+08:00`)。
/// W3-004 发现 9 起为 `Dashboard::generated_at` 的唯一来源(与事件 `ts`
/// 同格式,页眉不再 UTC/本地并存);hook 的事件打点也经此(hook.rs 委托)。
pub(crate) fn ts_now() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    let offset = local_utc_offset(now);
    format_ts(now + offset, offset)
}

/// (本地纪元秒, 偏移秒) → `YYYY-MM-DDTHH:MM:SS±HH:MM`。偏移 0 输出 `+00:00`
/// (承 Python `isoformat` 语义)。
fn format_ts(local_secs: i64, offset_secs: i64) -> String {
    let days = local_secs.div_euclid(86_400);
    let day_secs = local_secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (sign, off) = if offset_secs < 0 {
        ("-", -offset_secs)
    } else {
        ("+", offset_secs)
    };
    format!(
        "{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}{sign}{oh:02}:{om:02}",
        h = day_secs / 3_600,
        m = (day_secs % 3_600) / 60,
        s = day_secs % 60,
        oh = off / 3_600,
        om = (off % 3_600) / 60,
    )
}

/// 天序数(1970-01-01 = 0)→ (年, 月, 日)。Hinnant 算法,常规日期域内无溢出。
/// `pub(crate)` 供 W4-005 性质测试(与 [`days_from_civil`] 互逆)。
pub(crate) fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(windows)]
/// (年, 月, 日) → 天序数。仅 Windows 偏移差分(GetLocalTime/GetSystemTime)使用。
/// `pub(crate)` 供 W4-005 性质测试(与 [`civil_from_days`] 互逆)。
pub(crate) fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 }; // [0, 11]
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(windows)]
/// 本地 UTC 偏移秒:kernel32 `GetLocalTime` 与 `GetSystemTime` 同瞬时差分
/// (自动含 DST;按分钟取整吸收两次取时之间的秒级间隙)。
fn local_utc_offset(_utc: i64) -> i64 {
    #[repr(C)]
    #[derive(Default)]
    struct SysTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        millis: u16,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocalTime(out: *mut SysTime);
        fn GetSystemTime(out: *mut SysTime);
    }
    let pseudo = |st: &SysTime| {
        days_from_civil(i64::from(st.year), i64::from(st.month), i64::from(st.day)) * 86_400
            + i64::from(st.hour) * 3_600
            + i64::from(st.minute) * 60
            + i64::from(st.second)
    };
    // SAFETY:两 API 均只向调用方提供的单个 SYSTEMTIME(8×u16)写入,无失败路径;
    // 指针指向本栈帧内变量,调用期间线程不被重入。
    unsafe {
        let mut local = SysTime::default();
        let mut system = SysTime::default();
        GetLocalTime(std::ptr::addr_of_mut!(local));
        GetSystemTime(std::ptr::addr_of_mut!(system));
        let drift = pseudo(&local) - pseudo(&system);
        (drift + 30).div_euclid(60) * 60
    }
}

#[cfg(unix)]
/// 本地 UTC 偏移秒:libc `localtime_r` 的 `tm_gmtoff`(`struct tm` 布局由 libc
/// crate 保证,glibc/musl/macOS 与非 LP64 全覆盖,不再手搓布局)。
fn local_utc_offset(utc: i64) -> i64 {
    // SAFETY:`localtime_r` 线程安全(结果只写入调用方缓冲 `tm`,不触碰静态区);
    // `t` 与 `tm` 均为本栈帧内变量,调用期间线程不被重入。仅读取 `tm_gmtoff`。
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        // try_into 而非直接赋值:32 位 glibc 目标 `time_t` = i32,i64 直接赋值
        // E0308;LP64 下为恒等转换,永不走 fallback(超界时间戳退化为 epoch)。
        #[allow(clippy::useless_conversion)] // LP64 恒等转换被该 lint 误报
        let t: libc::time_t = utc.try_into().unwrap_or(0);
        // &raw 显式化:CI stable(1.98)clippy borrow_as_ptr 拒绝隐式借用转裸指针
        if libc::localtime_r(&raw const t, &raw mut tm).is_null() {
            return 0;
        }
        if tm.tm_gmtoff.abs() >= 86_400 {
            return 0; // 离谱偏移按损坏处理,退化 UTC
        }
        tm.tm_gmtoff as i64
    }
}

#[cfg(not(any(windows, unix)))]
/// 未知平台:退化为 UTC 偏移。
fn local_utc_offset(_utc: i64) -> i64 {
    0
}

/// 纯函数:RFC 3339 串 → Unix 纪元秒(Hinnant civil 逆变换,与
/// [`utc_timestamp`] / [`format_ts`] 互逆)。兼容三形态:`…Z`(UTC)、
/// `…±HH:MM`(本地偏移)与 `…±HHMM`(基本格式,`date +%z` 等,W6-001),
/// 偏移按绝对时刻折算。形态不符、字段越界或年份为 0 返回 [`None`]。
#[must_use]
pub(crate) fn rfc3339_to_secs(text: &str) -> Option<u64> {
    // 前缀 `YYYY-MM-DDTHH:MM:SS` 逐字段解析(两形态共用)
    let head = text.get(..19)?;
    let bytes = head.as_bytes();
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let field = |range: std::ops::Range<usize>| head.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (field(0..4)?, field(5..7)?, field(8..10)?);
    let (hour, minute, second) = (field(11..13)?, field(14..16)?, field(17..19)?);
    if year < 1
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return None;
    }
    // 尾缀:`Z`(UTC)、`±HH:MM`(扩展)或 `±HH:MM` 无冒号的 `±HHMM`(基本
    // 格式,date +%z 等,W6-001),偏移折算为绝对时刻
    let tail = text.get(19..)?;
    let offset_secs = match tail.as_bytes() {
        [b'Z'] => 0,
        [sign, rest @ ..] if rest.len() == 4 || (rest.len() == 5 && rest[2] == b':') => {
            let (h1, h2, m1, m2) = if rest.len() == 5 {
                (rest[0], rest[1], rest[3], rest[4])
            } else {
                (rest[0], rest[1], rest[2], rest[3])
            };
            offset_seconds(*sign, h1, h2, m1, m2)?
        }
        _ => return None,
    };
    let (year, month) = if month <= 2 {
        (year - 1, month + 12)
    } else {
        (year, month)
    };
    let era = year / 400;
    let yoe = year - era * 400;
    let doy = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hour * 3_600 + minute * 60 + second - offset_secs;
    u64::try_from(secs).ok()
}

/// 偏移尾缀折算(W6-001):(符号, 时 hh, 分 mm)→ 偏移秒;越界(时>23 /
/// 分>59)拒解析(不臆造)。
fn offset_seconds(sign: u8, h1: u8, h2: u8, m1: u8, m2: u8) -> Option<i64> {
    let parse = |pair: [u8; 2]| {
        std::str::from_utf8(&pair)
            .ok()?
            .parse::<i64>()
            .ok()
            .filter(|value| (0..=99).contains(value))
    };
    let (off_hour, off_min) = (parse([h1, h2])?, parse([m1, m2])?);
    if !(0..=23).contains(&off_hour) || !(0..=59).contains(&off_min) {
        return None;
    }
    let magnitude = off_hour * 3_600 + off_min * 60;
    Some(if sign == b'-' { -magnitude } else { magnitude })
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
