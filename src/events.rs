//! `events.jsonl` 事件流重放(W1-003)。
//!
//! 一行一 JSON(spec §4.2):`gate` / `agent` / `tool` 三类事件,hook 追加写入。
//! 重放语义承 state.jsonl:身份首见、gate 后到状态覆盖先到;agent 以 `who` 为主键,
//! `task` 是可选注记(宿主 `SubagentStop` 载荷天然无 task,hook 侧只发 who)。
//! `ts` 保留原串不做时区运算,乱序容忍 = 后到事件按到达序处理。
//! 残缺行(非合法 JSON / 缺关键字段 / 未知 kind)一律丢弃并收集警告,绝不中断重放。

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use crate::model::rfc3339_to_secs;

/// 无 host 戳事件的归桶键(W11-004 stats):诚实标注"不知道归属",不猜。
pub const UNKNOWN_HOST: &str = "unknown";

/// 门终态折叠计数(W11-004 stats):state passed/failed 的门事件按
/// (门名 × 宿主)各一桶;`unknown` 收 exit 不可知的失败折叠(W4-001 D1 的
/// `(exit unknown)` 路径)——证据缺失不与真失败混计。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GateFoldTally {
    /// 折叠为 passed 的门事件数。
    pub passed: u64,
    /// 折叠为 failed 且带显式退出码的门事件数。
    pub failed: u64,
    /// 折叠为 failed 但 exit 不可知(null/缺字段)的门事件数。
    pub unknown: u64,
}

/// 验证门终态:同 gate 后到事件覆盖先到。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateState {
    Running,
    Passed {
        detail: String,
    },
    /// `unknown`:折叠无退出码证据(W4-001 D1 的 exit=null 路径)——
    /// 渲染层据此分第三态(W12-009),真失败与无证据视觉可辨。
    Failed {
        detail: String,
        unknown: bool,
    },
}

/// 仍活跃的子代理:`dispatched` 按 `who` 首见入表(同 who 再派刷新 task 注记,
/// host 保留首见不覆盖),`completed` 按 `who` 移除;`task`/`host` 为可选注记
/// (host = 宿主归属,W7-001),有则带入显示,无则 [`None`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEntry {
    pub who: String,
    pub task: Option<String>,
    /// 首次 `dispatched` 事件的 `ts` 原串(缺省为空串)。
    pub first_seen: String,
    /// 首次 `dispatched` 事件的宿主归属(`--host` 盖章;缺省 [`None`])。
    pub host: Option<String>,
    /// W11-003:被无 `who` 的 completed **推断配对**为完成——配对只是 FIFO
    /// 启发,非实测配对,故显式标记供显示层标注(`▶⇢✓ … (inferred)`),
    /// 计数走完成侧不占在跑(显示真相,不是新状态);同 who 再派即复活
    /// 置回 `false`。严格路径(`--no-infer`)恒 `false`。
    pub inferred: bool,
}

/// 事件尾容量(W4-002):详情面板只看最近 10 条,超限截旧。
const TAIL_CAP: usize = 10;

/// 事件尾条目(W4-002):已生效 agent/gate 事件的紧凑事实,供详情面板渲染。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailEntry {
    /// 事件类:`agent` / `gate`。
    pub kind: String,
    /// 主体名:agent 为 `who`,gate 为门名。
    pub name: String,
    /// 事件词:gate 取 `state`(running/passed/failed),agent 取 `event`
    /// (dispatched/completed)。
    pub state: String,
    /// `ts` 原串(缺省为空串)。
    pub ts: String,
}

/// 重放产出的模型。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventModel {
    /// 活跃 agent 表,按 `dispatched` 首见序排列。
    pub agents: Vec<AgentEntry>,
    /// gate 终态(后态覆盖前态)。
    pub gates: HashMap<String, GateState>,
    /// tool 事件计数(仅 `end` 相位计数,按 `tool` 名累加)。
    pub tools: HashMap<String, u64>,
    /// 详情面板事件尾(W4-002):已生效 agent/gate 事件紧凑行,到达序,
    /// 最近 [`TAIL_CAP`] 条(超限截旧);`tool` 事件与残缺行不入尾。
    pub tail: Vec<TailEntry>,
    /// 对账锚(W5-001):最近一次 passed gate 的 `ts` 原串(每次 passed 覆盖,
    /// 全窗有效——不走 tail,容量会滚掉旧门);从未有 passed → [`None`]。
    pub last_gate_passed: Option<String>,
    /// 事件流活动窗 ts 极值(W3-006,纪元秒):全部已知 kind(gate/agent/
    /// tool)事件的可解析 `ts` 最小/最大值;合格窗判定(max > min)收口在
    /// model 侧,这里只存极值事实。
    pub ts_min: Option<u64>,
    pub ts_max: Option<u64>,
    /// 残缺行警告(格式 `line {n}: ...`,行号从 1 起计,空行不计)。
    pub warnings: Vec<String>,
    /// 宿主使用率底账(W11-004 stats):**生效**事件按行上 `host` 戳计数,
    /// 无戳/空白戳归 [`UNKNOWN_HOST`] 桶。生效 = replay 消费的行:gate 合法
    /// 行、agent dispatched/completed 生效行(含推断配对)、tool `end` 相位
    /// 行;残缺行与未知 kind 不计(tools 计数同口径:仅 end 相位)。
    pub host_events: BTreeMap<String, u64>,
    /// 门终态折叠计数(W11-004 stats):(门名, 宿主) → 各态计数。running
    /// 在途行与残缺行不计;failed 折叠按行上 `exit` 证据分流(有码 →
    /// `failed`,无码 → `unknown`)。折叠本身仍由本模型 `gates` 独家产出,
    /// 这里只对折叠产物计数投影,不重算折叠。
    pub gate_folds: BTreeMap<(String, String), GateFoldTally>,
}

/// 一行一 JSON 的宽松事件形状:字段类型不符即整行判残缺。
#[derive(Deserialize)]
struct RawEvent {
    kind: String,
    ts: Option<String>,
    gate: Option<String>,
    state: Option<String>,
    detail: Option<String>,
    event: Option<String>,
    phase: Option<String>,
    task: Option<String>,
    who: Option<String>,
    host: Option<String>,
    tool: Option<String>,
    /// W11-004 stats:门折叠行的退出码证据(hook 落盘为数字或 `null`;
    /// running 行无此字段)。`null`/缺失 = exit 不可知。
    exit: Option<serde_json::Value>,
}

/// 重放事件流:按到达序逐行折叠出模型(缺省开启无 who completed 配对
/// 启发,W11-003)。
#[must_use]
pub fn replay(lines: impl Iterator<Item = String>) -> EventModel {
    replay_infer(lines, true)
}

/// [`replay`] 的旗标变体(W11-003):`infer = false` 关闭配对启发,无
/// `who` 的 completed 一律严格丢弃 + 逐行警告(`--no-infer`,与启发落地
/// 前行为逐字节一致);`infer = true` 时配对成功的逐行警告降频为回放
/// 收尾的一条汇总。
#[must_use]
pub fn replay_infer(lines: impl Iterator<Item = String>, infer: bool) -> EventModel {
    let mut model = EventModel::default();
    let mut inferred_pairs = 0usize;
    for (idx, line) in lines.enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim();
        // 空行(含文件末尾换行)静默跳过,不算残缺
        if trimmed.is_empty() {
            continue;
        }
        let Ok(raw) = serde_json::from_str::<RawEvent>(trimmed) else {
            model
                .warnings
                .push(format!("line {line_no}: invalid JSON, line dropped"));
            continue;
        };
        // W3-006:活动窗极值折叠——全部已知 kind(gate/agent/tool)事件的
        // `ts` 都是会话活动证据,即便该行因缺关键字段被按残缺丢弃;ts 缺失
        // 或不可解析、未知 kind 与残缺 JSON 行不参与。这里只存极值事实,
        // 「≥2 条不同 ts」的合格窗判定(max > min)收口在 model 侧。
        if matches!(raw.kind.as_str(), "gate" | "agent" | "tool")
            && let Some(secs) = raw.ts.as_deref().and_then(rfc3339_to_secs)
        {
            model.ts_min = Some(model.ts_min.map_or(secs, |min| min.min(secs)));
            model.ts_max = Some(model.ts_max.map_or(secs, |max| max.max(secs)));
        }
        match raw.kind.as_str() {
            "gate" => apply_gate(&mut model, raw, line_no),
            "agent" => apply_agent(&mut model, raw, line_no, infer, &mut inferred_pairs),
            "tool" => apply_tool(&mut model, raw, line_no),
            other => model.warnings.push(format!(
                "line {line_no}: unknown event kind `{other}`, line dropped"
            )),
        }
    }
    // 警告降频(W11-003):凡发生推断配对,配对成功的逐行警告一律不发,
    // 回放收尾一条汇总(未配对的逐行警告照常在各自到达位)
    if inferred_pairs > 0 {
        model
            .warnings
            .push(format!("{inferred_pairs} 个无 who completed 已推断配对"));
    }
    model
}

fn apply_gate(model: &mut EventModel, raw: RawEvent, line_no: usize) {
    let Some(gate) = raw.gate else {
        model.warnings.push(format!(
            "line {line_no}: gate event with missing `gate`, line dropped"
        ));
        return;
    };
    let (label, state) = match raw.state.as_deref() {
        Some("running") => ("running", GateState::Running),
        Some("passed") => (
            "passed",
            GateState::Passed {
                detail: raw.detail.unwrap_or_default(),
            },
        ),
        Some("failed") => (
            "failed",
            GateState::Failed {
                detail: raw.detail.unwrap_or_default(),
                // W12-009:折叠无退出码证据(exit=null,W4-001 D1 路径)→
                // unknown 置位,渲染层分第三态
                unknown: !matches!(raw.exit, Some(serde_json::Value::Number(_))),
            },
        ),
        _ => {
            model.warnings.push(format!(
                "line {line_no}: gate event with unknown or missing `state`, line dropped"
            ));
            return;
        }
    };
    let ts = raw.ts;
    // 后态覆盖前态;生效行入事件尾(W4-002)
    model.gates.insert(gate.clone(), state);
    // W11-004 stats:生效门事件计入宿主使用率;终态折叠(passed/failed)
    // 另按(门名, host)累计——failed 折叠按 exit 证据分流(有码 failed,
    // 无码 unknown 即 W4-001 D1 的 `(exit unknown)` 路径);running 在途不算折叠
    let host = host_key(raw.host.as_deref());
    *model.host_events.entry(host.clone()).or_default() += 1;
    if label != "running" {
        let tally = model.gate_folds.entry((gate.clone(), host)).or_default();
        if label == "passed" {
            tally.passed += 1;
        } else if matches!(raw.exit, Some(serde_json::Value::Number(_))) {
            tally.failed += 1;
        } else {
            tally.unknown += 1;
        }
    }
    push_tail(
        &mut model.tail,
        "gate",
        gate,
        label,
        ts.clone().unwrap_or_default(),
    );
    // 对账锚(W5-001):最近 passed gate 的 ts(无 ts 的 passed 销毁既有锚——
    // 当前证据说不了谎,也不借旧证)
    if label == "passed" {
        model.last_gate_passed = ts;
    }
}

/// 事件尾追加(W4-002):到达序 push,超 [`TAIL_CAP`] 截最旧。
fn push_tail(tail: &mut Vec<TailEntry>, kind: &str, name: String, state: &str, ts: String) {
    tail.push(TailEntry {
        kind: kind.to_owned(),
        name,
        state: state.to_owned(),
        ts,
    });
    if tail.len() > TAIL_CAP {
        tail.remove(0);
    }
}

fn apply_agent(
    model: &mut EventModel,
    raw: RawEvent,
    line_no: usize,
    infer: bool,
    inferred_pairs: &mut usize,
) {
    // W11-004 stats:宿主键归一先行(AgentEntry.host 仍存行上原值不动);
    // 生效 dispatched/completed(含推断配对)各计一次使用率,残缺行不计
    let host = host_key(raw.host.as_deref());
    let Some(who) = raw.who else {
        // W11-003:无 who 的 completed 在启发开启时配给最老在跑;其余缺 who
        // 形态(dispatched / 未知 event / 启发关闭)照旧残缺丢弃
        if infer && raw.event.as_deref() == Some("completed") {
            infer_pair(model, raw, line_no, inferred_pairs, &host);
        } else {
            model.warnings.push(format!(
                "line {line_no}: agent event with missing `who`, line dropped"
            ));
        }
        return;
    };
    match raw.event.as_deref() {
        Some("dispatched") => {
            // 生效行先入尾(W4-002);who 主键:已在表中则刷新 task 注记
            // (first_seen 保留首见),否则新条目入表;task 缺省记 None
            // (宿主 SubagentStop 载荷天然无 task)
            push_tail(
                &mut model.tail,
                "agent",
                who.clone(),
                "dispatched",
                raw.ts.clone().unwrap_or_default(),
            );
            match model.agents.iter_mut().find(|a| a.who == who) {
                // host 保留首见(D2:再派刷新 task 注记不改归属);
                // 再派复活(W11-003):推断完成行回到真在跑
                Some(entry) => {
                    entry.task = raw.task;
                    entry.inferred = false;
                }
                None => model.agents.push(AgentEntry {
                    who,
                    task: raw.task,
                    first_seen: raw.ts.unwrap_or_default(),
                    host: raw.host,
                    inferred: false,
                }),
            }
            *model.host_events.entry(host).or_default() += 1;
        }
        // 按 who 移除(与 task 注记无关);未在册的 who(幽灵)静默忽略。
        // 生效行入尾(W4-002)
        Some("completed") => {
            push_tail(
                &mut model.tail,
                "agent",
                who.clone(),
                "completed",
                raw.ts.unwrap_or_default(),
            );
            model.agents.retain(|a| a.who != who);
            *model.host_events.entry(host).or_default() += 1;
        }
        _ => model.warnings.push(format!(
            "line {line_no}: agent event with unknown or missing `event`, line dropped"
        )),
    }
}

/// 无 who 的 completed 配对启发(W11-003):配给**最老在跑**行(表首即
/// 派发首见序,FIFO;已推断完成的行跳过,不再吃配对)。配对行标
/// [`AgentEntry::inferred`](= 完成,不占在跑),事件尾以被配对 who 记一笔
/// 生效 completed(匿名行由此落到具体 agent 的叙事序);无在跑可配 →
/// 保持今天的逐行丢弃警告(汇总由 [`replay_infer`] 收尾统一出)。
/// `host` = 匿名行自身宿主键(W11-004 stats:配对成功计一次使用率)。
fn infer_pair(
    model: &mut EventModel,
    raw: RawEvent,
    line_no: usize,
    inferred_pairs: &mut usize,
    host: &str,
) {
    let Some(entry) = model.agents.iter_mut().find(|agent| !agent.inferred) else {
        model.warnings.push(format!(
            "line {line_no}: agent event with missing `who`, line dropped"
        ));
        return;
    };
    entry.inferred = true;
    *inferred_pairs += 1;
    *model.host_events.entry(host.to_owned()).or_default() += 1;
    push_tail(
        &mut model.tail,
        "agent",
        entry.who.clone(),
        "completed",
        raw.ts.unwrap_or_default(),
    );
}

fn apply_tool(model: &mut EventModel, raw: RawEvent, line_no: usize) {
    let Some(tool) = raw.tool else {
        model.warnings.push(format!(
            "line {line_no}: tool event with missing `tool`, line dropped"
        ));
        return;
    };
    // 计数口径钉死:仅 `end` 相位计数;`start` 相位静默忽略;
    // 缺相 / 未知相按残缺行丢弃 + 警告
    match raw.phase.as_deref() {
        Some("end") => {
            // W11-004 stats:生效 tool 行计入宿主使用率(与 tools 同口径)
            let host = host_key(raw.host.as_deref());
            *model.host_events.entry(host).or_default() += 1;
            let count = model.tools.entry(tool).or_default();
            *count = count.saturating_add(1);
        }
        Some("start") => {}
        _ => model.warnings.push(format!(
            "line {line_no}: tool event with unknown or missing `phase`, line dropped"
        )),
    }
}

/// 宿主键归一(W11-004 stats):行上有非空 `host` 戳即原样作键;缺失/空白
/// 归 [`UNKNOWN_HOST`] 桶(诚实标注,不猜归属)。
fn host_key(host: Option<&str>) -> String {
    match host.map(str::trim).filter(|h| !h.is_empty()) {
        Some(host) => host.to_owned(),
        None => UNKNOWN_HOST.to_owned(),
    }
}
