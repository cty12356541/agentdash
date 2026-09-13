//! TUI watch(W1-007):ratatui 差分重绘 + crossterm 键鼠交互。
//!
//! - 视图:面板 ⇄ 图(`g` 切换),文本取自 `render` 层,SGR 码换算为
//!   ratatui 样式(`spans_from_ansi`),每帧只提交与上一帧的单元格差;
//! - 键位:`f` 进入输入态(行编辑 task id,⏎ 确认 / Esc 取退)、`c` 发送聚焦
//!   提示(环境变量 `DASH_TMUX_TARGET` 存在且 tmux 可用 → `tmux send-keys`,
//!   否则状态行给可复制文本)、`d`/⏎ 进任务详情右栏(40%,Esc/`d` 返回)、
//!   `↑`/`↓` 在波次间滚动(渲染视图按选中波次折算任务集,纯函数
//!   [`select_wave`])、`?` 全键位帮助覆盖层(任意键关闭)、`q`/Ctrl-C 退出;
//! - 鼠标:SGR 左键点击 → [`handle_click`] 复用 `render::graph` 的布局几何
//!   与 [`render::graph::hit_test`] 命中(仅图视图);
//! - 分级刷新:模型每 interval 档重建,git 快照仅每 30s 边界重取(节流做在
//!   merge 外:持有快照缓存经 [`model::merge_with_git`] 注入,合并语义不变;
//!   到期判定用注入时钟的纯函数 [`model_due`] / [`git_due`],可测);
//! - 终端还原:正常退出与 panic hook 都走同一条还原路径。
//!
//! 真终端仅在 stdin 为 tty 时启用;管道/重定向下 [`watch`] 等同
//! [`watch_once`](渲染一帧即退,即 `--once` 冒烟路径)。

use std::io::{self, IsTerminal, Stdout, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::{Frame, Terminal};

use crate::model::{self, Dashboard, GateView, TaskView};
use crate::render::{self, C_ACTIVE, C_DONE, C_END, C_STALLED, elide, graph::Cell};
use crate::sources::git::{self, GitFacts};

/// 模型重建节奏默认档(秒;`agentdash watch` 的 interval)。
pub const MODEL_INTERVAL: u64 = 5;
/// git 快照重取边界(秒):模型档内复用上次快照,仅跨过该边界才重新探测。
pub const GIT_REFRESH_SECS: u64 = 30;
/// 事件轮询粒度(毫秒):键鼠即时响应,刷新判定每轮复核。
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// watch 主入口:真 TUI 差分重绘循环。
///
/// stdin 非 tty(管道/重定向)时自动退化为 [`watch_once`]。
///
/// # Errors
/// 终端初始化/还原、事件轮询与帧绘制的 IO 错误原样透传。
pub fn watch(repo: &Path, interval_secs: u64) -> io::Result<()> {
    if !io::stdin().is_terminal() {
        // 管道/重定向 stdin:无法交互,等同 --once
        return watch_once(repo);
    }
    let mut terminal = setup_terminal()?;
    let outcome = run_loop(&mut terminal, repo, interval_secs);
    let restored = restore_terminal(&mut terminal);
    outcome.and(restored)
}

/// 单帧冒烟(`--once`):重建一次模型,经 [`once_output`] 决策形态后打印。
///
/// # Errors
/// 打印失败(如管道关闭)时透传 `io::Error`。
pub fn watch_once(repo: &Path) -> io::Result<()> {
    let dash = model::merge(repo);
    let frame = once_output(&dash, stdout_cols(), render::DEFAULT_PANEL_WIDTH);
    writeln!(io::stdout(), "{frame}")
}

/// 原始终端列数(AD-ERR-004):非 tty(管道/重定向)或读取失败 → `None`
/// (调用方以 `default` 档兜底)。**不钳位**——形态退化判定必须先于
/// [`render::clamp_width`],先钳到 40 就看不出终端本来就窄。
#[must_use]
pub fn stdout_cols() -> Option<usize> {
    if io::stdout().is_terminal() {
        crossterm::terminal::size()
            .ok()
            .map(|(cols, _)| usize::from(cols))
    } else {
        None
    }
}

/// 输出形态(AD-ERR-004):框化视图(带 40..120 钳位宽)或窄终端退化的
/// oneline 单行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputForm {
    /// 框化视图(panel/graph),字段为钳位后的宽度。
    Framed(usize),
    /// oneline 单行(statusline)。
    OneLine,
}

/// 形态决策(`render`/`watch` 出图前的收口,纯函数可测):原始列数低于
/// [`render::MIN_WIDTH`] 时框化视图必破图(钳位下限硬抬 40 会顶穿终端),
/// 退化 oneline 单行;否则按 40..120 钳位宽出框化视图。非 tty 无列数,
/// 按 `default` 档走框化(管道冒烟不缩水)。
#[must_use]
pub fn output_form(raw_cols: Option<usize>, default: usize) -> OutputForm {
    match raw_cols.unwrap_or(default) {
        cols if cols < render::MIN_WIDTH => OutputForm::OneLine,
        cols => OutputForm::Framed(render::clamp_width(cols)),
    }
}

/// 单帧输出(`--once`/管道冒烟,`render` 命令同判):形态决策 + 默认面板
/// 渲染收口,纯函数([`watch_once`] 的可测内核)。
#[must_use]
pub fn once_output(dash: &Dashboard, raw_cols: Option<usize>, default: usize) -> String {
    match output_form(raw_cols, default) {
        OutputForm::OneLine => render::render_oneline(dash),
        OutputForm::Framed(width) => render::render_panel(dash, width),
    }
}

// ---------- 可测纯函数(键位映射 / 行编辑 / 刷新节拍 / 命中 / 投递) ----------

/// 键位语义态:常规单键命令 vs 输入态行编辑 vs 帮助覆盖层模态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    /// 常规:单键命令。
    Normal,
    /// 输入:行编辑 task id。
    Editing,
    /// 帮助覆盖层:任意键关闭。关闭吞键做在 `on_key` 入口(模态),不走
    /// [`key_action`] 映射表——该态落到常规表仅保持函数全定义。
    Help,
}

/// 键盘动作(W1-007 键位表)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// 输入字符(编辑态)。
    Input(char),
    /// 删除末字符(编辑态)。
    Erase,
    /// 确认输入 → 设定聚焦(编辑态 ⏎)。
    Submit,
    /// 取消输入(编辑态 Esc)。
    Cancel,
    /// 进入输入态(常规 `f`)。
    StartInput,
    /// 发送/打印聚焦提示(常规 `c`)。
    FocusHint,
    /// 面板 ⇄ 图(常规 `g`)。
    ToggleView,
    /// 详情态开/关(常规 `d`:有聚焦进详情,已开则返回)。
    ToggleDetail,
    /// ⏎ 打开聚焦任务详情(无聚焦/任务不在模时运行层回落聚焦提示)。
    EnterDetail,
    /// 返回列表(详情态 Esc)。
    Back,
    /// 帮助覆盖层开(常规 `?`;关闭走 `on_key` 的模态吞键,任意键返回)。
    Help,
    /// 上一波次(常规 ↑)。
    PrevWave,
    /// 下一波次(常规 ↓)。
    NextWave,
    /// 退出(常规 `q`、任意态 Ctrl-C)。
    Quit,
    /// 忽略。
    Ignore,
}

/// 键 → 动作的纯映射(TUI 循环与测试共用同一张表)。
#[must_use]
pub fn key_action(mode: InputMode, key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if mode == InputMode::Editing {
        return match key.code {
            KeyCode::Char('c') if ctrl => Action::Quit,
            KeyCode::Char(ch) => Action::Input(ch),
            KeyCode::Backspace => Action::Erase,
            KeyCode::Enter => Action::Submit,
            KeyCode::Esc => Action::Cancel,
            _ => Action::Ignore,
        };
    }
    match (ctrl, key.code) {
        (true, KeyCode::Char('c')) => Action::Quit,
        (_, KeyCode::Char('q')) if !ctrl => Action::Quit,
        (_, KeyCode::Char('g')) if !ctrl => Action::ToggleView,
        (_, KeyCode::Char('f')) if !ctrl => Action::StartInput,
        (_, KeyCode::Char('c')) if !ctrl => Action::FocusHint,
        (_, KeyCode::Enter) if !ctrl => Action::EnterDetail,
        (_, KeyCode::Char('d')) if !ctrl => Action::ToggleDetail,
        (_, KeyCode::Char('?')) if !ctrl => Action::Help,
        (_, KeyCode::Up) if !ctrl => Action::PrevWave,
        (_, KeyCode::Down) if !ctrl => Action::NextWave,
        (_, KeyCode::Esc) => Action::Back,
        _ => Action::Ignore,
    }
}

/// 行编辑纯函数:编辑态动作应用到输入缓冲(按字符而非字节,CJK 安全);
/// 非编辑动作原样返回。
#[must_use]
pub fn edit_line(input: &str, action: &Action) -> String {
    match action {
        Action::Input(ch) => format!("{input}{ch}"),
        Action::Erase => {
            let mut owned = input.to_owned();
            owned.pop();
            owned
        }
        _ => input.to_owned(),
    }
}

/// 刷新节拍(注入时钟):本帧是否重建模型——距上次重建 ≥ interval 档。
#[must_use]
pub fn model_due(now_secs: u64, last_secs: u64, interval_secs: u64) -> bool {
    now_secs.saturating_sub(last_secs) >= interval_secs
}

/// 刷新节拍(注入时钟):本帧是否重取 git 快照——模型到期重建、且距上次
/// 快照跨过 30s 边界;模型未到期绝不重取。
#[must_use]
pub fn git_due(now_secs: u64, last_git_secs: u64, model_rebuild: bool) -> bool {
    model_rebuild && now_secs.saturating_sub(last_git_secs) >= GIT_REFRESH_SECS
}

/// 终端点击 → 命中任务 id:把终端 0 基坐标减去视图区原点后,换算成渲染文本
/// 1 基行号复用 [`render::graph::hit_test`](与 `render_graph` 布局同几何);空白/偏移
/// 越界返回 `None`。
#[must_use]
pub fn handle_click(
    layers: &[Vec<Cell>],
    col: u16,
    row: u16,
    origin_x: u16,
    origin_y: u16,
) -> Option<String> {
    if col < origin_x || row < origin_y {
        return None;
    }
    render::graph::hit_test(
        layers,
        usize::from(col - origin_x),
        usize::from(row - origin_y) + 1,
    )
}

/// 聚焦提示投递方式:`DASH_TMUX_TARGET` 存在且 tmux 可用 → send-keys;
/// 否则给可复制文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// 经 tmux 注入目标 pane。
    Tmux {
        /// 目标 pane(`DASH_TMUX_TARGET` 原样)。
        target: String,
        /// 注入文本(聚焦 task id)。
        text: String,
    },
    /// 打印可复制文本。
    Print(String),
}

/// 投递判定纯函数(检测在调用方:环境变量 + `tmux -V` 探测)。
#[must_use]
pub fn focus_delivery(target: Option<&str>, tmux_available: bool, text: &str) -> Delivery {
    match target.filter(|_| tmux_available) {
        Some(target) => Delivery::Tmux {
            target: target.to_owned(),
            text: text.to_owned(),
        },
        None => Delivery::Print(text.to_owned()),
    }
}

// ---------- W2-003 详情面板 / W2-004 波次滚动与帮助(纯函数) ----------

/// 详情右栏行(纯函数,快照可测):`label`/`state`/`lane`/`note`/`fix_round`/
/// `since` + 关联 gates(模型无 task↔gate 关联,全量列出)+ 该任务事件
/// tail——事件层暂未随 [`Dashboard`] 携带(W2-008 接入),以占位行明示。
/// 每行按显示宽截断(`width` 为栏内容宽;只裁行宽,不裁行数),着色在截断
/// 后注入。
#[must_use]
pub fn detail_lines(task: &TaskView, dash: &Dashboard, width: usize) -> Vec<String> {
    let state = render::visual(task.state);
    let body = format!("状态  {} {}", state.mark(), task.state.as_str());
    let mut lines = vec![
        field_line("标签", &task.label, width),
        format!("{}{}{C_END}", state.color(), elide(&body, width)),
        field_line("车道", task.lane.as_deref().unwrap_or("-"), width),
        field_line("备注", task.note.as_deref().unwrap_or("-"), width),
        field_line(
            "轮次",
            &task.fix_round.map_or_else(
                || "-".to_owned(),
                |(done, total)| format!("R{done}/{total}"),
            ),
            width,
        ),
        field_line("时刻", task.since.as_deref().unwrap_or("-"), width),
    ];
    lines.push(String::from("验证门"));
    if dash.gates.is_empty() {
        lines.push(String::from("  -"));
    } else {
        for gate in &dash.gates {
            lines.push(gate_line(gate, width));
        }
    }
    lines.push(String::from("事件"));
    // 事件 tail(最近 10 条,agent+gate 过滤)待 W2-008:Dashboard 尚无事件投影
    lines.push(String::from("  事件层 W2-008 接入"));
    lines
}

/// `键  值` 行,按显示宽截断(详情栏窄侧板防顶穿)。
fn field_line(label: &str, value: &str, width: usize) -> String {
    elide(&format!("{label}  {value}"), width)
}

/// 详情栏 gate 行(三态映射承 panel):`  <mark> <name> <state>[ · <detail>]`。
fn gate_line(gate: &GateView, width: usize) -> String {
    let (mark, color) = gate_visual(&gate.state);
    let body = if gate.detail.is_empty() {
        format!("  {mark} {} {}", gate.name, gate.state)
    } else {
        format!("  {mark} {} {} · {}", gate.name, gate.state, gate.detail)
    };
    format!("{color}{}{C_END}", elide(&body, width))
}

/// gate 状态串 → (标记, ANSI 前景色):passed ✓ 绿 / failed ✗ 黄 / 其余 ▶ 蓝。
fn gate_visual(state: &str) -> (&'static str, &'static str) {
    match state {
        "passed" => ("✓", C_DONE),
        "failed" => ("✗", C_STALLED),
        _ => ("▶", C_ACTIVE),
    }
}

/// 波次选中折算(纯函数,W2-004):选中索引钳位进里程碑界内,返回
/// `(折算索引, 可见任务集)`。任务↔波次关联模型未携带(单账本即单波),
/// 过滤规则:任务 `id` 以波次串为前缀段(`W2` 匹配 `W2-003`,不匹配
/// `W25-T1`)。无里程碑、选中波次未声明编号或全不匹配 → 回退全量——宁可
/// 多显示不少显示;关联字段入模后(W3 多账本)应收严此回退。
#[must_use]
pub fn select_wave(dash: &Dashboard, selected: usize) -> (usize, Vec<&TaskView>) {
    let idx = selected.min(dash.milestones.len().saturating_sub(1));
    let Some(milestone) = dash.milestones.get(idx) else {
        return (0, dash.tasks.iter().collect());
    };
    let Some(wave) = milestone.wave.as_deref() else {
        return (idx, dash.tasks.iter().collect());
    };
    let matched: Vec<&TaskView> = dash
        .tasks
        .iter()
        .filter(|task| wave_matches(&task.id, wave))
        .collect();
    if matched.is_empty() {
        (idx, dash.tasks.iter().collect())
    } else {
        (idx, matched)
    }
}

/// 任务 id 与波次串的前缀段匹配:分隔符或串尾定界(`W2`~`W2-003` 成立,
/// 不吃 `W25` 的半截词)。
fn wave_matches(id: &str, wave: &str) -> bool {
    match id.strip_prefix(wave) {
        None => false,
        Some(rest) => match rest.chars().next() {
            None => true,
            Some(ch) => !ch.is_alphanumeric(),
        },
    }
}

/// 选中波次的渲染视图(纯函数):[`Dashboard`] 仅把 `tasks` 折算为可见集
/// (克隆换集),里程碑/屏障/git 原样——model 不动,波次过滤收口在 tui 层,
/// `render` 层不感知滚动。
#[must_use]
pub fn wave_view(dash: &Dashboard, selected: usize) -> Dashboard {
    let (_, visible) = select_wave(dash, selected);
    let mut view = dash.clone();
    view.tasks = visible.into_iter().cloned().collect();
    view
}

/// 页眉波次标注(纯函数):`波次 <wave> <k>/<n>`;无里程碑返回空串(不标注)。
#[must_use]
pub fn wave_tag(dash: &Dashboard, selected: usize) -> String {
    if dash.milestones.is_empty() {
        return String::new();
    }
    let idx = selected.min(dash.milestones.len() - 1);
    let total = dash.milestones.len();
    let wave = dash.milestones[idx].wave.as_deref().unwrap_or("-");
    format!("波次 {wave} {}/{total}", idx + 1)
}

/// 全键位帮助卡片(W2-004,纯函数):一行一键位,`?` 覆盖层与测试共用。
#[must_use]
pub fn help_lines() -> Vec<String> {
    vec![
        "g      面板 ⇄ 图切换".to_owned(),
        "f      输入 task id 聚焦".to_owned(),
        "c      发送聚焦提示(tmux send-keys 或可复制文本)".to_owned(),
        "⏎      打开聚焦任务详情(无聚焦回落提示)".to_owned(),
        "d      任务详情开/关(Esc 或 d 返回)".to_owned(),
        "Esc    返回列表(详情态)".to_owned(),
        "↑/↓    切换选中波次(↑ 上一波,↓ 下一波)".to_owned(),
        "?      本帮助(任意键关闭)".to_owned(),
        "q      退出(Ctrl-C 任意态)".to_owned(),
    ]
}

/// 渲染层 SGR 调色 → ratatui 样式:只识别 render 层实际发出的码
/// (0 复位 / 1 加粗 / 32 绿 / 33 黄 / 34 蓝 / 90 暗灰),其余码剥除不着色。
fn sgr_style(codes: &str, base: Style) -> Style {
    let mut style = base;
    for code in codes.split(';') {
        match code.parse::<u8>() {
            Ok(0) => style = Style::new(),
            Ok(1) => style = style.add_modifier(Modifier::BOLD),
            Ok(32) => style = style.fg(Color::Green),
            Ok(33) => style = style.fg(Color::Yellow),
            Ok(34) => style = style.fg(Color::Blue),
            Ok(90) => style = style.fg(Color::DarkGray),
            _ => {}
        }
    }
    style
}

/// ANSI 行 → ratatui Span 序列(差分重绘吃结构化文本):SGR 码换算为样式,
/// 其余控制序列剥除;空行返回空表。
#[must_use]
pub fn spans_from_ansi(line: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut text = String::new();
    let mut style = Style::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next(); // 吃掉 '['
            let mut codes = String::new();
            for esc in chars.by_ref() {
                if esc == 'm' {
                    break;
                }
                codes.push(esc);
            }
            let next = sgr_style(&codes, style);
            if next != style && !text.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut text), style));
            }
            style = next;
        } else {
            text.push(ch);
        }
    }
    if !text.is_empty() {
        spans.push(Span::styled(text, style));
    }
    spans
}

// ---------- 终端设置 / 还原 ----------

/// 本 crate 的终端类型别名(crossterm 后端 + stdout)。
type Tui = Terminal<CrosstermBackend<Stdout>>;

/// 初始化:raw mode + 备用屏 + SGR 鼠标捕获;panic hook 先还原终端再接回
/// 原 hook(崩溃不留 raw/备用屏残留)。中途失败自动回滚已做的步骤。
fn setup_terminal() -> io::Result<Tui> {
    install_panic_restore_hook();
    enable_raw_mode()?;
    let attempt = || -> io::Result<Tui> {
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        Terminal::new(CrosstermBackend::new(io::stdout()))
    };
    let result = attempt();
    if result.is_err() {
        let _ = restore_raw();
    }
    result
}

/// 正常路径还原:鼠标/备用屏/raw mode + 显示光标;幂等可重入。
fn restore_terminal(terminal: &mut Tui) -> io::Result<()> {
    let outcome = restore_raw();
    terminal.show_cursor()?;
    outcome
}

/// 还原核心(panic 路径与正常路径共用;各步骤失败静默——还原序尽量走完)。
fn restore_raw() -> io::Result<()> {
    let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
    disable_raw_mode()
}

/// panic hook:先还原终端,再调用接管前的原 hook。
fn install_panic_restore_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_raw();
        previous(info);
    }));
}

// ---------- 主循环与状态 ----------

/// 主循环:分级刷新 → 差分重绘 → 事件分派 → 退出判定。
fn run_loop(terminal: &mut Tui, repo: &Path, interval_secs: u64) -> io::Result<()> {
    let mut app = Watch::new(repo, interval_secs);
    loop {
        app.refresh();
        // ratatui 差分重绘:draw 每帧提交与上一帧缓冲的单元格差
        terminal.draw(|frame| draw(frame, &mut app))?;
        if !event::poll(POLL_INTERVAL)? {
            continue;
        }
        match event::read()? {
            // Windows 终端会补发 Release 事件:只认 Press 防止动作双触发
            Event::Key(key) if key.kind == KeyEventKind::Press => app.on_key(key),
            Event::Mouse(mouse)
                if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) =>
            {
                app.on_click(mouse);
            }
            _ => {}
        }
        if app.quit {
            break;
        }
    }
    Ok(())
}

/// 视图模式:面板 ⇄ 图。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    /// 面板(默认)。
    Panel,
    /// 任务 DAG。
    Graph,
}

/// watch 会话状态(一帧一刷新;几何随选中波次每帧折算重建)。
struct Watch {
    /// 监视的仓库/计划目录。
    repo: PathBuf,
    /// 模型重建档(interval 秒)。
    interval: u64,
    /// 当前视图。
    view: View,
    /// 详情态(右栏 40% 详情卡片)。
    detail: bool,
    /// 选中波次索引(`dashboard.milestones` 下标,渲染时钳位)。
    wave: usize,
    /// 键位语义态(常规/行编辑/帮助覆盖层)。
    mode: InputMode,
    /// 输入缓冲。
    input: String,
    /// 聚焦任务 id。
    focused: Option<String>,
    /// 状态行消息(提示/反馈)。
    message: String,
    /// 退出标记。
    quit: bool,
    /// 单调时钟零点(刷新判定用,秒)。
    clock: Instant,
    /// 上次模型重建时刻。
    last_model: u64,
    /// 上次 git 快照时刻。
    last_git: u64,
    /// git 快照缓存(30s 边界内复用)。
    git_facts: GitFacts,
    /// 当前模型。
    dash: Dashboard,
    /// 图布局几何(命中测试与渲染共用;draw 每帧按选中波次折算重建)。
    layers: Vec<Vec<Cell>>,
    /// 视图区高度(点击越界判定;draw 时更新)。
    view_height: u16,
}

impl Watch {
    fn new(repo: &Path, interval_secs: u64) -> Self {
        // 首帧全量:git 快照与模型同步取,此后进入分级节拍
        let git_facts = git::snapshot(repo);
        let dash = model::merge_with_git(repo, git_facts.clone());
        Self {
            repo: repo.to_owned(),
            interval: interval_secs.max(1),
            view: View::Panel,
            detail: false,
            wave: 0,
            mode: InputMode::Normal,
            input: String::new(),
            focused: None,
            message: "就绪:g 换视图 f 聚焦 ⏎/d 详情 ↑↓ 波次 ? 帮助 q 退出".to_owned(),
            quit: false,
            clock: Instant::now(),
            last_model: 0,
            last_git: 0,
            git_facts,
            dash,
            layers: Vec::new(),
            view_height: 0,
        }
    }

    /// 分级刷新:模型每 interval 档重建;git 快照仅每 30s 边界重取。
    ///
    /// 节流做在 merge 外(W1-007):git 探测最重(多命令 × 3s 超时上限),
    /// watch 持快照缓存经 `merge_with_git` 注入,`model::merge` 语义不变。
    /// 图布局几何不在此缓存:随选中波次的可见任务集在 [`draw`] 每帧折算。
    fn refresh(&mut self) {
        let now = self.clock.elapsed().as_secs();
        let rebuild = model_due(now, self.last_model, self.interval);
        if !rebuild {
            return;
        }
        if git_due(now, self.last_git, rebuild) {
            self.git_facts = git::snapshot(&self.repo);
            self.last_git = now;
        }
        self.dash = model::merge_with_git(&self.repo, self.git_facts.clone());
        self.last_model = now;
    }

    /// 键事件分派(映射表见 [`key_action`])。帮助覆盖层为模态:任意键关闭
    /// 且不透传(含 q/Ctrl-C,再按一次才生效)。
    fn on_key(&mut self, key: KeyEvent) {
        if self.mode == InputMode::Help {
            self.mode = InputMode::Normal;
            return;
        }
        match key_action(self.mode, key) {
            Action::Quit => self.quit = true,
            Action::StartInput => {
                self.mode = InputMode::Editing;
                self.input.clear();
                self.message = String::from("输入 task id");
            }
            Action::Submit => {
                self.mode = InputMode::Normal;
                let id = self.input.trim().to_owned();
                if id.is_empty() {
                    self.message = String::from("空 id:聚焦未改动");
                } else {
                    self.focused = Some(id.clone());
                    self.message = format!("聚焦 {id}");
                }
                self.input.clear();
            }
            Action::Cancel => {
                self.mode = InputMode::Normal;
                self.input.clear();
                self.message = String::from("取消输入");
            }
            action @ (Action::Input(_) | Action::Erase) => {
                self.input = edit_line(&self.input, &action);
            }
            Action::FocusHint => self.focus_hint(),
            Action::ToggleDetail => {
                if self.detail {
                    self.close_detail();
                } else {
                    self.open_detail();
                }
            }
            Action::EnterDetail => {
                if !self.detail && !self.open_detail() {
                    // 无聚焦/任务不在模:沿用 ⏎ 原聚焦提示行为
                    self.focus_hint();
                }
            }
            Action::Back => {
                if self.detail {
                    self.close_detail();
                }
            }
            Action::Help => self.mode = InputMode::Help,
            Action::PrevWave => self.shift_wave(false),
            Action::NextWave => self.shift_wave(true),
            Action::ToggleView => {
                self.view = match self.view {
                    View::Panel => View::Graph,
                    View::Graph => View::Panel,
                };
                self.message = match self.view {
                    View::Panel => "面板视图".to_owned(),
                    View::Graph => "图视图:点击节点可聚焦".to_owned(),
                };
            }
            Action::Ignore => {}
        }
    }

    /// `c` / `⏎`:聚焦提示——`DASH_TMUX_TARGET` 存在且 tmux 可用则
    /// send-keys,否则状态行给可复制文本(TUI 内 println 会打进备用屏,
    /// 统一走状态行)。
    fn focus_hint(&mut self) {
        let Some(id) = self.focused.clone() else {
            self.message = String::from("未聚焦:f 输入 id,或在图视图点击节点");
            return;
        };
        let target = std::env::var("DASH_TMUX_TARGET")
            .ok()
            .filter(|target| !target.is_empty());
        match focus_delivery(target.as_deref(), tmux_available(), &id) {
            Delivery::Tmux { target, text } => {
                self.message = if tmux_send_keys(&target, &text) {
                    format!("已 send-keys 到 {target}:{text}")
                } else {
                    format!("tmux 发送失败;聚焦 id:{text}")
                };
            }
            Delivery::Print(text) => {
                self.message = format!("聚焦:{text}(可复制)");
            }
        }
    }

    /// 尝试打开详情(W2-003):聚焦 id 命中当前模型任务才开;未聚焦或任务
    /// 已被模型刷新洗掉时不开,给反馈行。
    fn open_detail(&mut self) -> bool {
        let Some(id) = self.focused.clone() else {
            self.message = String::from("详情未开:未聚焦(f 输入 id,或图视图点击节点)");
            return false;
        };
        if self.dash.tasks.iter().any(|task| task.id == id) {
            self.detail = true;
            self.message = format!("详情:{id}(Esc/d 返回)");
            true
        } else {
            self.message = format!("详情未开:任务 {id} 不在当前模型(刷新后重试)");
            false
        }
    }

    /// 关详情返回列表。
    fn close_detail(&mut self) {
        self.detail = false;
        self.message = String::from("返回列表");
    }

    /// 波次滚动(W2-004):`down` 为真走 ↓ 下一波,否则 ↑ 上一波;界内钳位,
    /// 无波次不动,状态行给新标注。
    fn shift_wave(&mut self, down: bool) {
        if self.dash.milestones.is_empty() {
            return;
        }
        self.wave = if down {
            (self.wave + 1).min(self.dash.milestones.len() - 1)
        } else {
            self.wave.saturating_sub(1)
        };
        self.message = wave_tag(&self.dash, self.wave);
    }

    /// SGR 左键:图视图命中节点即聚焦;面板视图无节点几何,忽略。
    fn on_click(&mut self, mouse: MouseEvent) {
        if self.view != View::Graph || mouse.row >= self.view_height {
            return;
        }
        let Some(id) = handle_click(&self.layers, mouse.column, mouse.row, 0, 0) else {
            return;
        };
        self.message = format!("点击命中 {id}");
        self.focused = Some(id);
    }

    /// 状态行(单行):键位帮助(含 ?)+ 视图/详情态 + 波次标注 + 聚焦 +
    /// git 快照 + 消息。
    fn status_line(&self) -> String {
        if self.mode == InputMode::Editing {
            let input = &self.input;
            return format!(" 聚焦 id: {input}▏(⏎ 确认 · Backspace 删除 · Esc 取消)");
        }
        let view_label = if self.detail {
            "详情"
        } else {
            match self.view {
                View::Panel => "面板",
                View::Graph => "图",
            }
        };
        let tag = wave_tag(&self.dash, self.wave);
        let wave = if tag.is_empty() {
            String::new()
        } else {
            format!(" │ {tag}")
        };
        let focus = self.focused.as_deref().unwrap_or("—");
        let git = &self.dash.git;
        let branch = git.branch.as_deref().unwrap_or("-");
        let head = git.head_short.as_deref().unwrap_or("-");
        format!(
            " [{}] f 聚焦 c 提示 ⏎/d 详情 g 切换 ↑↓ 波次 ? 帮助 q 退出 │ 聚焦:{focus}{wave} │ {branch}@{head} ↑{} ↓{} ●{} │ {}",
            view_label, git.ahead, git.behind, git.dirty, self.message
        )
    }
}

/// 模型屏障 → 图布局输入(model 与 render 的同构类型换形;与
/// `render_graph` 默认入口同一语义,布局几何与渲染共用一份屏障边)。
fn graph_barriers(dash: &Dashboard) -> Vec<render::graph::BarrierEdges> {
    dash.barriers
        .iter()
        .map(|barrier| render::graph::BarrierEdges {
            after: barrier.after.clone(),
            unlocks: barrier.unlocks.clone(),
        })
        .collect()
}

/// 图布局几何(命中测试与渲染共用 `graph::layout_layers` 单一几何源)。
fn layout_layers(dash: &Dashboard) -> Vec<Vec<Cell>> {
    render::graph::layout_layers(dash, &graph_barriers(dash))
}

/// 一帧:视图区(面板或图;详情态右分栏 60% 主区 + 40% 详情)+ 底部状态行,
/// 帮助覆盖层最上。主区窄于 40 列时框化视图必破图([AD-ERR-004]),退化
/// oneline 单行,状态行照常。波次过滤收口在 tui 层:每帧把模型折算成选中
/// 波次的渲染视图,布局几何随之重建(命中测试与渲染同源)。
fn draw(frame: &mut Frame, app: &mut Watch) {
    let area = frame.area();
    let rows = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);
    app.view_height = rows[0].height;
    let view_dash = wave_view(&app.dash, app.wave);
    app.layers = layout_layers(&view_dash);
    if app.detail {
        let cols = Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(rows[0]);
        render_main(frame, cols[0], &view_dash, app.view);
        render_detail(frame, cols[1], app);
    } else {
        render_main(frame, rows[0], &view_dash, app.view);
    }
    frame.render_widget(app.status_line(), rows[1]);
    if app.mode == InputMode::Help {
        render_help(frame, area);
    }
}

/// 主视图区:窄态退化 oneline,否则面板/图。
fn render_main(frame: &mut Frame, area: Rect, dash: &Dashboard, view: View) {
    let width = usize::from(area.width).max(1);
    let rendered = if width < render::MIN_WIDTH {
        render::render_oneline(dash)
    } else {
        match view {
            View::Panel => render::render_panel(dash, width),
            View::Graph => render::graph::render_graph(dash, width),
        }
    };
    let lines: Vec<_> = rendered
        .lines()
        .map(|line| Line::from(spans_from_ansi(line)))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// 详情右栏(40%):聚焦任务的详情卡片;聚焦任务被模型刷新洗掉时框内给
/// 提示行(不白板)。
fn render_detail(frame: &mut Frame, area: Rect, app: &Watch) {
    let Some(task) = app
        .focused
        .as_deref()
        .and_then(|id| app.dash.tasks.iter().find(|task| task.id == id))
    else {
        let block = Block::default().borders(Borders::ALL).title("详情");
        frame.render_widget(Paragraph::new("聚焦任务不在当前模型").block(block), area);
        return;
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("详情 {} · Esc/d 返回", task.id));
    let inner_width = usize::from(area.width).saturating_sub(2).max(1);
    let lines: Vec<_> = detail_lines(task, &app.dash, inner_width)
        .into_iter()
        .map(|line| Line::from(spans_from_ansi(&line)))
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// 帮助覆盖层(W2-004):居中键位卡片;关闭由 [`Watch::on_key`] 的模态吞键。
fn render_help(frame: &mut Frame, area: Rect) {
    let card = centered(area, 70, 45);
    frame.render_widget(Clear, card);
    let lines: Vec<_> = help_lines().into_iter().map(Line::from).collect();
    let block = Block::default()
        .borders(Borders::ALL)
        .title("键位帮助(任意键关闭)");
    frame.render_widget(Paragraph::new(lines).block(block), card);
}

/// 居中取 `percent_x` × `percent_y` 的子矩形(双轴百分比留白)。
fn centered(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let rows = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Fill(1),
    ])
    .split(area);
    let cols = Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Fill(1),
    ])
    .split(rows[1]);
    cols[1]
}

/// tmux 是否可用(`tmux -V` 探测;spawn 失败或非零一律不可用)。
fn tmux_available() -> bool {
    std::process::Command::new("tmux")
        .arg("-V")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// `tmux send-keys -t <target> -l <text>`(字面注入,不代按回车)。
fn tmux_send_keys(target: &str, text: &str) -> bool {
    std::process::Command::new("tmux")
        .args(["send-keys", "-t", target, "-l", text])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}
