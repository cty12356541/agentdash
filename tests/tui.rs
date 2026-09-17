//! W1-007 TUI 集成测试:键→动作映射、行编辑、SGR 鼠标命中(空面板与 w25
//! 样例)、刷新节拍(注入时钟)、聚焦投递判定、SGR→Span 换算、barriers
//! 入模后 `render_graph` 默认入口消费模型屏障(双轨已收敛:显式入口删除),
//! 以及窄终端形态退化与输出宽度决策(AD-ERR-004,纯函数注入列数)。
//! 不测真终端。
//!
//! W2 批二增补:任务详情面板(`detail_lines` 快照/截断,`d`/⏎/Esc 键位)、
//! 多波次滚动(`select_wave`/`wave_view`/`wave_tag` 过滤与钳位)与 help
//! 覆盖层键位表(`?`,全键位标注)。
//!
//! W2 批三增补(T5/T8):过滤(`filter_tasks`/`filter_view`,子串匹配
//! lane/state/label)、车道折叠(`done_lanes`/`collapse_view` + tab 循环)、
//! watch 位置参数(`parse_watch_args`,数字首参 = interval 钳 1..3600)、
//! Windows 送对话通道(无 tmux 落盘 `prompt.txt`)、help 新键位。
//!
//! agentdash 是纯二进制 crate,集成测试按 `#[path]` 在 crate 根挂载模块树,
//! 与 `tests/merge.rs` / `tests/render_graph.rs` 同约定;挂载源中本测试未
//! 触达的 pub 项在此 crate 属死代码,按文件级 allow 放行。

#![allow(dead_code)]

#[path = "../src/contract.rs"]
mod contract;
#[path = "../src/events.rs"]
mod events;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/render/mod.rs"]
mod render;
#[path = "../src/sources/mod.rs"]
mod sources;
#[path = "../src/tui.rs"]
mod tui;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier};

use contract::TaskState;
use model::{BarrierEdges, Dashboard, EventTailView, GateView, MilestoneView, TaskView};
use render::graph::Cell;
use sources::git::GitFacts;
use tui::{Action, Delivery, InputMode};

/// 挂载 tui.rs 会连带消费渲染/模型面;本测试的 Dashboard 均按黄金样例手工构造。
const GENERATED_AT: &str = "2026-09-13T08:30:00Z";

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn task(id: &str, label: &str, lane: &str) -> TaskView {
    TaskView {
        id: id.into(),
        label: label.into(),
        state: TaskState::Done,
        lane: Some(lane.into()),
        note: None,
        fix_round: None,
        since: None,
        done_at: None,
    }
}

/// w25 形态样例(与 `tests/render_graph.rs` 同构,但 barriers 随模型携带):
/// 车道各一任务,无车道链,全部层级由屏障边(T1→T2、T2+T3→T4)形成。
fn w25_dash() -> Dashboard {
    Dashboard {
        tasks: vec![
            task("T1", "fiber join 即回收", "A"),
            task("T2", "scope by_id 索引", "B"),
            task("T3", "orphan 上界", "C"),
            task("T4", "集成收尾", "D"),
        ],
        milestones: vec![MilestoneView {
            wave: Some("W25".into()),
            title: "并行车道冲刺".into(),
            done: 4,
            total: 4,
        }],
        warnings: Vec::new(),
        barriers: vec![
            BarrierEdges {
                after: vec!["T1".into()],
                unlocks: vec!["T2".into()],
            },
            BarrierEdges {
                after: vec!["T2".into(), "T3".into()],
                unlocks: vec!["T4".into()],
            },
        ],
        git: GitFacts::absent(),
        agents: Vec::new(),
        gates: Vec::new(),
        remote: None,
        event_span_secs: None,       // W3-006:速度线事件活动窗;TUI 样例默认无
        project: "agentdash".into(), // W3-004 D3:项目名上模型
        event_tail: Vec::new(),
        events_present: false, // W5-001:物证窗;渲染组样例默认无
        last_gate_passed: None,
        generated_at: GENERATED_AT.into(),
    }
}

/// 空 Dashboard:全无空态,命中测试必须处处 None。
fn empty_dash() -> Dashboard {
    Dashboard {
        tasks: Vec::new(),
        milestones: Vec::new(),
        warnings: Vec::new(),
        barriers: Vec::new(),
        git: GitFacts::absent(),
        agents: Vec::new(),
        gates: Vec::new(),
        remote: None,
        event_span_secs: None,       // W3-006:速度线事件活动窗;TUI 样例默认无
        project: "agentdash".into(), // W3-004 D3:项目名上模型
        event_tail: Vec::new(),
        events_present: false, // W5-001:物证窗;渲染组样例默认无
        last_gate_passed: None,
        generated_at: GENERATED_AT.into(),
    }
}

/// 模型屏障 → 图屏障(与 tui 内部换形同款);布局几何取自同一源。
fn layout(dash: &Dashboard) -> Vec<Vec<Cell>> {
    let barriers: Vec<render::graph::BarrierEdges> = dash
        .barriers
        .iter()
        .map(|barrier| render::graph::BarrierEdges {
            after: barrier.after.clone(),
            unlocks: barrier.unlocks.clone(),
        })
        .collect();
    render::graph::layout_layers(dash, &barriers)
}

fn cell_of(layers: &[Vec<Cell>], id: &str) -> Cell {
    layers
        .iter()
        .flatten()
        .find(|cell| cell.id == id)
        .cloned()
        .unwrap_or_else(|| panic!("cell {id} not laid out"))
}

/// 剥 ANSI 转义(颜色码 `\x1b[...m`;与 `tests/render_graph.rs` 同款)。
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            for esc in chars.by_ref() {
                if esc.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

// ---------- 键 → 动作映射(纯函数) ----------

#[test]
fn normal_mode_key_map() {
    let none = KeyModifiers::NONE;
    assert_eq!(
        tui::key_action(InputMode::Normal, key(KeyCode::Char('q'), none)),
        Action::Quit
    );
    assert_eq!(
        tui::key_action(
            InputMode::Normal,
            key(KeyCode::Char('c'), KeyModifiers::CONTROL)
        ),
        Action::Quit,
        "Ctrl-C 任意态退出"
    );
    assert_eq!(
        tui::key_action(InputMode::Normal, key(KeyCode::Char('g'), none)),
        Action::ToggleView
    );
    assert_eq!(
        tui::key_action(InputMode::Normal, key(KeyCode::Char('f'), none)),
        Action::StartInput
    );
    assert_eq!(
        tui::key_action(InputMode::Normal, key(KeyCode::Char('c'), none)),
        Action::FocusHint
    );
    assert_eq!(
        tui::key_action(InputMode::Normal, key(KeyCode::Enter, none)),
        Action::EnterDetail,
        "⏎ 进详情(无聚焦时运行层回落聚焦提示)"
    );
    assert_eq!(
        tui::key_action(InputMode::Normal, key(KeyCode::Char('x'), none)),
        Action::Ignore
    );
}

#[test]
fn editing_mode_key_map() {
    let none = KeyModifiers::NONE;
    assert_eq!(
        tui::key_action(InputMode::Editing, key(KeyCode::Char('W'), none)),
        Action::Input('W')
    );
    assert_eq!(
        tui::key_action(InputMode::Editing, key(KeyCode::Backspace, none)),
        Action::Erase
    );
    assert_eq!(
        tui::key_action(InputMode::Editing, key(KeyCode::Enter, none)),
        Action::Submit
    );
    assert_eq!(
        tui::key_action(InputMode::Editing, key(KeyCode::Esc, none)),
        Action::Cancel
    );
    assert_eq!(
        tui::key_action(
            InputMode::Editing,
            key(KeyCode::Char('c'), KeyModifiers::CONTROL)
        ),
        Action::Quit,
        "编辑态 Ctrl-C 仍即时退出"
    );
    assert_eq!(
        tui::key_action(InputMode::Editing, key(KeyCode::Char('c'), none)),
        Action::Input('c'),
        "编辑态裸 c 是输入字符,不是提示"
    );
}

// ---------- 行编辑(纯函数,CJK 安全) ----------

#[test]
fn edit_line_appends_and_erases_by_char() {
    let mut input = String::new();
    for ch in "W1-007".chars() {
        input = tui::edit_line(&input, &Action::Input(ch));
    }
    assert_eq!(input, "W1-007");
    input = tui::edit_line(&input, &Action::Erase);
    assert_eq!(input, "W1-00");
    let mut cjk = String::new();
    for ch in "聚焦".chars() {
        cjk = tui::edit_line(&cjk, &Action::Input(ch));
    }
    cjk = tui::edit_line(&cjk, &Action::Erase);
    assert_eq!(cjk, "聚", "退格按字符撕,不撕裂码点");
    assert_eq!(tui::edit_line("", &Action::Erase), "", "空串退格保持空");
    assert_eq!(
        tui::edit_line("abc", &Action::ToggleView),
        "abc",
        "非编辑动作不改缓冲"
    );
}

// ---------- 刷新节拍(注入时钟的决策函数) ----------

#[test]
fn model_rebuilds_on_interval_boundary() {
    assert!(!tui::model_due(4, 0, 5), "未满一档不重建");
    assert!(tui::model_due(5, 0, 5), "满档重建");
    assert!(tui::model_due(105, 100, 5));
    assert!(tui::model_due(9, 0, 5), "事件停滞后一次性补上");
}

#[test]
fn git_snapshot_only_at_thirty_second_boundary() {
    assert!(!tui::git_due(29, 0, true), "30s 内不重取");
    assert!(tui::git_due(30, 0, true), "30s 边界重取");
    assert!(tui::git_due(100, 70, true));
    assert!(
        !tui::git_due(100, 0, false),
        "模型未到期绝不重取 git(分级刷新)"
    );
}

// ---------- SGR 鼠标命中(空面板 + w25 样例) ----------

#[test]
fn click_on_empty_dashboard_hits_nothing() {
    let layers = layout(&empty_dash());
    assert_eq!(tui::handle_click(&layers, 0, 0, 0, 0), None);
    assert_eq!(tui::handle_click(&layers, 50, 5, 0, 0), None);
}

#[test]
fn click_hits_w25_box_region_only() {
    let dash = w25_dash();
    let layers = layout(&dash);
    let t2 = cell_of(&layers, "T2");
    for row in [t2.line - 1, t2.line, t2.line + 1] {
        let col = u16::try_from(t2.x + 2).expect("col fits u16");
        let row = u16::try_from(row).expect("row fits u16");
        assert_eq!(
            tui::handle_click(&layers, col, row, 0, 0).as_deref(),
            Some("T2"),
            "框三行整框可命中"
        );
    }
    let outside_x = u16::try_from(t2.x + t2.width + 1).expect("col fits u16");
    let row = u16::try_from(t2.line).expect("row fits u16");
    assert_eq!(
        tui::handle_click(&layers, outside_x, row, 0, 0),
        None,
        "框外不命中"
    );
}

#[test]
fn click_maps_through_view_origin_offset() {
    let dash = w25_dash();
    let layers = layout(&dash);
    let t4 = cell_of(&layers, "T4");
    let (ox, oy) = (3_u16, 2_u16); // 视图区在终端里的原点
    // 取 T4 右缘外一格:只有扣掉原点偏移后才落回框内
    let col = u16::try_from(t4.x + t4.width + 1).expect("col fits u16");
    let row = u16::try_from(t4.line + usize::from(oy)).expect("row fits u16");
    assert_eq!(
        tui::handle_click(&layers, col, row, ox, oy).as_deref(),
        Some("T4"),
        "原点偏移参与换算"
    );
    assert_eq!(
        tui::handle_click(&layers, col, row, 0, 0),
        None,
        "不带偏移的同一坐标落在框间空档"
    );
}

// ---------- 聚焦投递判定(纯函数;W2-008 起 Windows 通道走 prompt.txt) ----------

#[test]
fn focus_delivery_prefers_tmux_when_available() {
    assert_eq!(
        tui::focus_delivery(Some("session:0.1"), true, "W1-007"),
        Delivery::Tmux {
            target: "session:0.1".into(),
            text: "W1-007".into()
        }
    );
    assert_eq!(
        tui::focus_delivery(Some("session:0.1"), false, "W1-007"),
        Delivery::PromptFile {
            text: "W1-007".into()
        },
        "tmux 不可用:落盘 prompt.txt(Windows 送对话通道)"
    );
    assert_eq!(
        tui::focus_delivery(None, true, "W1-007"),
        Delivery::PromptFile {
            text: "W1-007".into()
        },
        "无 DASH_TMUX_TARGET 同上"
    );
    assert_eq!(
        tui::focus_delivery(None, false, "W1-007"),
        Delivery::PromptFile {
            text: "W1-007".into()
        }
    );
}

#[test]
fn prompt_file_written_overwrite() {
    static SERIAL: AtomicU32 = AtomicU32::new(0);
    let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "agentdash-w2-008-prompt-{}-{serial}",
        std::process::id()
    ));
    let path = tui::write_prompt_file(&dir, "W2-008").expect("首次写入成功");
    assert_eq!(
        path,
        dir.join(".agentdash").join("prompt.txt"),
        "落点恒为 <repo>/.agentdash/prompt.txt"
    );
    assert_eq!(fs::read_to_string(&path).expect("read"), "W2-008");
    tui::write_prompt_file(&dir, "T-next").expect("重写成功");
    assert_eq!(
        fs::read_to_string(&path).expect("read"),
        "T-next",
        "覆盖写,不追加不残留"
    );
    assert!(fs::remove_dir_all(&dir).is_ok());
}

// ---------- SGR → Span 换算(差分重绘的结构化文本) ----------

#[test]
fn plain_line_is_single_span() {
    let spans = tui::spans_from_ansi("agentdash · DAG");
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].content, "agentdash · DAG");
    assert_eq!(spans[0].style.fg, None);
}

#[test]
fn sgr_codes_become_styles_and_are_stripped() {
    let green = tui::spans_from_ansi("\x1b[32m✓ T1 x\x1b[0m plain");
    assert_eq!(green[0].style.fg, Some(Color::Green), "done 绿");
    assert_eq!(green[1].content, " plain");
    assert_eq!(green[1].style.fg, None, "reset 后回归默认");

    let bold = tui::spans_from_ansi("\x1b[1m标题\x1b[0m");
    assert!(bold[0].style.add_modifier.contains(Modifier::BOLD));

    let pending = tui::spans_from_ansi("\x1b[90m· T2\x1b[0m");
    assert_eq!(pending[0].style.fg, Some(Color::DarkGray), "pending 暗灰");

    let unknown = tui::spans_from_ansi("\x1b[7mreverse\x1b[0m");
    assert_eq!(unknown[0].content, "reverse", "未知码只剥除不着色");
    assert_eq!(unknown[0].style.fg, None);
}

// ---------- barriers 入模与双轨收敛 ----------

#[test]
fn merge_fills_barriers_from_ledger() {
    static SERIAL: AtomicU32 = AtomicU32::new(0);
    let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "agentdash-w1-007-barriers-{}-{serial}",
        std::process::id()
    ));
    let dot = dir.join(".agentdash");
    fs::create_dir_all(&dot).expect("create fixture dir");
    fs::write(
        dot.join("ledger.json"),
        r#"{
  "$schema": "agentdash.tasklog.v1",
  "wave": "W25",
  "title": "并行车道冲刺",
  "lanes": [{"name": "A", "tasks": ["T1"]}, {"name": "B", "tasks": ["T2"]}],
  "tasks": {
    "T1": {"label": "fiber join", "state": "done"},
    "T2": {"label": "scope by_id", "state": "active"}
  },
  "barriers": [
    {"id": "B1", "after": ["T1"], "unlocks": ["T2"]},
    {"id": "B2", "after": ["T2"], "unlocks": []}
  ]
}"#,
    )
    .expect("write ledger.json");

    let dash = model::merge(&dir);

    assert_eq!(dash.barriers.len(), 2, "台账 barriers 转换进模型");
    assert_eq!(dash.barriers[0].after, vec!["T1".to_owned()]);
    assert_eq!(dash.barriers[0].unlocks, vec!["T2".to_owned()]);
    assert_eq!(dash.barriers[1].after, vec!["T2".to_owned()]);
    assert_eq!(dash.barriers[1].unlocks, Vec::<String>::new());
    assert!(fs::remove_dir_all(&dir).is_ok());
}

#[test]
fn default_render_graph_consumes_model_barriers() {
    let dash = w25_dash();
    let layers = layout(&dash);
    let t3 = cell_of(&layers, "T3");
    let t4 = cell_of(&layers, "T4");
    assert!(t4.line > t3.line, "车道互不重叠,层间差只能来自模型屏障边");
    // 双轨已收敛(W2-008):显式入口 `render_graph_with` 删除,默认单参版唯一
    let plain = strip_ansi(&render::graph::render_graph(
        &dash,
        render::graph::DEFAULT_GRAPH_WIDTH,
    ));
    // 屏障真正进了画图:T4 框顶有入线箭头(无边时孤立节点无 ▼)
    let rows: Vec<&str> = plain.lines().collect();
    assert!(
        rows[t4.line - 1].contains('▼'),
        "默认入口消费模型屏障:T4 顶有入线箭头"
    );
}

// ---------- 窄终端形态退化(AD-ERR-004) ----------

#[test]
fn narrow_terminal_degrades_to_oneline() {
    // 原始 39 列(钳位下限之下):决策必须先于钳位——先钳到 40 就看不出
    // 终端本来就窄,框化视图只会顶穿终端
    assert_eq!(
        tui::output_form(Some(39), render::DEFAULT_PANEL_WIDTH),
        tui::OutputForm::OneLine
    );
    // 单帧输出退化为 oneline:单行、含 "[dash]"、无框字符(框化破图)
    let out = strip_ansi(&tui::once_output(
        &w25_dash(),
        Some(39),
        render::DEFAULT_PANEL_WIDTH,
    ));
    assert!(out.contains("[dash]"), "窄终端输出 oneline 形态");
    assert_eq!(out.lines().count(), 1, "oneline 恰一行");
    assert!(!out.contains('┌'), "不得含框字符(框化视图必破图)");
}

#[test]
fn framed_widths_clamped_and_nontty_takes_default() {
    // 40 列起(含边界)仍出框化视图,宽度钳 40..120
    assert_eq!(
        tui::output_form(Some(40), render::DEFAULT_PANEL_WIDTH),
        tui::OutputForm::Framed(40)
    );
    assert_eq!(
        tui::output_form(Some(200), render::graph::DEFAULT_GRAPH_WIDTH),
        tui::OutputForm::Framed(120)
    );
    // 非 tty(管道/重定向)无列数:按 default 档走框化(冒烟路径不缩水)
    assert_eq!(
        tui::output_form(None, render::DEFAULT_PANEL_WIDTH),
        tui::OutputForm::Framed(render::DEFAULT_PANEL_WIDTH)
    );
    let framed = strip_ansi(&tui::once_output(
        &w25_dash(),
        None,
        render::DEFAULT_PANEL_WIDTH,
    ));
    assert!(framed.lines().count() > 1, "非 tty 冒烟仍出整帧面板");
}

// ---------- W2-003 详情面板(纯函数快照/截断) ----------

/// 富字段任务样例:`note`/`fix_round`/`since` 齐备,状态走富态 `FixRound`(⚑)。
fn detail_dash() -> Dashboard {
    Dashboard {
        tasks: vec![TaskView {
            id: "T2".into(),
            label: "scope by_id 索引".into(),
            state: TaskState::FixRound,
            lane: Some("B".into()),
            note: Some("fix round 2/5".into()),
            fix_round: Some((2, 5)),
            since: Some("2026-09-13T08:30:00Z".into()),
            done_at: None,
        }],
        milestones: Vec::new(),
        warnings: Vec::new(),
        barriers: Vec::new(),
        git: GitFacts::absent(),
        agents: Vec::new(),
        gates: vec![
            GateView {
                name: "pre-merge".into(),
                state: "passed".into(),
                detail: "test+clippy".into(),
                unknown: false,
            },
            GateView {
                name: "ship".into(),
                state: "failed".into(),
                detail: "1 red".into(),
                unknown: false,
            },
        ],
        remote: None,
        event_span_secs: None,       // W3-006:速度线事件活动窗;TUI 样例默认无
        project: "agentdash".into(), // W3-004 D3:项目名上模型
        // W4-002:详情事件尾样例(agent 派发 + gate 通过,到达序)
        event_tail: vec![
            EventTailView {
                kind: "agent".into(),
                name: "explore".into(),
                state: "dispatched".into(),
                ts: "2026-09-13T08:30:00Z".into(),
            },
            EventTailView {
                kind: "gate".into(),
                name: "test".into(),
                state: "passed".into(),
                ts: "2026-09-13T09:00:00Z".into(),
            },
        ],
        events_present: false, // W5-001:物证窗;渲染组样例默认无
        last_gate_passed: None,
        generated_at: GENERATED_AT.into(),
    }
}

#[test]
fn detail_lines_snapshot_fields_complete() {
    let dash = detail_dash();
    let lines: Vec<String> = tui::detail_lines(&dash.tasks[0], &dash, 80)
        .iter()
        .map(|line| strip_ansi(line))
        .collect();
    assert_eq!(lines[0], "标签  scope by_id 索引");
    assert_eq!(lines[1], "状态  ⚑ fix-round");
    assert_eq!(lines[2], "车道  B");
    assert_eq!(lines[3], "备注  fix round 2/5");
    assert_eq!(lines[4], "轮次  R2/5");
    assert_eq!(lines[5], "时刻  2026-09-13T08:30:00Z");
    assert_eq!(lines[6], "物证  -", "非 done 任务物证不可断言(W5-001)");
    assert_eq!(lines[7], "验证门");
    assert_eq!(lines[8], "  ✓ pre-merge passed · test+clippy");
    assert_eq!(lines[9], "  ✗ ship failed · 1 red");
    assert_eq!(lines[10], "事件");
    // W4-002:事件尾真数据(agent 派发 ▶ + gate 通过 ✓,到达序,ts 切 MM-DDTHH:MM)
    assert_eq!(lines[11], "  ▶ explore dispatched · 09-13T08:30");
    assert_eq!(lines[12], "  ✓ test passed · 09-13T09:00");
    assert_eq!(
        lines.len(),
        13,
        "字段齐:8 字段 + 门区块 + 事件区块(尾 2 条)"
    );
}

#[test]
fn detail_lines_absent_fields_show_dash_placeholder() {
    let dash = w25_dash(); // T1..T4 无 note/fix_round/since,无 gates
    let lines: Vec<String> = tui::detail_lines(&dash.tasks[0], &dash, 80)
        .iter()
        .map(|line| strip_ansi(line))
        .collect();
    assert_eq!(lines[0], "标签  fiber join 即回收");
    assert_eq!(lines[1], "状态  ✓ done");
    assert_eq!(lines[2], "车道  A");
    assert_eq!(lines[3], "备注  -");
    assert_eq!(lines[4], "轮次  -");
    assert_eq!(lines[5], "时刻  -");
    assert_eq!(lines[8], "  -", "无 gate 时显示占位,整段不消失");
}

#[test]
fn detail_empty_tail_shows_dash_placeholder() {
    // W4-002:无事件尾时空态占位 `-`(整段不消失),旧占位行(事件层 W2-008
    // 接入)已退役
    let dash = w25_dash();
    let lines: Vec<String> = tui::detail_lines(&dash.tasks[0], &dash, 80)
        .iter()
        .map(|line| strip_ansi(line))
        .collect();
    // w25 样例无 gates:验证门段只占 7/8 两行,事件段自 9 起(物证字段占 6)
    assert_eq!(lines[9], "事件");
    assert_eq!(lines[10], "  -", "无事件尾时空态占位");
    assert!(
        !lines.iter().any(|l| l.contains("W2-008")),
        "旧占位行必须消失: {lines:?}"
    );
}

#[test]
fn detail_lines_elide_to_budget() {
    let dash = detail_dash();
    let lines = tui::detail_lines(&dash.tasks[0], &dash, 20);
    assert_eq!(lines.len(), 13, "截断只裁行宽,不裁行数");
    for line in &lines {
        let plain = strip_ansi(line);
        assert!(
            render::display_width(&plain) <= 20,
            "行超显示宽预算:{plain}"
        );
    }
    assert!(
        lines.iter().any(|line| strip_ansi(line).ends_with('…')),
        "长字段截断以省略号收尾"
    );
}

// ---------- W2-003/W2-004 新键位(d/?/Esc/↑/↓/⏎) ----------

#[test]
fn detail_help_wave_key_map() {
    let none = KeyModifiers::NONE;
    assert_eq!(
        tui::key_action(InputMode::Normal, key(KeyCode::Char('d'), none)),
        Action::MarkDone
    );
    assert_eq!(
        tui::key_action(InputMode::Normal, key(KeyCode::Char('?'), none)),
        Action::Help
    );
    assert_eq!(
        tui::key_action(InputMode::Normal, key(KeyCode::Esc, none)),
        Action::Back
    );
    assert_eq!(
        tui::key_action(InputMode::Normal, key(KeyCode::Up, none)),
        Action::PrevWave
    );
    assert_eq!(
        tui::key_action(InputMode::Normal, key(KeyCode::Down, none)),
        Action::NextWave
    );
}

// ---------- W2-004 多波次滚动(纯函数过滤/钳位) ----------

/// 双波次样例:W2 波任务以波次串为 id 前缀段(W2-003/W2-004);历史波次 W1
/// 的关联任务全不匹配(回退全量,现行单账本即单波的形态)。
fn wave_dash() -> Dashboard {
    Dashboard {
        tasks: vec![
            task("W2-003", "详情面板", "A"),
            task("W2-004", "波次滚动", "B"),
            task("T9", "发布件", "C"),
        ],
        milestones: vec![
            MilestoneView {
                wave: Some("W1".into()),
                title: "一期".into(),
                done: 3,
                total: 3,
            },
            MilestoneView {
                wave: Some("W2".into()),
                title: "二期".into(),
                done: 0,
                total: 2,
            },
        ],
        warnings: Vec::new(),
        barriers: vec![BarrierEdges {
            after: vec!["W2-003".into()],
            unlocks: vec!["W2-004".into()],
        }],
        git: GitFacts::absent(),
        agents: Vec::new(),
        gates: Vec::new(),
        remote: None,
        event_span_secs: None,       // W3-006:速度线事件活动窗;TUI 样例默认无
        project: "agentdash".into(), // W3-004 D3:项目名上模型
        event_tail: Vec::new(),
        events_present: false, // W5-001:物证窗;渲染组样例默认无
        last_gate_passed: None,
        generated_at: GENERATED_AT.into(),
    }
}

#[test]
fn select_wave_filters_tasks_and_clamps() {
    let dash = wave_dash();
    let (idx, visible) = tui::select_wave(&dash, 1);
    assert_eq!(idx, 1);
    let ids: Vec<&str> = visible.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["W2-003", "W2-004"],
        "选中 W2:仅前缀段匹配任务可见"
    );

    // 历史波次 W1 无关联任务:回退全量(宁可多显示不少显示)
    let (_, visible) = tui::select_wave(&dash, 0);
    assert_eq!(visible.len(), 3, "全不匹配回退全量");

    let (idx, _) = tui::select_wave(&dash, 99);
    assert_eq!(idx, 1, "越界钳位到末波");

    let bare = empty_dash();
    let (idx, visible) = tui::select_wave(&bare, 7);
    assert_eq!(idx, 0, "无里程碑索引归零");
    assert!(visible.is_empty());
}

#[test]
fn wave_view_swaps_only_tasks() {
    let dash = wave_dash();
    let view = tui::wave_view(&dash, 1);
    let ids: Vec<&str> = view.tasks.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["W2-003", "W2-004"],
        "渲染视图按选中波次折算任务集"
    );
    assert_eq!(view.milestones, dash.milestones, "里程碑原样保留");
    assert_eq!(
        view.barriers, dash.barriers,
        "屏障原样保留(图侧自滤未知 id)"
    );
    assert_eq!(view.git, dash.git, "git 快照原样保留");
}

#[test]
fn wave_tag_marks_current_over_total() {
    let dash = wave_dash();
    assert_eq!(tui::wave_tag(&dash, 0), "波次 W1 1/2");
    assert_eq!(tui::wave_tag(&dash, 1), "波次 W2 2/2");
    assert_eq!(tui::wave_tag(&dash, 99), "波次 W2 2/2", "越界钳位");
    assert_eq!(tui::wave_tag(&w25_dash(), 0), "波次 W25 1/1");
    assert_eq!(tui::wave_tag(&empty_dash(), 0), "", "无波次不标注");
}

// ---------- W2-004 帮助覆盖层(全键位) ----------

#[test]
fn help_lines_cover_all_keys() {
    let help = tui::help_lines().join("\n");
    for token in [
        "g", "f", "c", "⏎", "d", "Esc", "↑", "↓", "?", "q", "/", "tab",
    ] {
        assert!(help.contains(token), "帮助缺键位标注:{token}");
    }
    let lines = tui::help_lines();
    assert!(
        lines.iter().any(|line| line.starts_with("d ")),
        "详情键独立条目"
    );
    assert!(
        lines.iter().any(|line| line.starts_with("? ")),
        "帮助键独立条目"
    );
    assert!(
        lines.iter().any(|line| line.starts_with("/ ")),
        "过滤键独立条目(W2-005)"
    );
    assert!(
        lines.iter().any(|line| line.starts_with("tab ")),
        "折叠键独立条目(W2-005)"
    );
    assert!(
        lines.iter().any(|line| line.contains("任意键关闭")),
        "关闭方式可见"
    );
}

// ---------- W2-005 过滤(纯函数) ----------

/// 过滤/折叠样例:alpha 车道全 done(2 任务,成员不相邻)、beta 车道进行中
/// (1 active)、gamma 车道单任务 done,另有 1 条无车道 pending 任务。
fn filter_dash() -> Dashboard {
    let t1 = task("T1", "fiber join", "alpha");
    let mut t2 = task("T2", "scope 索引", "beta");
    t2.state = TaskState::Active;
    let t3 = task("T3", "发布件", "alpha");
    let t4 = task("T4", "orphan 清理", "gamma");
    let t5 = TaskView {
        id: "T9".into(),
        label: "无车道任务".into(),
        state: TaskState::Pending,
        lane: None,
        note: None,
        fix_round: None,
        since: None,
        done_at: None,
    };
    Dashboard {
        tasks: vec![t1, t2, t3, t4, t5],
        milestones: Vec::new(),
        warnings: Vec::new(),
        barriers: Vec::new(),
        git: GitFacts::absent(),
        agents: Vec::new(),
        gates: Vec::new(),
        remote: None,
        event_span_secs: None,       // W3-006:速度线事件活动窗;TUI 样例默认无
        project: "agentdash".into(), // W3-004 D3:项目名上模型
        event_tail: Vec::new(),
        events_present: false, // W5-001:物证窗;渲染组样例默认无
        last_gate_passed: None,
        generated_at: GENERATED_AT.into(),
    }
}

#[test]
fn filter_tasks_substring_matches_lane_state_label() {
    let dash = filter_dash();
    let ids = |query: &str| -> Vec<String> {
        tui::filter_tasks(&dash, query)
            .into_iter()
            .map(|task| task.id.clone())
            .collect()
    };
    assert_eq!(ids("alpha"), vec!["T1", "T3"], "按 lane 名子串匹配");
    assert_eq!(ids("scope"), vec!["T2"], "按 label 子串匹配");
    assert_eq!(ids("active"), vec!["T2"], "按 state 名子串匹配");
    assert_eq!(
        ids("DONE"),
        vec!["T1", "T3", "T4"],
        "大小写不敏感:done 命中全部完成态任务"
    );
    assert_eq!(ids("Scope"), vec!["T2"], "label 匹配同样大小写不敏感");
    assert_eq!(ids("").len(), 5, "空查询全通过");
    assert!(ids("nomatch").is_empty(), "无命中返回空集");
}

#[test]
fn filter_view_swaps_only_tasks() {
    let dash = filter_dash();
    let view = tui::filter_view(&dash, "active");
    assert_eq!(view.tasks.len(), 1, "过滤视图只留命中任务");
    assert_eq!(view.tasks[0].id, "T2");
    assert_eq!(view.milestones, dash.milestones, "里程碑原样保留");
    assert_eq!(view.barriers, dash.barriers, "屏障原样保留");
    assert_eq!(view.git, dash.git, "git 快照原样保留");
    assert_eq!(
        tui::filter_view(&dash, "").tasks,
        dash.tasks,
        "空查询:视图任务集与原模型一致"
    );
}

// ---------- W2-005 车道折叠(纯函数) ----------

#[test]
fn done_lanes_groups_complete_lanes_in_first_seen_order() {
    let lanes = tui::done_lanes(&filter_dash().tasks);
    assert_eq!(
        lanes,
        vec![("alpha".to_owned(), 2), ("gamma".to_owned(), 1)],
        "只收全 done 车道,按首见序,计数为 done 任务数"
    );
}

#[test]
fn collapse_view_swaps_done_lanes_for_markers() {
    let dash = filter_dash();
    let view = tui::collapse_view(&dash, tui::LaneCollapse::DoneLanes);
    let marker = &view.tasks[0];
    assert!(marker.id.is_empty(), "折叠伪任务以空 id 为哨兵");
    assert_eq!(marker.label, "(2 done)", "伪任务 label 携带 done 计数");
    assert_eq!(marker.lane.as_deref(), Some("alpha"));
    let ids: Vec<&str> = view.tasks.iter().map(|task| task.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["", "T2", "", "T9"],
        "完成车道折叠为单标记(原首成员位),其余任务原样"
    );
    assert_eq!(
        tui::collapse_view(&dash, tui::LaneCollapse::All).tasks,
        dash.tasks,
        "全部展开态:视图与原模型一致"
    );
}

#[test]
fn collapse_mode_cycles() {
    assert_eq!(
        tui::LaneCollapse::All.next(),
        tui::LaneCollapse::DoneLanes,
        "全部展开 → 折叠完成车道"
    );
    assert_eq!(
        tui::LaneCollapse::DoneLanes.next(),
        tui::LaneCollapse::All,
        "循环回全部展开"
    );
}

// ---------- W2-005 页眉与空结果提示(纯函数) ----------

#[test]
fn header_line_shows_filter_and_collapse() {
    assert_eq!(
        tui::header_line("", tui::LaneCollapse::All),
        None,
        "无过滤且全展开:不占页眉行"
    );
    assert_eq!(
        tui::header_line("abc", tui::LaneCollapse::All).as_deref(),
        Some("filter:\"abc\"")
    );
    assert_eq!(
        tui::header_line("", tui::LaneCollapse::DoneLanes).as_deref(),
        Some("折叠:完成车道")
    );
    assert_eq!(
        tui::header_line("a b", tui::LaneCollapse::DoneLanes).as_deref(),
        Some("filter:\"a b\" │ 折叠:完成车道")
    );
    assert_eq!(
        tui::filter_empty_notice("xyz"),
        "filter:\"xyz\" 无匹配任务",
        "空结果显式提示"
    );
}

// ---------- W2-005 新键位(/ tab)与过滤输入态 ----------

#[test]
fn filter_collapse_key_map() {
    let none = KeyModifiers::NONE;
    assert_eq!(
        tui::key_action(InputMode::Normal, key(KeyCode::Char('/'), none)),
        Action::StartFilter
    );
    assert_eq!(
        tui::key_action(InputMode::Normal, key(KeyCode::Tab, none)),
        Action::ToggleCollapse
    );
    // 过滤输入态与聚焦编辑态同一套行编辑语义
    assert_eq!(
        tui::key_action(InputMode::Filter, key(KeyCode::Char('x'), none)),
        Action::Input('x')
    );
    assert_eq!(
        tui::key_action(InputMode::Filter, key(KeyCode::Backspace, none)),
        Action::Erase
    );
    assert_eq!(
        tui::key_action(InputMode::Filter, key(KeyCode::Enter, none)),
        Action::Submit,
        "⏎ 应用过滤"
    );
    assert_eq!(
        tui::key_action(InputMode::Filter, key(KeyCode::Esc, none)),
        Action::Cancel,
        "Esc 取消过滤输入"
    );
    assert_eq!(
        tui::key_action(
            InputMode::Filter,
            key(KeyCode::Char('c'), KeyModifiers::CONTROL)
        ),
        Action::Quit,
        "过滤态 Ctrl-C 仍退出"
    );
}

// ---------- W2-008 watch 位置参数(纯函数) ----------

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn parse_watch_args_accepts_both_forms() {
    let default_interval = tui::MODEL_INTERVAL;
    assert_eq!(
        tui::parse_watch_args(&args(&[])),
        Ok((false, default_interval, PathBuf::from("."))),
        "`watch`:默认档 + 当前目录"
    );
    assert_eq!(
        tui::parse_watch_args(&args(&["plan"])),
        Ok((false, default_interval, PathBuf::from("plan"))),
        "`watch [PATH]`:非数字首参是路径"
    );
    assert_eq!(
        tui::parse_watch_args(&args(&["30"])),
        Ok((false, 30, PathBuf::from("."))),
        "`watch [SECONDS]`:数字首参是间隔"
    );
    assert_eq!(
        tui::parse_watch_args(&args(&["30", "plan"])),
        Ok((false, 30, PathBuf::from("plan"))),
        "`watch [SECONDS] [PATH]` 两形态"
    );
    assert_eq!(
        tui::parse_watch_args(&args(&["--once", "2", "p"])),
        Ok((true, 2, PathBuf::from("p"))),
        "--once 与位置参数可混用"
    );
}

#[test]
fn parse_watch_args_clamps_interval() {
    assert_eq!(
        tui::parse_watch_args(&args(&["0"])),
        Ok((false, 1, PathBuf::from("."))),
        "下钳 1 秒"
    );
    assert_eq!(
        tui::parse_watch_args(&args(&["99999"])),
        Ok((false, 3600, PathBuf::from("."))),
        "上钳 3600 秒"
    );
    assert_eq!(
        tui::parse_watch_args(&args(&["99999999999999999999999"])),
        Ok((false, 3600, PathBuf::from("."))),
        "溢出数字仍按间隔钳到上限,不落成路径"
    );
}

#[test]
fn parse_watch_args_rejects_bad_input() {
    assert!(
        tui::parse_watch_args(&args(&["--wat"])).is_err(),
        "未知旗标报错"
    );
    assert!(
        tui::parse_watch_args(&args(&["a", "b", "c"])).is_err(),
        "三个位置参数超出两形态"
    );
    assert!(
        tui::parse_watch_args(&args(&["plan", "30"])).is_err(),
        "PATH 之后再跟位置参数不合两形态"
    );
}

// ------------------------------------------------------------ 物证(W5-001)

#[test]
fn detail_attestation_lines_cover_all_states() {
    // done + done_at + 事件窗在场:四种物证文案逐一核对(✓/?/无通过门/无事件窗)
    let mut dash = detail_dash();
    dash.events_present = true;
    dash.tasks[0].state = TaskState::Done;
    dash.tasks[0].done_at = Some("2026-09-13T10:00:00Z".into());
    dash.last_gate_passed = Some("2026-09-13T09:00:00Z".into());

    let lines: Vec<String> = tui::detail_lines(&dash.tasks[0], &dash, 80)
        .iter()
        .map(|line| strip_ansi(line))
        .collect();
    assert!(
        lines[6].starts_with("物证  ? 晚于最近通过门(09-13T09:00)"),
        "自报晚于通过门 → ?: {}",
        lines[6]
    );

    // 自报早于通过门 → ✓
    dash.tasks[0].done_at = Some("2026-09-13T08:00:00Z".into());
    let lines: Vec<String> = tui::detail_lines(&dash.tasks[0], &dash, 80)
        .iter()
        .map(|line| strip_ansi(line))
        .collect();
    assert!(
        lines[6].starts_with("物证  ✓ 早于最近通过门(09-13T09:00)"),
        "自报早于通过门 → ✓: {}",
        lines[6]
    );

    // 事件窗在场但从未有通过门 → ?(无物证)
    dash.tasks[0].done_at = Some("2026-09-13T08:00:00Z".into());
    dash.last_gate_passed = None;
    let lines: Vec<String> = tui::detail_lines(&dash.tasks[0], &dash, 80)
        .iter()
        .map(|line| strip_ansi(line))
        .collect();
    assert!(
        lines[6].starts_with("物证  ? 自报无物证"),
        "事件在场无通过门 → ?: {}",
        lines[6]
    );

    // 无事件窗:done_at 在场也只能挂起(无证可查不作怀疑)
    dash.events_present = false;
    let lines: Vec<String> = tui::detail_lines(&dash.tasks[0], &dash, 80)
        .iter()
        .map(|line| strip_ansi(line))
        .collect();
    assert!(
        lines[6].starts_with("物证  done_at 2026-09-13T08:00:00Z(无事件窗可核)"),
        "无事件窗 → 挂起不怀疑: {}",
        lines[6]
    );
}

#[test]
fn scroll_step_clamps_to_content_and_zero() {
    // W12-010:上滚到顶即停,下滚到 `内容 - 视口` 即停
    assert_eq!(tui::scroll_step(0, true, 100, 10), 0, "顶不减");
    assert_eq!(tui::scroll_step(3, true, 100, 10), 2, "上滚向顶");
    assert_eq!(tui::scroll_step(3, false, 100, 10), 4, "下滚向底");
    assert_eq!(tui::scroll_step(90, false, 100, 10), 90, "底即钳住");
    assert_eq!(tui::scroll_step(90, false, 5, 10), 0, "内容不足一屏滚不动");
    assert_eq!(tui::scroll_max(100, 10), 90, "上限 = 内容 - 视口");
    assert_eq!(tui::scroll_max(5, 10), 0, "内容短于视口上限 0");
}
