//! 终端面板:区块 A 在跑/健康 → B 轨迹 → C 车道(W1-006,移植自 claude-dash
//! `dashlib/render_panel.py`)。ANSI 着色、按显示宽自适应,只读不落盘。

use std::fmt::Write as _;

use super::{
    C_ACTIVE, C_BOLD, C_DONE, C_END, C_PENDING, C_STALLED, C_WARN, Visual, active_milestone,
    clamp_width, clock_slice, display_width, elide, is_lane_marker, milestone_state,
    round_half_even, truncate_width, unattested_done, visual,
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
    panel_lines(dash, width)
        .into_iter()
        .map(|(line, _)| line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// 行级面板产物(W12-010 TUI 点击命中):每行 `(文本, 任务 id)`——任务行
/// `Some(id)`,其余行(页眉/健康/轨迹/车道头/警告/屏障)`None`。与
/// [`render_panel`] 同一构建路径,命中测试与渲染零漂移。
#[must_use]
pub fn render_panel_rows(dash: &Dashboard, width: usize) -> (String, Vec<Option<String>>) {
    let lines = panel_lines(dash, width);
    let rows = lines.iter().map(|(_, id)| id.clone()).collect();
    let text = lines
        .into_iter()
        .map(|(line, _)| line)
        .collect::<Vec<_>>()
        .join("\n");
    (text, rows)
}

/// 面板行构建(单一事实源):`(文本, 任务 id)` 对,id 见 [`render_panel_rows`]。
fn panel_lines(dash: &Dashboard, width: usize) -> Vec<(String, Option<String>)> {
    let width = clamp_width(width);
    let mut lines: Vec<(String, Option<String>)> = Vec::new();

    // 页眉:项目 · 活跃里程碑(D3:project 读模型字段;整行过 elide 钳宽)
    plain(&mut lines, elide(&head_line(dash), width));
    // 页眉 agent 计数即在跑表行数;吞吐口径已入模型,见区块 B 速度线
    // (W3-006 起为事件活动窗真实吞吐,不再是里程碑计数近似)
    let tally = counts(&dash.tasks);
    plain(
        &mut lines,
        stats_line(
            &tally,
            dash.running_agents(),
            &clock_slice(&dash.generated_at),
        ),
    );
    plain(&mut lines, "═".repeat(width));

    // 区块 A:在跑 / 健康(在跑 agents → 验证门终态;W11-003 起推断配对行
    // 随行带显式标注,计数与空态判定都只看未推断行)
    plain(&mut lines, format!("{C_BOLD}在跑 / 健康{C_END}"));
    if dash.agents.is_empty() && dash.gates.is_empty() {
        plain(&mut lines, format!("{C_DONE}✓ 无活跃/卡死{C_END}"));
    } else {
        if dash.running_agents() == 0 {
            plain(&mut lines, format!("{C_PENDING}· 无活跃{C_END}"));
        }
        for agent in &dash.agents {
            plain(&mut lines, agent_line(agent, width));
        }
        // 模型层保证 gates 按门名字典序,此处按存储序直出
        for gate in &dash.gates {
            plain(&mut lines, gate_line(gate, width));
        }
    }
    // 区块 A′:PR / 远程(W2-007;remote 缺省 = 探测降级,整块不打不虚占版面)
    if let Some(remote) = &dash.remote {
        push_remote_block(&mut lines, remote, width);
    }
    for warning in &dash.warnings {
        plain(
            &mut lines,
            format!(
                "{C_WARN}⚠ {}{C_END}",
                elide(warning, width.saturating_sub(2))
            ),
        );
    }
    plain(&mut lines, "─".repeat(width));

    // 区块 B:轨迹
    let done_ms = dash
        .milestones
        .iter()
        .filter(|milestone| milestone_state(milestone) == "done")
        .count();
    plain(
        &mut lines,
        format!("{C_BOLD}轨迹 · {done_ms} 里程碑{C_END}"),
    );
    for milestone in dash.milestones.iter().rev().take(5).rev() {
        plain(&mut lines, milestone_line(milestone, width));
    }
    if let Some(speed) = speed_line(dash) {
        plain(&mut lines, speed);
    }
    let planned = dash
        .milestones
        .iter()
        .find(|milestone| milestone_state(milestone) == "planned");
    let active_ms = active_milestone(dash);
    if let (Some(next), None) = (planned, active_ms) {
        plain(&mut lines, format!("  下一步:待启动 {}", ms_id(next)));
    } else if active_ms.is_some() {
        plain(&mut lines, "  进行中".to_owned());
    } else {
        plain(&mut lines, "  —".to_owned()); // 全部完成/全新仓库:中性占位,不虚报"进行中"
    }
    plain(&mut lines, "─".repeat(width));

    // 区块 C:车道 / 任务
    if !dash.tasks.is_empty() {
        plain(&mut lines, format!("{C_BOLD}车道 / 任务{C_END}"));
        let lane_groups = lane_groups(&dash.tasks);
        push_lane_lines(&mut lines, &lane_groups, dash);
    }
    // 区块 C′:屏障行(W3-002;claude-dash `  {barrier}`)——车道区之后按
    // 台账声明序直出,语义与 graph 的 after→unlocks 边同向
    push_barrier_lines(&mut lines, &dash.barriers, width);
    lines
}

/// 精要视图(W10-001 多仓聚合):每仓一块——页眉(项目 · 活跃里程碑)→
/// 统计行(与 [`render_panel`] 同口径同格式)→ 在跑 agents → 失败门 →
/// blocked 任务 → ⚠ 警告。缺项零残留(不打空标题/空态占位),无区块分隔
/// 线;块间空行由调用方拼装。宽度先经 40..120 钳位。
#[must_use]
pub fn render_brief(dash: &Dashboard, width: usize) -> String {
    let width = clamp_width(width);
    let mut lines: Vec<String> = Vec::new();

    lines.push(elide(&head_line(dash), width));
    let tally = counts(&dash.tasks);
    lines.push(stats_line(
        &tally,
        dash.running_agents(),
        &clock_slice(&dash.generated_at),
    ));
    for agent in &dash.agents {
        lines.push(agent_line(agent, width));
    }
    // 失败门:✗ 名 · detail(门行复用全面板同款;passed/running 不上精要)
    for gate in &dash.gates {
        if gate.state == "failed" {
            lines.push(gate_line(gate, width));
        }
    }
    // blocked 任务(⊘ 行复用全面板同款;其余态不上精要)
    for task in &dash.tasks {
        if visual(task.state) == Visual::Blocked {
            lines.push(task_line(task, dash));
        }
    }
    for warning in &dash.warnings {
        lines.push(format!(
            "{C_WARN}⚠ {}{C_END}",
            elide(warning, width.saturating_sub(2))
        ));
    }
    lines.join("\n")
}

/// 任务五态计数(panel 统计行与 W10-001 精要视图/W10-002 摘要同源):▶ 含
/// active + stalled(承 Python 口径),· 含 pending + blocked 双计,⊘ 仅
/// blocked(W3-001 口径,与 · 双计)。
pub(super) struct Counts {
    done: usize,
    running: usize,
    stalled: usize,
    resting: usize,
    blocked: usize,
}

/// [`Counts`] 原地复算(单一事实源,panel/精要/摘要三视图共享)。
pub(super) fn counts(tasks: &[TaskView]) -> Counts {
    let mut tally = Counts {
        done: 0,
        running: 0,
        stalled: 0,
        resting: 0,
        blocked: 0,
    };
    for task in tasks {
        match visual(task.state) {
            Visual::Done => tally.done += 1,
            Visual::Active => tally.running += 1,
            Visual::Stalled => {
                tally.running += 1;
                tally.stalled += 1;
            }
            Visual::Pending => tally.resting += 1,
            Visual::Blocked => {
                tally.resting += 1;
                tally.blocked += 1;
            }
        }
    }
    tally
}

/// 页眉:项目 · 活跃里程碑(D3:project 读模型字段;panel/精要/摘要同源)。
pub(super) fn head_line(dash: &Dashboard) -> String {
    match active_milestone(dash) {
        Some(milestone) => format!(
            "{} · {} {}",
            dash.project,
            ms_id(milestone),
            milestone.title
        ),
        None => dash.project.clone(),
    }
}

/// 统计槽位(单一事实源):(符号, 计数) 五槽——panel 着色版与 digest
/// 纯文本版(W10-002)共同的上游,槽位序/口径改这里一处即两视图同步。
fn stats_slots(tally: &Counts) -> [(char, usize); 5] {
    [
        ('✓', tally.done),
        ('▶', tally.running),
        ('·', tally.resting),
        ('⚑', tally.stalled),
        ('⊘', tally.blocked),
    ]
}

/// 统计行正文(纯文本):`✓d ▶r ·rest ⚑st ⊘b · N agents · MM-DDTHH:MM`。
/// digest(无 ANSI)直用;着色版([`stats_line`])逐槽着色同一槽位。
pub(super) fn stats_body(tally: &Counts, agents: usize, clock: &str) -> String {
    let slots = stats_slots(tally)
        .map(|(mark, count)| format!("{mark}{count}"))
        .join(" ");
    format!("{slots} · {agents} agents · {clock}")
}

/// 统计行(着色,panel 与精要视图同格式):逐槽 ANSI 色,槽位与正文
/// ([`stats_body`])同源;agents 计数即在跑表行数。
fn stats_line(tally: &Counts, agents: usize, clock: &str) -> String {
    let colors = [C_DONE, C_ACTIVE, C_PENDING, C_STALLED, C_PENDING];
    let slots = stats_slots(tally)
        .iter()
        .zip(colors)
        .map(|((mark, count), color)| format!("{color}{mark}{count}{C_END}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!("{slots} · {agents} agents · {clock}")
}

/// 屏障行(W3-002):`  ⇕ <after 逗号表> → <unlocks 逗号表>`,台账声明序;
/// 无屏障不产出任何行(零残留,不打标题/空态)。屏障 id 未随模型入模
/// ([`BarrierEdges`] 只承载 after/unlocks),不虚标 `B<N>`。after/unlocks
/// 按台账声明原样直出(graph 侧对未知 id 的过滤是布局约束,面板是声明
/// 视图,照单全收)。超宽整行截断。
fn push_barrier_lines(
    lines: &mut Vec<(String, Option<String>)>,
    barriers: &[BarrierEdges],
    width: usize,
) {
    for barrier in barriers {
        let body = format!(
            "  ⇕ {} → {}",
            barrier.after.join(","),
            barrier.unlocks.join(",")
        );
        lines.push((format!("{C_PENDING}{}{C_END}", elide(&body, width)), None));
    }
}

/// PR / 远程区块(W2-007):`#<号> <标题>` + 逐 check 行;超宽整行截断。
fn push_remote_block(
    lines: &mut Vec<(String, Option<String>)>,
    remote: &crate::sources::remote::RemoteFacts,
    width: usize,
) {
    plain(lines, format!("{C_BOLD}PR / 远程{C_END}"));
    if remote.pr_number == 0 {
        plain(lines, format!("{C_PENDING}· 无关联 PR{C_END}"));
    } else {
        let head = format!("#{} {}", remote.pr_number, remote.pr_title);
        plain(lines, format!("{C_ACTIVE}{}{C_END}", elide(&head, width)));
    }
    for (name, state) in &remote.checks {
        plain(lines, check_line(name, state, width));
    }
}

/// 非任务行入列(健康/轨迹/警告等,id 恒 `None`)。
fn plain(lines: &mut Vec<(String, Option<String>)>, text: String) {
    lines.push((text, None));
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
/// `▸ 车道名 (N done)` 单行;其余车道头 + 逐任务行照旧(任务行带 id,
/// W12-010 点击命中用)。
fn push_lane_lines(
    lines: &mut Vec<(String, Option<String>)>,
    groups: &[(String, Vec<&TaskView>)],
    dash: &Dashboard,
) {
    for (name, members) in groups {
        if members.len() == 1 && is_lane_marker(members[0]) {
            plain(
                lines,
                format!("{C_BOLD}▸ {name} {}{C_END}", members[0].label),
            );
            continue;
        }
        plain(lines, format!("{C_BOLD}{name}{C_END}"));
        for task in members {
            lines.push((task_line(task, dash), Some(task.id.clone())));
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

/// 任务行:`<mark> <id> <label>[ R<N>/<M>[ <残余 note>]][ · <note>][ ⚑][ ?]`——
/// R 尾缀出自 `fix_round`(W2-002);W3-001 起仅抑**匹配前缀**:note 为
/// `fix round N/M <残余>` 时残余折到尾缀之后(`R2/5 auth bug`),解析不出
/// 残余(纯 `fix round N/M`)只出尾缀,解析不了 `fix_round` 才回退整段
/// note 原文;since 距 `generated_at` 超过 2 小时([`STALE_THRESHOLD_SECS`])
/// 打停滞 ⚑,无戳/坏戳/时刻在未来一律不打(不虚报);done 且自报无物证
/// ([`unattested_done`],W5-001)打 `?`;两种 RFC 3339 形态(`…Z` UTC /
/// `…±HH:MM` 本地偏移,发现 9)统一按绝对时刻折算比较
/// ([`rfc3339_to_secs`] 出自 model,W3-004)。
fn task_line(task: &TaskView, dash: &Dashboard) -> String {
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
            .zip(rfc3339_to_secs(&dash.generated_at))
            .is_some_and(|(since, now)| now.saturating_sub(since) > STALE_THRESHOLD_SECS);
    let stale_flag = if is_stale { " ⚑" } else { "" };
    // 物证怀疑标记(W5-001):done 且自报时刻晚于最近通过门(或事件在场而无
    // 通过门)→ `?`;判定细节(无 done_at/无事件窗不标)收口 unattested_done
    let suspect_flag = if unattested_done(task, dash) {
        " ?"
    } else {
        ""
    };
    format!(
        "{}{} {} {}{}{}{}{}{}",
        state.color(),
        state.mark(),
        task.id,
        task.label,
        fix_round,
        note,
        stale_flag,
        suspect_flag,
        C_END
    )
}

/// agent 行正文(纯文本):真在跑 `▶ <who>[ [<host>]][ · <task>][ ·
/// <MM-DDTHH:MM>]`;W11-003 推断配对行(已完成,不占在跑计数)显式标注
/// `▶⇢✓ <who>… (inferred)`——推断非实测,标注是显示真相(着色版与
/// digest 纯文本版同源;宿主归属 W7-001)。
pub(super) fn agent_body(agent: &AgentView) -> String {
    let mark = if agent.inferred { "▶⇢✓" } else { "▶" };
    let mut line = format!("{mark} {}", agent.who);
    if let Some(host) = agent.host.as_deref() {
        let _ = write!(line, " [{host}]"); // 宿主归属(W7-001)
    }
    if let Some(task) = agent.task.as_deref() {
        let _ = write!(line, " · {task}");
    }
    if !agent.since.is_empty() {
        let _ = write!(line, " · {}", clock_slice(&agent.since));
    }
    if agent.inferred {
        line.push_str(" (inferred)");
    }
    line
}

/// agent 行(着色):正文([`agent_body`])按宽截断后套前景色——真在跑
/// 活跃色,推断完成行走完成色(W11-003,与完成侧计数口径同源)。
fn agent_line(agent: &AgentView, width: usize) -> String {
    let color = if agent.inferred { C_DONE } else { C_ACTIVE };
    format!("{color}{}{C_END}", elide(&agent_body(agent), width))
}

/// 验证门行正文(纯文本):`<mark> <name>[ · <detail>][ (unknown)]`,running ▶ /
/// passed ✓ / failed ✗;W12-009 起失败分两态——有退出码证据 ✗(红,真失败),
/// 无证据 `?` + `(unknown)` 尾注(暗,承 W11-003 `(inferred)` 先例:标注是
/// 显示真相);着色版与 digest 纯文本版同源。
pub(super) fn gate_body(gate: &GateView) -> String {
    let (mark, unknown) = match gate.state.as_str() {
        "passed" => ("✓", false),
        "failed" if gate.unknown => ("?", true),
        "failed" => ("✗", false),
        _ => ("▶", false), // running;未知态兜底按进行中呈现
    };
    // 旧折叠把 `(exit unknown)` 写进 detail 尾(events 契约原文);第三态下
    // 尾注由渲染统一表达,剥掉避免双注
    let detail = gate
        .detail
        .strip_suffix(" (exit unknown)")
        .unwrap_or(&gate.detail);
    let mut line = format!("{mark} {}", gate.name);
    if !detail.is_empty() {
        let _ = write!(line, " · {detail}");
    }
    if unknown {
        line.push_str(" (unknown)");
    }
    line
}

/// 验证门行(着色):正文([`gate_body`])按宽截断后套门态前景色——
/// unknown 第三态走暗色,不占红(真失败)也不冒绿(通过)。
fn gate_line(gate: &GateView, width: usize) -> String {
    let color = match gate.state.as_str() {
        "passed" => C_DONE,
        "failed" if gate.unknown => C_PENDING,
        "failed" => C_STALLED,
        _ => C_ACTIVE,
    };
    format!("{color}{}{C_END}", elide(&gate_body(gate), width))
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
