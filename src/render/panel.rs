//! 终端面板:区块 A 在跑/健康 → B 轨迹 → C 车道(W1-006,移植自 claude-dash
//! `dashlib/render_panel.py`)。ANSI 着色、按显示宽自适应,只读不落盘。

use std::fmt::Write as _;

use super::{
    C_ACTIVE, C_BOLD, C_DONE, C_END, C_PENDING, C_STALLED, C_WARN, Visual, active_milestone,
    clamp_width, display_width, milestone_state, project_label, round_half_even, visual,
};
use crate::model::{AgentView, Dashboard, GateView, MilestoneView, TaskView};

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
    // TODO(model):Dashboard 无 velocity(commits_7d)字段,吞吐段暂缺(W2-T2)。
    let agents = dash.agents.len();

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

    // 区块 A:在跑 / 健康(在跑 agents → 验证门终态)
    lines.push(format!("{C_BOLD}在跑 / 健康{C_END}"));
    if dash.agents.is_empty() && dash.gates.is_empty() {
        lines.push(format!("{C_DONE}✓ 无活跃/卡死{C_END}"));
    } else {
        if dash.agents.is_empty() {
            lines.push(format!("{C_PENDING}· 无活跃{C_END}"));
        }
        for agent in &dash.agents {
            lines.push(agent_line(agent, width));
        }
        // 模型层保证 gates 按门名字典序,此处按存储序直出
        for gate in &dash.gates {
            lines.push(gate_line(gate, width));
        }
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

/// 在跑 agent 行:`▶ <who>[ · <task>][ · <MM-DDTHH:MM>]`,超宽整行截断。
fn agent_line(agent: &AgentView, width: usize) -> String {
    let mut line = format!("▶ {}", agent.who);
    if let Some(task) = agent.task.as_deref() {
        let _ = write!(line, " · {task}");
    }
    if !agent.since.is_empty() {
        let _ = write!(line, " · {}", clock_slice(&agent.since));
    }
    format!("{C_ACTIVE}{}{C_END}", elide(&line, width))
}

/// 验证门行:running ▶ / passed ✓ / failed ✗,detail 非空时带尾注并按宽截断。
fn gate_line(gate: &GateView, width: usize) -> String {
    let (mark, color) = match gate.state.as_str() {
        "passed" => ("✓", C_DONE),
        "failed" => ("✗", C_STALLED),
        _ => ("▶", C_ACTIVE), // running;未知态兜底按进行中呈现
    };
    let mut line = format!("{mark} {}", gate.name);
    if !gate.detail.is_empty() {
        let _ = write!(line, " · {}", gate.detail);
    }
    format!("{color}{}{C_END}", elide(&line, width))
}

/// 按显示宽截断到 `budget` 列内的最长前缀(与里程碑标题同一算法)。
fn truncate_width(text: &str, budget: usize) -> String {
    for cut in (0..=text.chars().count()).rev() {
        let prefix: String = text.chars().take(cut).collect();
        if display_width(&prefix) <= budget {
            return prefix;
        }
    }
    String::new()
}

/// 截断并在发生截断时以 `…`(1 列)收尾;总宽仍不超 `budget`。
fn elide(text: &str, budget: usize) -> String {
    let cut = truncate_width(text, budget);
    if cut.chars().count() == text.chars().count() {
        return cut;
    }
    let mut out = truncate_width(text, budget.saturating_sub(1));
    if display_width(&out) < budget {
        out.push('…');
    }
    out
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
    let title = truncate_width(&milestone.title, budget);
    let color = if state == "done" { C_DONE } else { C_ACTIVE };
    format!("  {color}{id} {title} {bar} {state}{C_END}")
}
