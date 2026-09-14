//! 终端面板:区块 A 在跑/健康 → B 轨迹 → C 车道(W1-006,移植自 claude-dash
//! `dashlib/render_panel.py`)。ANSI 着色、按显示宽自适应,只读不落盘。

use std::fmt::Write as _;

use super::{
    C_ACTIVE, C_BOLD, C_DONE, C_END, C_PENDING, C_STALLED, C_WARN, Visual, active_milestone,
    clamp_width, display_width, is_lane_marker, milestone_state, project_label, round_half_even,
    visual,
};
use crate::contract::TaskState;
use crate::model::{AgentView, Dashboard, GateView, MilestoneView, TaskView};

/// 面板默认宽。
pub const DEFAULT_PANEL_WIDTH: usize = 64;

/// 任务停滞阈值:任务 `since` 距 `generated_at` 超过 2 小时(严格大于)打 ⚑。
const STALE_THRESHOLD_SECS: u64 = 2 * 60 * 60;

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
    // 吞吐暂无逐时刻数据,以里程碑计数近似(见区块 B 速度线)
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
        None => lines.push(project_label(dash)),
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
    // 区块 A′:PR / 远程(W2-007;remote 缺省 = 探测降级,整块不打不虚占版面)
    if let Some(remote) = &dash.remote {
        push_remote_block(&mut lines, remote, width);
    }
    for warning in &dash.warnings {
        lines.push(format!(
            "{C_WARN}⚠ {}{C_END}",
            elide(warning, width.saturating_sub(2))
        ));
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
    if let Some(speed) = speed_line(&dash.milestones) {
        lines.push(speed);
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
        let lane_groups = lane_groups(&dash.tasks);
        push_lane_lines(&mut lines, &lane_groups, &dash.generated_at);
        // TODO(render):屏障行(claude-dash `  {barrier}`)——模型已携带 barriers
        // (T7 入模),面板侧渲染仍未做,待后续车道
    }
    lines.join("\n")
}

/// PR / 远程区块(W2-007):`#<号> <标题>` + 逐 check 行;超宽整行截断。
fn push_remote_block(
    lines: &mut Vec<String>,
    remote: &crate::sources::remote::RemoteFacts,
    width: usize,
) {
    lines.push(format!("{C_BOLD}PR / 远程{C_END}"));
    if remote.pr_number == 0 {
        lines.push(format!("{C_PENDING}· 无关联 PR{C_END}"));
    } else {
        let head = format!("#{} {}", remote.pr_number, remote.pr_title);
        lines.push(format!("{C_ACTIVE}{}{C_END}", elide(&head, width)));
    }
    for (name, state) in &remote.checks {
        lines.push(check_line(name, state, width));
    }
}

/// 车道分组(首见序;无车道任务归"无车道"组)。
fn lane_groups(tasks: &[TaskView]) -> Vec<(String, Vec<&TaskView>)> {
    let mut groups: Vec<(String, Vec<&TaskView>)> = Vec::new();
    for task in tasks {
        let name = task.lane.clone().unwrap_or_else(|| "无车道".to_owned());
        match groups.iter_mut().find(|(group, _)| *group == name) {
            Some((_, members)) => members.push(task),
            None => groups.push((name, vec![task])),
        }
    }
    groups
}

/// 车道行输出:折叠车道(W2-005,视图层发空 id 伪任务)出
/// `▸ 车道名 (N done)` 单行;其余车道头 + 逐任务行照旧。
fn push_lane_lines(
    lines: &mut Vec<String>,
    groups: &[(String, Vec<&TaskView>)],
    generated_at: &str,
) {
    for (name, members) in groups {
        if members.len() == 1 && is_lane_marker(members[0]) {
            lines.push(format!("{C_BOLD}▸ {name} {}{C_END}", members[0].label));
            continue;
        }
        lines.push(format!("{C_BOLD}{name}{C_END}"));
        for task in members {
            lines.push(task_line(task, generated_at));
        }
    }
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

/// 速度线:里程碑无逐任务时刻,吞吐以「最近 3 波 done 均值(任务/波次)」
/// 近似(银行家舍入);单里程碑无均值可言,返回 [`None`] 不打。
fn speed_line(milestones: &[MilestoneView]) -> Option<String> {
    if milestones.len() < 2 {
        return None;
    }
    let window = milestones.len().min(3);
    let recent_done: usize = milestones
        .iter()
        .rev()
        .take(window)
        .map(|milestone| milestone.done)
        .sum();
    let per_wave = round_half_even(recent_done, window);
    Some(format!("  速度 {per_wave} 任务/波次"))
}

/// 任务行:`<mark> <id> <label>[ · <note>][ R<N>/<M>][ ⚑]`——R 尾缀出自
/// `fix_round`(W2-002);note 已解析出 `fix_round` 时不再重复输出原文(W2-3b);
/// since 距 `generated_at` 超过 2 小时([`STALE_THRESHOLD_SECS`])打停滞 ⚑,
/// 无戳/坏戳/时刻在未来一律不打(不虚报)。
fn task_line(task: &TaskView, generated_at: &str) -> String {
    let state = visual(task.state);
    let note = if task.fix_round.is_some() {
        String::new()
    } else {
        task.note
            .as_deref()
            .map_or(String::new(), |note| format!(" · {note}"))
    };
    let fix_round = task
        .fix_round
        .map_or(String::new(), |(done, total)| format!(" R{done}/{total}"));
    let is_stale = task.state != TaskState::Done
        && task
            .since
            .as_deref()
            .and_then(rfc3339_to_secs)
            .zip(rfc3339_to_secs(generated_at))
            .is_some_and(|(since, now)| now.saturating_sub(since) > STALE_THRESHOLD_SECS);
    let stale_flag = if is_stale { " ⚑" } else { "" };
    format!(
        "{}{} {} {}{}{}{}{}",
        state.color(),
        state.mark(),
        task.id,
        task.label,
        note,
        fix_round,
        stale_flag,
        C_END
    )
}

/// 纯函数:`YYYY-MM-DDTHH:MM:SSZ` → Unix 纪元秒(Hinnant civil 逆变换,与
/// `model::utc_timestamp` 互逆)。形态不符、字段越界或年份为 0 返回 [`None`]。
fn rfc3339_to_secs(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return None;
    }
    let field = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
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
    u64::try_from(days * 86_400 + hour * 3_600 + minute * 60 + second).ok()
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

/// PR check 行(W2-007,gh 词表原样直出不翻译):SUCCESS ✓ 绿 /
/// FAILURE ✗ 黄 / 其余(PENDING 等)▶ 蓝;超宽整行截断。
fn check_line(name: &str, state: &str, width: usize) -> String {
    let (mark, color) = match state {
        "SUCCESS" => ("✓", C_DONE),
        "FAILURE" => ("✗", C_STALLED),
        _ => ("▶", C_ACTIVE),
    };
    let body = format!("  {mark} {name} {state}");
    format!("{color}{}{C_END}", elide(&body, width))
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
