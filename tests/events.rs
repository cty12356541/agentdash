//! `events.jsonl` 重放的集成测试(W1-003)。
//!
//! crate 是纯二进制(无 lib 目标),集成测试用 `#[path]` 直接引入被测模块。
//! W3-006 起事件流折出活动窗极值,解析器取自 model(与 `tests/merge.rs`
//! 同约定挂载模块树,使 events.rs 内的 `crate::model` 路径照常解析);
//! 本组用例只触达重放/极值,挂载树的其余 pub 项属死代码,按文件级 allow
//! 放行(同 `tests/render_panel.rs` 约定)。

#![allow(dead_code)]

#[path = "../src/contract.rs"]
mod contract;
#[path = "../src/events.rs"]
mod events;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/sources/mod.rs"]
mod sources;

use std::collections::HashMap;

use events::{AgentEntry, EventModel, GateState};

fn replay_strs(lines: &[&str]) -> EventModel {
    events::replay(lines.iter().map(ToString::to_string))
}

#[test]
fn gate_states_fold_last_write_wins() {
    let model = replay_strs(&[
        // 同 gate running→passed 折叠
        r#"{"ts":"T1","kind":"gate","gate":"cargo-test","state":"running"}"#,
        r#"{"ts":"T2","kind":"gate","gate":"cargo-test","state":"passed","detail":"298 passed / 0 failed"}"#,
        // 乱序 ts 容忍:后到事件按到达处理,ts 更早的 failed 仍覆盖 passed
        r#"{"ts":"T9","kind":"gate","gate":"cargo-test","state":"failed","detail":"3 failed"}"#,
        // 只 running 未出终态的门保持 Running
        r#"{"ts":"T1","kind":"gate","gate":"cargo-clippy","state":"running"}"#,
        // 无 detail 的终态
        r#"{"ts":"T2","kind":"gate","gate":"cargo-fmt","state":"passed"}"#,
    ]);
    let mut expect = HashMap::new();
    expect.insert(
        "cargo-test".to_string(),
        GateState::Failed {
            detail: "3 failed".to_string(),
        },
    );
    expect.insert("cargo-clippy".to_string(), GateState::Running);
    expect.insert(
        "cargo-fmt".to_string(),
        GateState::Passed {
            detail: String::new(),
        },
    );
    assert_eq!(model.gates, expect);
    assert!(model.warnings.is_empty());
}

#[test]
fn agent_dispatch_completed_pairing() {
    let model = replay_strs(&[
        // ts 原串保留,不做时区运算
        r#"{"ts":"2026-09-13T21:00:00+08:00","kind":"agent","event":"dispatched","task":"1","who":"implementer-1"}"#,
        r#"{"ts":"2026-09-13T21:01:00+08:00","kind":"agent","event":"dispatched","task":"2","who":"reviewer-1"}"#,
        // 同 who 第二次 dispatched:who 主键,刷新 task 注记,不新建条目,first_seen 保留首见
        r#"{"ts":"2026-09-13T21:02:00+08:00","kind":"agent","event":"dispatched","task":"3","who":"implementer-1"}"#,
        // completed 按 who 配对移除(与 task 注记无关)
        r#"{"ts":"2026-09-13T21:03:00+08:00","kind":"agent","event":"completed","who":"reviewer-1"}"#,
        // completed 未知 agent(幽灵):忽略且不告警
        r#"{"ts":"2026-09-13T21:04:30+08:00","kind":"agent","event":"completed","task":"9","who":"ghost"}"#,
        // 完成后再次 dispatched:作为新首见入表(按到达序排在表尾)
        r#"{"ts":"2026-09-13T21:05:00+08:00","kind":"agent","event":"dispatched","task":"2","who":"reviewer-1"}"#,
    ]);
    assert_eq!(
        model.agents,
        vec![
            AgentEntry {
                who: "implementer-1".to_string(),
                task: Some("3".to_string()),
                first_seen: "2026-09-13T21:00:00+08:00".to_string(),
            },
            AgentEntry {
                who: "reviewer-1".to_string(),
                task: Some("2".to_string()),
                first_seen: "2026-09-13T21:05:00+08:00".to_string(),
            },
        ]
    );
    assert!(model.warnings.is_empty());
}

#[test]
fn hook_agent_line_with_only_who_lands_in_active_table() {
    // 宿主 SubagentStop 载荷天然无 task,hook 侧只发 who:
    // 只有 who 的 agent 行重放后必须出现在活跃表(task 记 None)
    let model = replay_strs(&[
        r#"{"ts":"2026-09-13T21:00:00+08:00","kind":"agent","event":"dispatched","who":"implementer-1"}"#,
        r#"{"kind":"agent","event":"dispatched","who":"reviewer-1"}"#,
        // 且只有 who 的 completed 能按 who 正常配对移除
        r#"{"ts":"2026-09-13T21:03:00+08:00","kind":"agent","event":"completed","who":"reviewer-1"}"#,
    ]);
    assert_eq!(
        model.agents,
        vec![AgentEntry {
            who: "implementer-1".to_string(),
            task: None,
            first_seen: "2026-09-13T21:00:00+08:00".to_string(),
        }]
    );
    assert!(model.warnings.is_empty());
}

#[test]
fn gate_missing_name_dropped_with_warning() {
    let model = replay_strs(&[
        r#"{"ts":"T1","kind":"gate","state":"passed","detail":"no gate name"}"#,
        // 残缺行不中断重放,后续合法行照常
        r#"{"ts":"T2","kind":"gate","gate":"cargo-test","state":"running"}"#,
    ]);
    assert!(model.gates.contains_key("cargo-test"));
    assert_eq!(
        model.warnings,
        vec!["line 1: gate event with missing `gate`, line dropped".to_string()]
    );
}

#[test]
fn agent_missing_who_dropped_with_warning() {
    let model = replay_strs(&[
        // task 不再是必填,缺 who 才判残缺
        r#"{"ts":"T1","kind":"agent","event":"dispatched","task":"1"}"#,
        // completed 同样必须有 who
        r#"{"ts":"T2","kind":"agent","event":"completed"}"#,
        // 残缺行不中断重放,后续合法行照常
        r#"{"ts":"T3","kind":"agent","event":"dispatched","who":"a"}"#,
    ]);
    assert_eq!(model.agents.len(), 1);
    assert_eq!(
        model.warnings,
        vec![
            "line 1: agent event with missing `who`, line dropped".to_string(),
            "line 2: agent event with missing `who`, line dropped".to_string(),
        ]
    );
}

#[test]
fn gate_unknown_state_dropped_with_warning() {
    let model = replay_strs(&[
        r#"{"ts":"T1","kind":"gate","gate":"cargo-test","state":"paused"}"#,
        // 未知 state 不入表,同一门后续合法状态照常折叠
        r#"{"ts":"T2","kind":"gate","gate":"cargo-test","state":"running"}"#,
    ]);
    assert_eq!(model.gates.get("cargo-test"), Some(&GateState::Running));
    assert_eq!(
        model.warnings,
        vec!["line 1: gate event with unknown or missing `state`, line dropped".to_string()]
    );
}

#[test]
fn tool_counts_only_end_phase_start_ignored() {
    let model = replay_strs(&[
        // start 相位:静默忽略,不计数
        r#"{"ts":"T1","kind":"tool","tool":"bash","phase":"start"}"#,
        // end 相位:计数
        r#"{"ts":"T2","kind":"tool","tool":"bash","phase":"end","exit":0,"summary":"ls"}"#,
        r#"{"ts":"T3","kind":"tool","tool":"bash","phase":"end","exit":1,"summary":"cargo test"}"#,
        r#"{"ts":"T4","kind":"tool","tool":"gh","phase":"start"}"#,
        r#"{"ts":"T5","kind":"tool","tool":"gh","phase":"end","exit":0,"summary":"pr checks"}"#,
        // 缺相:按残缺行丢弃 + 警告
        r#"{"ts":"T6","kind":"tool","tool":"bash","exit":0,"summary":"no phase"}"#,
    ]);
    let mut expect = HashMap::new();
    expect.insert("bash".to_string(), 2_u64);
    expect.insert("gh".to_string(), 1_u64);
    assert_eq!(model.tools, expect);
    assert_eq!(
        model.warnings,
        vec!["line 6: tool event with unknown or missing `phase`, line dropped".to_string()]
    );
}

#[test]
fn truncated_tail_line_dropped_with_warning() {
    let model = replay_strs(&[
        r#"{"ts":"T1","kind":"gate","gate":"g1","state":"running"}"#,
        r#"{"ts":"T2","kind":"agent","event":"dispatched","task":"1","who":"a"}"#,
        r#"{"kind":"tool","tool":"bash","phase":"end","exit":0,"summary":"ls"}"#,
        // 并发追加被截断的残缺尾行:丢弃 + 警告
        r#"{"ts":"2026-09-13T21:0"#,
        // 文件末尾换行产生的空行:静默跳过,不算残缺
        "",
    ]);
    assert_eq!(model.gates.get("g1"), Some(&GateState::Running));
    assert_eq!(model.agents.len(), 1);
    assert_eq!(model.tools.get("bash"), Some(&1));
    assert_eq!(
        model.warnings,
        vec!["line 4: invalid JSON, line dropped".to_string()]
    );
}

#[test]
fn malformed_middle_line_dropped_stream_continues() {
    let model = replay_strs(&[
        r#"{"ts":"T1","kind":"tool","tool":"bash","phase":"end","exit":0,"summary":"cargo test"}"#,
        // 中间坏行:丢弃 + 警告,但不影响后续行
        "not-json-at-all {{{",
        r#"{"ts":"T2","kind":"tool","tool":"bash","phase":"end","exit":1,"summary":"cargo test"}"#,
        r#"{"ts":"T3","kind":"tool","tool":"gh","phase":"end","exit":0,"summary":"pr checks"}"#,
        // 合法 JSON 但未知 kind:同样降级为警告(承 spec §9 精神)
        r#"{"ts":"T4","kind":"mystery","x":1}"#,
    ]);
    let mut expect = HashMap::new();
    expect.insert("bash".to_string(), 2_u64);
    expect.insert("gh".to_string(), 1_u64);
    assert_eq!(model.tools, expect);
    assert_eq!(
        model.warnings,
        vec![
            "line 2: invalid JSON, line dropped".to_string(),
            "line 5: unknown event kind `mystery`, line dropped".to_string(),
        ]
    );
}

/// W3-006:活动窗 ts 极值跨全部已知 kind(gate/agent/tool)折叠,按绝对
/// 时刻折算(偏移形态并存);已知 kind 但行残缺(如 gate 无名)ts 仍是
/// 真实活动证据,参与极值;ts 缺失/不可解析、未知 kind 与残缺 JSON 行
/// 不参与;单条事件两端同值(合格与否由 model 侧 max>min 判定)。
#[test]
fn ts_window_folds_min_max_over_known_kinds() {
    let model = replay_strs(&[
        // max 端:09:00:00Z(纪元 1_789_290_000)
        r#"{"ts":"2026-09-13T09:00:00Z","kind":"agent","event":"dispatched","who":"a"}"#,
        r#"{"ts":"2026-09-13T08:00:00Z","kind":"gate","gate":"cargo-fmt","state":"running"}"#,
        // 偏移形态折算:= 08:30Z,夹在中间不沾极值(误当 UTC 读会顶掉 max)
        r#"{"ts":"2026-09-13T10:30:00+02:00","kind":"tool","tool":"bash","phase":"end"}"#,
        // 已知 kind 但行残缺(gate 无名):min 端 07:30:00Z 必须参与
        r#"{"ts":"2026-09-13T07:30:00Z","kind":"gate","state":"passed"}"#,
        // 以下三类不参与:无 ts / 坏 ts / 未知 kind
        r#"{"kind":"gate","gate":"g2","state":"passed"}"#,
        r#"{"ts":"not-a-time","kind":"tool","tool":"bash","phase":"end"}"#,
        r#"{"ts":"2026-09-13T23:59:59Z","kind":"mystery","x":1}"#,
    ]);
    assert_eq!(
        model.ts_min,
        Some(1_789_284_600),
        "min = 2026-09-13T07:30:00Z(残缺 gate 行的 ts 也算活动)"
    );
    assert_eq!(
        model.ts_max,
        Some(1_789_290_000),
        "max = 2026-09-13T09:00:00Z(±HH:MM 偏移按绝对时刻折算)"
    );
    assert_eq!(model.ts_max.unwrap() - model.ts_min.unwrap(), 5_400);

    // 单条事件:两端同值(零宽窗,由 model 侧判不合格)
    let single = replay_strs(&[
        r#"{"ts":"2026-09-13T09:00:00Z","kind":"gate","gate":"g","state":"passed"}"#,
    ]);
    assert_eq!(single.ts_min, single.ts_max);
    assert_eq!(single.ts_min, Some(1_789_290_000));

    // 无事件:无窗
    let empty = replay_strs(&[]);
    assert_eq!(empty.ts_min, None);
    assert_eq!(empty.ts_max, None);
}

// ------------------------------------------------------------ 事件尾(W4-002)

#[test]
fn tail_keeps_effective_agent_and_gate_in_arrival_order() {
    let model = replay_strs(&[
        r#"{"kind":"tool","tool":"edit","phase":"end","ts":"2026-09-15T10:00:00+08:00"}"#,
        r#"{"kind":"gate","gate":"cargo-test","state":"running","ts":"2026-09-15T10:00:01+08:00"}"#,
        r#"{"kind":"agent","event":"dispatched","who":"explore","task":"找","ts":"2026-09-15T10:00:02+08:00"}"#,
        r#"{"kind":"agent","event":"completed","who":"explore","ts":"2026-09-15T10:00:03+08:00"}"#,
        r#"{"kind":"gate","gate":"cargo-test","state":"passed","detail":"ok","ts":"2026-09-15T10:00:04+08:00"}"#,
        "not json at all",
        r#"{"kind":"agent","event":"dispatched","ts":"2026-09-15T10:00:05+08:00"}"#,
    ]);
    let tail: Vec<_> = model
        .tail
        .iter()
        .map(|e| (e.kind.as_str(), e.name.as_str(), e.state.as_str()))
        .collect();
    assert_eq!(
        tail,
        [
            ("gate", "cargo-test", "running"),
            ("agent", "explore", "dispatched"),
            ("agent", "explore", "completed"),
            ("gate", "cargo-test", "passed"),
        ],
        "到达序生效行;tool 行与残缺行(非 JSON/缺 who)不入尾"
    );
}

#[test]
fn tail_caps_at_ten_dropping_oldest() {
    let lines: Vec<String> = (0..12)
        .map(|i| {
            format!(
                r#"{{"kind":"agent","event":"dispatched","who":"a{i:02}","ts":"2026-09-15T10:{i:02}:00+08:00"}}"#
            )
        })
        .collect();
    let strs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let model = replay_strs(&strs);
    assert_eq!(model.tail.len(), 10, "超容量截旧");
    assert_eq!(model.tail[0].name, "a02", "最旧两条(a00/a01)被截");
    assert_eq!(model.tail[9].name, "a11", "最新在尾");
}
