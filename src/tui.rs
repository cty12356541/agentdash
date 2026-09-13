//! TUI watch(W1-007):ratatui 差分重绘 + crossterm 键鼠交互。
//!
//! - 视图:面板 ⇄ 图(`g` 切换),文本取自 `render` 层,SGR 码换算为
//!   ratatui 样式(`spans_from_ansi`),每帧只提交与上一帧的单元格差;
//! - 键位:`f` 进入输入态(行编辑 task id,⏎ 确认 / Esc 取退)、`c` 与 `⏎`
//!   发送聚焦提示(环境变量 `DASH_TMUX_TARGET` 存在且 tmux 可用 →
//!   `tmux send-keys`,否则状态行给可复制文本)、`q`/Ctrl-C 退出;
//! - 鼠标:SGR 左键点击 → [`handle_click`] 复用 `render::graph` 的布局几何
//!   与 [`render::hit_test`] 命中(仅图视图);
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
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::{Frame, Terminal};

use crate::model::{self, Dashboard};
use crate::render::{self, Cell};
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

/// 单帧冒烟(`--once`):重建一次模型,打印默认面板帧后返回。
///
/// # Errors
/// 打印失败(如管道关闭)时透传 `io::Error`。
pub fn watch_once(repo: &Path) -> io::Result<()> {
    let dash = model::merge(repo);
    let width = stdout_width(render::DEFAULT_PANEL_WIDTH);
    let mut out = io::stdout();
    writeln!(out, "{}", render::render_panel(&dash, width))
}

/// 输出宽度:非 tty 用默认;tty 读终端列并经 40..120 钳位
/// ([`render::clamp_width`],panel/graph 共用)。
#[must_use]
pub fn stdout_width(default: usize) -> usize {
    let cols = if io::stdout().is_terminal() {
        crossterm::terminal::size()
            .ok()
            .map(|(cols, _)| usize::from(cols))
    } else {
        None
    };
    render::clamp_width(cols.unwrap_or(default))
}

// ---------- 可测纯函数(键位映射 / 行编辑 / 刷新节拍 / 命中 / 投递) ----------

/// 键位语义态:常规单键命令 vs 输入态行编辑。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    /// 常规:单键命令。
    Normal,
    /// 输入:行编辑 task id。
    Editing,
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
    /// 发送/打印聚焦提示(常规 `c` 或 `⏎`)。
    FocusHint,
    /// 面板 ⇄ 图(常规 `g`)。
    ToggleView,
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
        (_, KeyCode::Char('c') | KeyCode::Enter) if !ctrl => Action::FocusHint,
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
/// 1 基行号复用 [`render::hit_test`](与 `render_graph` 布局同几何);空白/偏移
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
    render::hit_test(
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

/// watch 会话状态(一帧一刷新;几何缓存随模型重建)。
struct Watch {
    /// 监视的仓库/计划目录。
    repo: PathBuf,
    /// 模型重建档(interval 秒)。
    interval: u64,
    /// 当前视图。
    view: View,
    /// 输入态(行编辑 task id)。
    editing: bool,
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
    /// 图布局几何(命中测试与渲染共用)。
    layers: Vec<Vec<Cell>>,
    /// 视图区高度(点击越界判定;draw 时更新)。
    view_height: u16,
}

impl Watch {
    fn new(repo: &Path, interval_secs: u64) -> Self {
        // 首帧全量:git 快照与模型同步取,此后进入分级节拍
        let git_facts = git::snapshot(repo);
        let dash = model::merge_with_git(repo, git_facts.clone());
        let layers = layout_layers(&dash);
        Self {
            repo: repo.to_owned(),
            interval: interval_secs.max(1),
            view: View::Panel,
            editing: false,
            input: String::new(),
            focused: None,
            message: "就绪:g 换视图 f 聚焦 c/⏎ 提示 q 退出".to_owned(),
            quit: false,
            clock: Instant::now(),
            last_model: 0,
            last_git: 0,
            git_facts,
            dash,
            layers,
            view_height: 0,
        }
    }

    /// 分级刷新:模型每 interval 档重建;git 快照仅每 30s 边界重取。
    ///
    /// 节流做在 merge 外(W1-007):git 探测最重(多命令 × 3s 超时上限),
    /// watch 持快照缓存经 `merge_with_git` 注入,`model::merge` 语义不变。
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
        self.layers = layout_layers(&self.dash);
        self.last_model = now;
    }

    /// 键事件分派(映射表见 [`key_action`])。
    fn on_key(&mut self, key: KeyEvent) {
        let mode = if self.editing {
            InputMode::Editing
        } else {
            InputMode::Normal
        };
        match key_action(mode, key) {
            Action::Quit => self.quit = true,
            Action::StartInput => {
                self.editing = true;
                self.input.clear();
                self.message = String::from("输入 task id");
            }
            Action::Submit => {
                self.editing = false;
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
                self.editing = false;
                self.input.clear();
                self.message = String::from("取消输入");
            }
            action @ (Action::Input(_) | Action::Erase) => {
                self.input = edit_line(&self.input, &action);
            }
            Action::FocusHint => self.focus_hint(),
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

    /// 状态行(单行):键位帮助 + 聚焦 + git 快照 + 消息。
    fn status_line(&self) -> String {
        if self.editing {
            let input = &self.input;
            return format!(" 聚焦 id: {input}▏(⏎ 确认 · Backspace 删除 · Esc 取消)");
        }
        let view_label = match self.view {
            View::Panel => "面板",
            View::Graph => "图",
        };
        let focus = self.focused.as_deref().unwrap_or("—");
        let git = &self.dash.git;
        let branch = git.branch.as_deref().unwrap_or("-");
        let head = git.head_short.as_deref().unwrap_or("-");
        format!(
            " [{}] f 聚焦 c/⏎ 提示 g 切换 q 退出 │ 聚焦:{focus} │ {branch}@{head} ↑{} ↓{} ●{} │ {}",
            view_label, git.ahead, git.behind, git.dirty, self.message
        )
    }
}

/// 模型屏障 → 图布局输入(model 与 render 的同构类型换形;与
/// `render_graph` 默认入口同一语义,布局几何与渲染共用一份屏障边)。
fn graph_barriers(dash: &Dashboard) -> Vec<render::BarrierEdges> {
    dash.barriers
        .iter()
        .map(|barrier| render::BarrierEdges {
            after: barrier.after.clone(),
            unlocks: barrier.unlocks.clone(),
        })
        .collect()
}

/// 图布局几何(命中测试与渲染共用 `render::layout_layers` 单一几何源)。
fn layout_layers(dash: &Dashboard) -> Vec<Vec<Cell>> {
    render::layout_layers(dash, &graph_barriers(dash))
}

/// 一帧:视图区(面板或图)+ 底部状态行。
fn draw(frame: &mut Frame, app: &mut Watch) {
    let area = frame.area();
    let rows = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);
    app.view_height = rows[0].height;
    let width = usize::from(rows[0].width).max(1);
    let rendered = match app.view {
        View::Panel => render::render_panel(&app.dash, width),
        View::Graph => render::render_graph(&app.dash, width),
    };
    let lines: Vec<_> = rendered
        .lines()
        .map(|line| Line::from(spans_from_ansi(line)))
        .collect();
    frame.render_widget(Paragraph::new(lines), rows[0]);
    frame.render_widget(app.status_line(), rows[1]);
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
