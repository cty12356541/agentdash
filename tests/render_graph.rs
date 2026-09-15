//! W1-006 渲染移植集成测试:graph 视图黄金断言(复刻 claude-dash
//! `test_dash_graph_v2.py` 的 w25 样例语义)。
//!
//! agentdash 是纯二进制 crate,集成测试按 `#[path]` 在 crate 根挂载模块树,
//! 与 `tests/merge.rs` 同约定。挂载的 model/contract/events/sources 各源
//! 以 `merge` 链路为主;本组测试手工构造 `Dashboard`(精确控制黄金样例),
//! 未触达的 pub 项在此 crate 属死代码,按文件级 allow 放行。
//!
//! 屏障集经 `graph::BarrierEdges` 显式传入(黄金样例需精确控制边集;
//! `Dashboard` 已随 T7 携带 barriers,消费模型屏障的默认单参入口见
//! `graph::render_graph`)。

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

use contract::TaskState;
use model::{BarrierEdges as ModelBarrierEdges, Dashboard, MilestoneView, TaskView};
use render::display_width;
use render::graph::{
    BarrierEdges, Cell, DEFAULT_GRAPH_WIDTH, barriers_of, hit_test, layout_layers, render_graph,
};
use sources::git::GitFacts;

/// 接合符须含上笔承接父竖线(承 Python `UP_STROKES`)。
const UP_STROKES: [char; 4] = ['└', '┘', '┴', '│'];

fn task(id: &str, label: &str, lane: &str) -> TaskView {
    TaskView {
        id: id.into(),
        label: label.into(),
        state: TaskState::Done,
        lane: Some(lane.into()),
        note: None,
        fix_round: None,
        since: None,
    }
}

/// w25 形态样例:T1→T2→T4 链 + T3 并行跨层汇入 T4(屏障 T1→T2、T2+T3→T4)。
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
        barriers: w25_barriers(),
        // W3-004 D3 起:图标题项目名走 `project` 字段(见下),不再读
        // `GitFacts::root`;此处钉固保留为零行为夹具,防口径回摆时再度
        // 隐性依赖测试 cwd 恰名 agentdash
        git: GitFacts {
            root: Some("agentdash".to_owned()),
            ..GitFacts::absent()
        },
        agents: Vec::new(),
        remote: None,
        gates: Vec::new(),
        event_span_secs: None,       // W3-006:速度线事件活动窗;图样例默认无
        project: "agentdash".into(), // W3-004 D3:项目名上模型
        event_tail: Vec::new(),      // W4-002:详情事件尾;渲染组样例默认无
        generated_at: "2026-09-13T08:30:00Z".into(),
    }
}

/// 屏障集:屏障 B1: T1 → T2;屏障 B2: T2+T3 → T4(复刻 `_w25_like`)。
fn w25_barriers() -> Vec<ModelBarrierEdges> {
    vec![
        ModelBarrierEdges {
            after: vec!["T1".into()],
            unlocks: vec!["T2".into()],
        },
        ModelBarrierEdges {
            after: vec!["T2".into(), "T3".into()],
            unlocks: vec!["T4".into()],
        },
    ]
}

/// 剥 ANSI 转义(颜色码 `\x1b[...m`,参数为数字/分号,终止于字母)。
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

/// 按显示列取字符(宽字符占 2 列:命中其首列返回该字符,次列视作空白)。
fn at(line: &str, col: usize) -> char {
    let mut x = 0usize;
    for ch in line.chars() {
        let w = display_width(&ch.to_string());
        if x <= col && col < x + w {
            return if col == x { ch } else { ' ' };
        }
        x += w;
    }
    ' '
}

fn plain_rows(dash: &Dashboard, _barriers: &[BarrierEdges]) -> Vec<String> {
    strip_ansi(&render_graph(dash, DEFAULT_GRAPH_WIDTH))
        .lines()
        .map(str::to_owned)
        .collect()
}

fn cell_of(layers: &[Vec<Cell>], id: &str) -> Cell {
    layers
        .iter()
        .flatten()
        .find(|cell| cell.id == id)
        .cloned()
        .unwrap_or_else(|| panic!("cell {id} not laid out"))
}

#[test]
fn every_task_has_framed_box() {
    let dash = w25_dash();
    let barriers = barriers_of(&dash);
    let layers = layout_layers(&dash, &barriers);
    let rows = plain_rows(&dash, &barriers);
    for tid in ["T1", "T2", "T3", "T4"] {
        let cell = cell_of(&layers, tid);
        let top = &rows[cell.line - 1];
        let txt = &rows[cell.line];
        let bot = &rows[cell.line + 1];
        assert_eq!(at(top, cell.x), '┌', "{tid} 框顶左");
        assert_eq!(at(top, cell.x + cell.width - 1), '┐', "{tid} 框顶右");
        assert_eq!(at(bot, cell.x), '└', "{tid} 框底左");
        assert_eq!(at(bot, cell.x + cell.width - 1), '┘', "{tid} 框底右");
        assert!(
            txt.contains(&format!("✓ {tid} ")),
            "{tid} 文字行含标记与 id"
        );
    }
}

#[test]
fn parent_bottom_has_out_stub() {
    let dash = w25_dash();
    let barriers = barriers_of(&dash);
    let layers = layout_layers(&dash, &barriers);
    let rows = plain_rows(&dash, &barriers);
    let c1 = cell_of(&layers, "T1");
    assert_eq!(
        at(&rows[c1.line + 1], c1.center()),
        '┬',
        "T1 有出边:框底出线桩"
    );
    let c4 = cell_of(&layers, "T4");
    assert_eq!(
        at(&rows[c4.line + 1], c4.center()),
        '─',
        "T4 无出边:框底纯 ─"
    );
}

#[test]
fn adjacent_edge_joins_parent_drop() {
    let dash = w25_dash();
    let barriers = barriers_of(&dash);
    let layers = layout_layers(&dash, &barriers);
    let rows = plain_rows(&dash, &barriers);
    let c1 = cell_of(&layers, "T1");
    let c2 = cell_of(&layers, "T2");
    assert_eq!(at(&rows[c1.line + 2], c1.center()), '│', "父框下竖线");
    assert!(
        UP_STROKES.contains(&at(&rows[c2.line - 2], c1.center())),
        "汇流行接合符承接父竖线"
    );
    assert_eq!(at(&rows[c2.line - 1], c2.center()), '▼', "箭头嵌框顶");
}

#[test]
fn cross_layer_edge_is_routed() {
    // 核心:T3(层0)→T4(层2) 的跨层汇入边必须画出来。
    let dash = w25_dash();
    let barriers = barriers_of(&dash);
    let layers = layout_layers(&dash, &barriers);
    let rows = plain_rows(&dash, &barriers);
    let c3 = cell_of(&layers, "T3");
    let c4 = cell_of(&layers, "T4");
    let c2 = cell_of(&layers, "T2");
    assert!(c4.line > c3.line + 5, "确为跨层");
    assert_eq!(at(&rows[c3.line + 1], c3.center()), '┬', "T3 出线桩");
    assert_eq!(at(&rows[c2.line], c3.center()), '│', "竖穿 T2 文字行");
    let z = c4.line - 2;
    assert!(
        UP_STROKES.contains(&at(&rows[z], c3.center())),
        "汇流行上 T3 中心处有含上笔的接合符"
    );
    let (lo, hi) = if c3.center() < c4.center() {
        (c3.center(), c4.center())
    } else {
        (c4.center(), c3.center())
    };
    for x in lo..=hi {
        assert!(
            rows[z]
                .chars()
                .nth(x)
                .is_some_and(|ch| "─└┘┴│".contains(ch)),
            "汇流行连通到 T4 中心列(col {x})"
        );
    }
    assert_eq!(at(&rows[c4.line - 1], c4.center()), '▼', "箭头嵌 T4 框顶");
}

#[test]
fn t4_receives_two_arrows_at_one_point() {
    let dash = w25_dash();
    let barriers = barriers_of(&dash);
    let layers = layout_layers(&dash, &barriers);
    let rows = plain_rows(&dash, &barriers);
    let c4 = cell_of(&layers, "T4");
    assert_eq!(
        rows[c4.line - 1].matches('▼').count(),
        1,
        "两入边同点单箭头"
    );
}

#[test]
fn cjk_label_fits_box() {
    // 东亚宽字符占 2 列:框宽必须按显示宽算,右框缘不被顶穿。
    let dash = Dashboard {
        tasks: vec![task("T1", "中文标签宽字符测试", "A")],
        milestones: Vec::new(),
        warnings: Vec::new(),
        barriers: w25_barriers(),
        git: GitFacts::absent(),
        agents: Vec::new(),
        remote: None,
        gates: Vec::new(),
        event_span_secs: None,       // W3-006:速度线事件活动窗;图样例默认无
        project: "agentdash".into(), // W3-004 D3:项目名上模型
        event_tail: Vec::new(),      // W4-002:详情事件尾;渲染组样例默认无
        generated_at: "2026-09-13T08:30:00Z".into(),
    };
    let layers = layout_layers(&dash, &[]);
    let rows = plain_rows(&dash, &[]);
    let cell = cell_of(&layers, "T1");
    assert_eq!(at(&rows[cell.line], cell.x), '│');
    assert_eq!(
        at(&rows[cell.line], cell.x + cell.width - 1),
        '│',
        "右框缘完好"
    );
    let text = "✓ T1 中文标签宽字符测试";
    assert_eq!(
        cell.width,
        display_width(text) + 4,
        "框宽 = 显示宽 + 内边距 2 + 边框 2"
    );
}

#[test]
fn cycle_remaining_merges_into_last_layer() {
    // 环保险:B→A 链 + A→B 屏障成环;环内节点迭代上限后并入末层并列,全量输出。
    let pending = |id: &str, label: &str, lane: &str| TaskView {
        id: id.into(),
        label: label.into(),
        state: TaskState::Pending,
        lane: Some(lane.into()),
        note: None,
        fix_round: None,
        since: None,
    };
    let dash = Dashboard {
        tasks: vec![
            pending("A", "任务甲", "L1"),
            pending("B", "任务乙", "L1"),
            pending("C", "任务丙", "L2"),
        ],
        milestones: Vec::new(),
        warnings: Vec::new(),
        barriers: w25_barriers(),
        git: GitFacts::absent(),
        agents: Vec::new(),
        remote: None,
        gates: Vec::new(),
        event_span_secs: None,       // W3-006:速度线事件活动窗;图样例默认无
        project: "agentdash".into(), // W3-004 D3:项目名上模型
        event_tail: Vec::new(),      // W4-002:详情事件尾;渲染组样例默认无
        generated_at: "2026-09-13T08:30:00Z".into(),
    };
    let barriers = vec![BarrierEdges {
        after: vec!["B".into()],
        unlocks: vec!["A".into()],
    }];
    let layers = layout_layers(&dash, &barriers);
    let a = cell_of(&layers, "A");
    let b = cell_of(&layers, "B");
    let c = cell_of(&layers, "C");
    assert_eq!(a.line, b.line, "环残余并入同一层并列");
    assert!(c.line < a.line, "无依赖节点在前层");
    let plain = strip_ansi(&render_graph(&dash, DEFAULT_GRAPH_WIDTH));
    for id in ["A", "B", "C"] {
        assert!(plain.contains(id), "环保险全量输出 {id}");
    }
}

#[test]
fn lane_chain_forms_layers() {
    let dash = Dashboard {
        tasks: vec![task("X2", "后手", "L"), task("X1", "先手", "L")],
        milestones: Vec::new(),
        warnings: Vec::new(),
        barriers: w25_barriers(),
        git: GitFacts::absent(),
        agents: Vec::new(),
        remote: None,
        gates: Vec::new(),
        event_span_secs: None,       // W3-006:速度线事件活动窗;图样例默认无
        project: "agentdash".into(), // W3-004 D3:项目名上模型
        event_tail: Vec::new(),      // W4-002:详情事件尾;渲染组样例默认无
        generated_at: "2026-09-13T08:30:00Z".into(),
    };
    let layers = layout_layers(&dash, &[]);
    let first = cell_of(&layers, "X1");
    let second = cell_of(&layers, "X2");
    assert_eq!(
        second.line - first.line,
        5,
        "同车道 id 序链:X1 层 0、X2 层 1(层距 = 框 3 行 + 空档 2 行)"
    );
}

#[test]
fn width_rules_and_clamp() {
    let dash = w25_dash();
    let _barriers = barriers_of(&dash);
    let lines: Vec<String> = render_graph(&dash, 46).lines().map(str::to_owned).collect();
    assert_eq!(lines[0], "agentdash · DAG");
    assert_eq!(lines[1], "─".repeat(46));
    assert_eq!(lines.last().expect("非空"), &"─".repeat(46));
    let narrow = render_graph(&dash, 10);
    assert_eq!(
        narrow.lines().nth(1).expect("非空").chars().count(),
        40,
        "宽度下钳 40"
    );
    let wide = render_graph(&dash, 500);
    assert_eq!(
        wide.lines().nth(1).expect("非空").chars().count(),
        120,
        "宽度上钳 120"
    );
}

#[test]
fn default_layout_fits_within_width() {
    let dash = w25_dash();
    let plain = strip_ansi(&render_graph(&dash, DEFAULT_GRAPH_WIDTH));
    for line in plain.lines() {
        assert!(
            display_width(line) <= DEFAULT_GRAPH_WIDTH,
            "零溢出: {line:?}"
        );
    }
}

#[test]
fn hit_test_targets_box_region() {
    let dash = w25_dash();
    let barriers = barriers_of(&dash);
    let layers = layout_layers(&dash, &barriers);
    let c2 = cell_of(&layers, "T2");
    for row in [c2.line - 1, c2.line, c2.line + 1] {
        assert_eq!(
            hit_test(&layers, c2.x + 2, row + 1).as_deref(),
            Some("T2"),
            "框三行整框可命中"
        );
    }
    assert_eq!(
        hit_test(&layers, c2.x + c2.width + 1, c2.line + 1).as_deref(),
        None,
        "框外不命中"
    );
}

#[test]
fn done_tasks_render_green_with_ansi() {
    let dash = w25_dash();
    let _barriers = barriers_of(&dash);
    let out = render_graph(&dash, DEFAULT_GRAPH_WIDTH);
    assert!(out.contains("\x1b[32m"), "done 绿");
    assert!(out.contains('▼') && out.contains('│'), "连接符在位");
}

// ---------- 屏障分流:W3-002 起 panel 承载屏障行,oneline 仍不进 ----------

#[test]
fn barriers_reach_panel_but_not_oneline() {
    let with_bars = w25_dash();
    let mut without = w25_dash();
    without.barriers.clear();
    // W3-002 起面板渲染屏障行:有屏障出 ⇕ 行、无屏障零残留(精确形态
    // 断言在 tests/render_panel.rs);oneline 仍不承载屏障——有无屏障
    // 输出逐字一致
    let with_panel = strip_ansi(&render::render_panel(
        &with_bars,
        render::DEFAULT_PANEL_WIDTH,
    ));
    let without_panel = strip_ansi(&render::render_panel(&without, render::DEFAULT_PANEL_WIDTH));
    assert!(
        with_panel.contains('⇕'),
        "panel 输出随 barriers 出屏障行: {with_panel}"
    );
    assert!(
        !without_panel.contains('⇕'),
        "无屏障零残留: {without_panel}"
    );
    assert_eq!(
        render::render_oneline(&with_bars),
        render::render_oneline(&without),
        "oneline 输出不随 barriers 变化"
    );
}
