//! `events.jsonl` 事件流重放(W1-003)。
//!
//! 一行一 JSON(spec §4.2):`gate` / `agent` / `tool` 三类事件,hook 追加写入。
//! 重放语义承 state.jsonl:身份首见、标签配对;同一 gate 后到状态覆盖先到;
//! `ts` 保留原串不做时区运算,乱序容忍 = 后到事件按到达序处理。
//! 残缺行(非合法 JSON / 缺关键字段 / 未知 kind)一律丢弃并收集警告,绝不中断重放。

// 渲染/源接线在后续车道接入 main.rs;在那之前 bin 目标视本模块为死代码。
#![allow(dead_code)]

use std::collections::HashMap;

use serde::Deserialize;

/// 验证门终态:同 gate 后到事件覆盖先到。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateState {
    Running,
    Passed { detail: String },
    Failed { detail: String },
}

/// 仍活跃的子代理:`dispatched` 首见入表,`completed` 按 (who, task) 配对移除。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEntry {
    pub who: String,
    pub task: String,
    /// 首次 `dispatched` 事件的 `ts` 原串(缺省为空串)。
    pub first_seen: String,
}

/// 重放产出的模型。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventModel {
    /// 活跃 agent 表,按 `dispatched` 首见序排列。
    pub agents: Vec<AgentEntry>,
    /// gate 终态(后态覆盖前态)。
    pub gates: HashMap<String, GateState>,
    /// tool 事件计数(按 `tool` 名逐行累加)。
    pub tools: HashMap<String, u64>,
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
    let state = match raw.state.as_deref() {
        Some("running") => GateState::Running,
        Some("passed") => GateState::Passed {
            detail: raw.detail.unwrap_or_default(),
        },
        Some("failed") => GateState::Failed {
            detail: raw.detail.unwrap_or_default(),
        },
        _ => {
            model.warnings.push(format!(
                "line {line_no}: gate event with unknown or missing `state`, line dropped"
            ));
            return;
        }
    };
    // 后态覆盖前态
    model.gates.insert(gate, state);
}

fn apply_agent(model: &mut EventModel, raw: RawEvent, line_no: usize) {
    let (Some(who), Some(task)) = (raw.who, raw.task) else {
        model.warnings.push(format!(
            "line {line_no}: agent event with missing `who`/`task`, line dropped"
        ));
        return;
    };
    match raw.event.as_deref() {
        Some("dispatched") => {
            // 首见:已在表中则保留原 first_seen,不重复入表
            if !model.agents.iter().any(|a| a.who == who && a.task == task) {
                model.agents.push(AgentEntry {
                    who,
                    task,
                    first_seen: raw.ts.unwrap_or_default(),
                });
            }
        }
        Some("completed") => model.agents.retain(|a| !(a.who == who && a.task == task)),
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
    let count = model.tools.entry(tool).or_default();
    *count = count.saturating_add(1);
}
