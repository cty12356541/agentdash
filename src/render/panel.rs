//! 终端面板:区块 A 在跑/健康 → B 轨迹 → C 车道(W1-006,移植自 claude-dash
//! `dashlib/render_panel.py`)。ANSI 着色、按显示宽自适应,只读不落盘。

use std::fmt::Write as _;

use super::{
    C_ACTIVE, C_BOLD, C_DONE, C_END, C_PENDING, C_STALLED, C_WARN, Visual, active_milestone,
    clamp_width, clock_slice, display_width, elide, is_lane_marker, milestone_state,
    round_half_even, truncate_width, visual,
};
use crate::contract::TaskState;
use crate::model::{
    AgentView, BarrierEdges, Dashboard, GateView, MilestoneView, TaskView, parse_fix_round,
    rfc3339_to_secs, velocity,
};

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
    let mut resting = 0; // pending + blocked(· 计数,口径不变)
    let mut blocked = 0; // 仅 blocked(⊘ 槽,W3-001;与 · 双计)
    for task in &dash.tasks {
        match visual(task.state) {
            Visual::Done => done += 1,
            Visual::Active => running += 1,
            Visual::Stalled => {
                running += 1;
                stalled += 1;
            }
            Visual::Pending => resting += 1,
            Visual::Blocked => {
                resting += 1;
                blocked += 1;
            }
        }
    }
    // 页眉 agent 计数即在跑表行数;吞吐口径已入模型,见区块 B 速度线
    // (W3-006 起为事件活动窗真实吞吐,不再是里程碑计数近似)
    let agents = dash.agents.len();

    // 页眉:项目 · 活跃里程碑(D3:project 读模型字段;整行过 elide 钳宽)
    let active_ms = active_milestone(dash);
    let head = match active_ms {
        Some(milestone) => format!(
            "{} · {} {}",
            dash.project,
            ms_id(milestone),
            milestone.title
        ),
        None => dash.project.clone(),
    };
    lines.push(elide(&head, width));
    let clock = clock_slice(&dash.generated_at);
    lines.push(format!(
        "{C_DONE}✓{done}{C_END} {C_ACTIVE}▶{running}{C_END} \
         {C_PENDING}·{resting}{C_END} {C_STALLED}⚑{stalled}{C_END} \
         {C_PENDING}⊘{blocked}{C_END} \
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
    if let Some(speed) = speed_line(dash) {
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
    }
    // 区块 C′:屏障行(W3-002;claude-dash `  {barrier}`)——车道区之后按
    // 台账声明序直出,语义与 graph 的 after→unlocks 边同向
    push_barrier_lines(&mut lines, &dash.barriers, width);
    lines.join("\n")
}

/// 屏障行(W3-002):`  ⇕ <after 逗号表> → <unlocks 逗号表>`,台账声明序;
/// 无屏障不产出任何行(零残留,不打标题/空态)。屏障 id 未随模型入模
/// ([`BarrierEdges`] 只承载 after/unlocks),不虚标 `B<N>`。after/unlocks
/// 按台账声明原样直出(graph 侧对未知 id 的过滤是布局约束,面板是声明
/// 视图,照单全收)。超宽整行截断。
fn push_barrier_lines(lines: &mut Vec<String>, barriers: &[BarrierEdges], width: usize) {
    for barrier in barriers {
        let body = format!(
            "  ⇕ {} → {}",
            barrier.after.join(","),
            barrier.unlocks.join(",")
        );
        lines.push(format!("{C_PENDING}{}{C_END}", elide(&body, width)));
    }
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

/// 速度线(W3-006 会话活动窗吞吐):`速度 <done/活动窗小时> tasks/h`——
/// 生效条件与数值全部出自 [`velocity`] 模型纯函数(≥2 里程碑,span 首选
/// 事件活动窗、缺失回退任务 `since` 跨度,须严格大于 0),不满足则不打
/// (负断言钉死,不虚报)。
fn speed_line(dash: &Dashboard) -> Option<String> {
    let rate = velocity(&dash.tasks, &dash.milestones, dash.event_span_secs)?;
    Some(format!("  速度 {rate:.1} tasks/h"))
}

/// 任务行:`<mark> <id> <label>[ R<N>/<M>[ <残余 note>]][ · <note>][ ⚑]`——
/// R 尾缀出自 `fix_round`(W2-002);W3-001 起仅抑**匹配前缀**:note 为
/// `fix round N/M <残余>` 时残余折到尾缀之后(`R2/5 auth bug`),解析不出
/// 残余(纯 `fix round N/M`)只出尾缀,解析不了 `fix_round` 才回退整段
/// note 原文;since 距 `generated_at` 超过 2 小时([`STALE_THRESHOLD_SECS`])
/// 打停滞 ⚑,无戳/坏戳/时刻在未来一律不打(不虚报);两种 RFC 3339 形态
/// (`…Z` UTC / `…±HH:MM` 本地偏移,发现 9)统一按绝对时刻折算比较
/// ([`rfc3339_to_secs`] 出自 model,W3-004)。
fn task_line(task: &TaskView, generated_at: &str) -> String {
    let state = visual(task.state);
    let residual = task
        .note
        .as_deref()
        .and_then(parse_fix_round)
        .map(|(_, rest)| rest)
        .filter(|rest| !rest.is_empty());
    let note = if task.fix_round.is_some() {
        residual.map_or(String::new(), |rest| format!(" {rest}"))
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
        fix_round,
        note,
        stale_flag,
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
