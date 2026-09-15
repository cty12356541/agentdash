//! `events.jsonl` 事件流重放(W1-003)。
//!
//! 一行一 JSON(spec §4.2):`gate` / `agent` / `tool` 三类事件,hook 追加写入。
//! 重放语义承 state.jsonl:身份首见、gate 后到状态覆盖先到;agent 以 `who` 为主键,
//! `task` 是可选注记(宿主 `SubagentStop` 载荷天然无 task,hook 侧只发 who)。
//! `ts` 保留原串不做时区运算,乱序容忍 = 后到事件按到达序处理。
//! 残缺行(非合法 JSON / 缺关键字段 / 未知 kind)一律丢弃并收集警告,绝不中断重放。

use std::collections::HashMap;

use serde::Deserialize;

use crate::model::rfc3339_to_secs;

/// 验证门终态:同 gate 后到事件覆盖先到。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateState {
    Running,
    Passed { detail: String },
    Failed { detail: String },
}

/// 仍活跃的子代理:`dispatched` 按 `who` 首见入表(同 who 再派刷新 task 注记,不新建条目),
/// `completed` 按 `who` 移除;`task` 为可选注记,有则带入显示,无则 [`None`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEntry {
    pub who: String,
    pub task: Option<String>,
    /// 首次 `dispatched` 事件的 `ts` 原串(缺省为空串)。
    pub first_seen: String,
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
    /// 事件流活动窗 ts 极值(W3-006,纪元秒):全部已知 kind(gate/agent/
    /// tool)事件的可解析 `ts` 最小/最大值;合格窗判定(max > min)收口在
    /// model 侧,这里只存极值事实。
    pub ts_min: Option<u64>,
    pub ts_max: Option<u64>,
    /// 残缺行警告(格式 `line {n}: ...`,行号从 1 起计,空行不计)。
    pub warnings: Vec<String>,
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
    tool: Option<String>,
}

/// 重放事件流:按到达序逐行折叠出模型。
#[must_use]
pub fn replay(lines: impl Iterator<Item = String>) -> EventModel {
    let mut model = EventModel::default();
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
            "agent" => apply_agent(&mut model, raw, line_no),
            "tool" => apply_tool(&mut model, raw, line_no),
            other => model.warnings.push(format!(
                "line {line_no}: unknown event kind `{other}`, line dropped"
            )),
        }
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
            },
        ),
        _ => {
            model.warnings.push(format!(
                "line {line_no}: gate event with unknown or missing `state`, line dropped"
            ));
            return;
        }
    };
    // 后态覆盖前态;生效行入事件尾(W4-002)
    model.gates.insert(gate.clone(), state);
    push_tail(
        &mut model.tail,
        "gate",
        gate,
        label,
        raw.ts.unwrap_or_default(),
    );
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

fn apply_agent(model: &mut EventModel, raw: RawEvent, line_no: usize) {
    let Some(who) = raw.who else {
        model.warnings.push(format!(
            "line {line_no}: agent event with missing `who`, line dropped"
        ));
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
                Some(entry) => entry.task = raw.task,
                None => model.agents.push(AgentEntry {
                    who,
                    task: raw.task,
                    first_seen: raw.ts.unwrap_or_default(),
                }),
            }
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
        }
        _ => model.warnings.push(format!(
            "line {line_no}: agent event with unknown or missing `event`, line dropped"
        )),
    }
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
            let count = model.tools.entry(tool).or_default();
            *count = count.saturating_add(1);
        }
        Some("start") => {}
        _ => model.warnings.push(format!(
            "line {line_no}: tool event with unknown or missing `phase`, line dropped"
        )),
    }
}
