//! 终端面板:区块 A 在跑/健康 → B 轨迹 → C 车道(W1-006,移植自 claude-dash
//! `dashlib/render_panel.py`)。ANSI 着色、按显示宽自适应,只读不落盘。

use std::collections::BTreeMap;

use super::{
    C_ACTIVE, C_BOLD, C_DONE, C_END, C_PENDING, C_STALLED, C_WARN, Visual, active_milestone,
    clamp_width, display_width, milestone_state, project_label, round_half_even, visual,
};
use crate::events::GateState;
use crate::model::{Dashboard, MilestoneView, TaskView};

/// 面板默认宽。
pub const DEFAULT_PANEL_WIDTH: usize = 64;

/// 面板:页眉统计 → 健康 → 轨迹 → 车道任务区。宽度先经 40..120 钳位。
#[must_use]
pub fn render_panel(dash: &Dashboard, width: usize) -> String {
    let width = clamp_width(width);
    let mut lines: Vec<String> = Vec::new();

    let mut done = 0;
    let mut running = 0; // active + stalled(▶ 计数承 Python口径)
    let mut stalled = 0;
    let mut resting = 0; // pending + blocked(· 计数)
    for task in &dash.tasks {
        match visual(task.state) {
            Visual::Done => done += 1,
            Visual::Active => running += 1,
            Visual::Stalled => {
                running += 1;
                stalled += 1;
            }
            Visual::Pending | Visual::Blocked => resting += 1,
        }
    }
    // TODO(model):Dashboard 无 activity(agent)字段,agent 计数 W1 恒 0;
    // 无 velocity(commits_7d)字段,吞吐段暂缺。
    let agents = 0;

    // 页眉:项目 · 活跃里程碑
    let active_ms = active_milestone(dash);
    match active_ms {
        Some(milestone) => lines.push(format!(
            "{} · {} {}",
            project_label(dash),
            ms_id(milestone),
            milestone.title
        )),
        None => lines.push(project_label(dash).to_owned()),
    }
    let clock = clock_slice(&dash.generated_at);
    lines.push(format!(
        "{C_DONE}✓{done}{C_END} {C_ACTIVE}▶{running}{C_END} \
         {C_PENDING}·{resting}{C_END} {C_STALLED}⚑{stalled}{C_END} \
         · {agents} agents · {clock}"
    ));
    lines.push("═".repeat(width));

    // 区块 A:在跑 / 健康
    lines.push(format!("{C_BOLD}在跑 / 健康{C_END}"));
    // TODO(model):无 activity / stalled 列表字段(在跑 agent、卡死任务名);
    // 健康区改列验证门终态(Dashboard.gates 为现有字段中最贴近项)。
    let gates: BTreeMap<&str, &GateState> = dash
        .gates
        .iter()
        .map(|(name, state)| (name.as_str(), state))
        .collect();
    for (name, state) in &gates {
        lines.push(gate_line(name, state));
    }
    if gates.is_empty() {
        lines.push(format!("{C_DONE}✓ 无活跃/卡死{C_END}"));
    }
    for warning in &dash.warnings {
        lines.push(format!("{C_WARN}⚠ {warning}{C_END}"));
    }
    lines.push("─".repeat(width));

    // 区块 B:轨迹
    let done_ms = dash
        .milestones
        .iter()
        .filter(|milestone| milestone_state(milestone) == "done")
        .count();
    lines.push(format!("{C_BOLD}轨迹 · {done_ms} 里程碑{C_END}"));
    for milestone in dash.milestones.iter().rev().take(5).rev() {
        lines.push(milestone_line(milestone, width));
    }
    let planned = dash
        .milestones
        .iter()
        .find(|milestone| milestone_state(milestone) == "planned");
    if let (Some(next), None) = (planned, active_ms) {
        lines.push(format!("  下一步:待启动 {}", ms_id(next)));
    } else if active_ms.is_some() {
        lines.push("  进行中".to_owned());
    } else {
        lines.push("  —".to_owned()); // 全部完成/全新仓库:中性占位,不虚报"进行中"
    }
    lines.push("─".repeat(width));

    // 区块 C:车道 / 任务
    if !dash.tasks.is_empty() {
        lines.push(format!("{C_BOLD}车道 / 任务{C_END}"));
        let mut lane_groups: Vec<(String, Vec<&TaskView>)> = Vec::new();
        for task in &dash.tasks {
            let name = task.lane.clone().unwrap_or_else(|| "无车道".to_owned());
            match lane_groups.iter_mut().find(|(group, _)| *group == name) {
                Some((_, members)) => members.push(task),
                None => lane_groups.push((name, vec![task])),
            }
        }
        for (name, members) in &lane_groups {
            lines.push(format!("{C_BOLD}{name}{C_END}"));
            for task in members {
                lines.push(task_line(task));
            }
        }
        // TODO(render):屏障行(claude-dash `  {barrier}`)——模型已携带 barriers
        // (T7 入模),面板侧渲染仍未做,待后续车道
    }
    lines.join("\n")
}

/// 里程碑显示 id(台账 wave;未声明时占位 `-`)。
fn ms_id(milestone: &MilestoneView) -> &str {
    milestone.wave.as_deref().unwrap_or("-")
}

/// `now_iso[5:16]` 同位切片:UTC 时刻的 `MM-DDTHH:MM` 段(不足则原样返回)。
fn clock_slice(rfc3339: &str) -> String {
    if rfc3339.chars().count() >= 16 {
        rfc3339.chars().skip(5).take(11).collect()
    } else {
        rfc3339.to_owned()
    }
}

/// 任务行:`<mark> <id> <label>[ · <note>]`(着色;在跑时长后缀待 since 字段)。
fn task_line(task: &TaskView) -> String {
    let state = visual(task.state);
    // TODO(model):无 since 字段,在跑时长后缀(· 2h)暂缺
    let note = task
        .note
        .as_deref()
        .map_or(String::new(), |note| format!(" · {note}"));
    format!(
        "{}{} {} {}{}{}",
        state.color(),
        state.mark(),
        task.id,
        task.label,
        note,
        C_END
    )
}

/// 验证门行:running ▶ / passed ✓ / failed ⚑,detail 非空时带尾注。
fn gate_line(name: &str, state: &GateState) -> String {
    match state {
        GateState::Running => format!("{C_ACTIVE}▶ {name}{C_END}"),
        GateState::Passed { detail } if detail.is_empty() => format!("{C_DONE}✓ {name}{C_END}"),
        GateState::Passed { detail } => format!("{C_DONE}✓ {name} · {detail}{C_END}"),
        GateState::Failed { detail } if detail.is_empty() => format!("{C_STALLED}⚑ {name}{C_END}"),
        GateState::Failed { detail } => format!("{C_STALLED}⚑ {name} · {detail}{C_END}"),
    }
}

/// 里程碑行:`  <id> <标题按显示宽截断> <▓░ x/n> <state>`——窄侧栏下不折行错位。
fn milestone_line(milestone: &MilestoneView, width: usize) -> String {
    let state = milestone_state(milestone);
    let bar = if milestone.total > 0 {
        let fill_n = round_half_even(10 * milestone.done, milestone.total).min(10);
        let fill = "▓".repeat(fill_n);
        format!(
            "{fill}{} {}/{}",
            "░".repeat(10 - fill_n),
            milestone.done,
            milestone.total
        )
    } else {
        String::new()
    };
    let id = ms_id(milestone);
    let reserve = display_width(&format!("  {id}  {bar} {state}")) + 1;
    let budget = width.saturating_sub(reserve).max(4);
    let mut title = String::new();
    for cut in (0..=milestone.title.chars().count()).rev() {
        let prefix: String = milestone.title.chars().take(cut).collect();
        if display_width(&prefix) <= budget {
            title = prefix;
            break;
        }
    }
    let color = if state == "done" { C_DONE } else { C_ACTIVE };
    format!("  {color}{id} {title} {bar} {state}{C_END}")
}
