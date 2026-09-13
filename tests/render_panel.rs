//! W1-006 渲染移植集成测试:panel / oneline 视图黄金断言。
//!
//! agentdash 是纯二进制 crate,集成测试按 `#[path]` 在 crate 根挂载模块树,
//! 与 `tests/merge.rs` 同约定。挂载的 model/contract/events/sources 各源
//! 以 `merge` 链路为主;本组测试手工构造 `Dashboard`(精确控制黄金样例),
//! 未触达的 pub 项在此 crate 属死代码,按文件级 allow 放行。

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

use std::collections::HashMap;

use contract::TaskState;
use events::GateState;
use model::{Dashboard, MilestoneView, TaskView};
use render::{DEFAULT_PANEL_WIDTH, display_width, render_oneline, render_panel};
use sources::git::GitFacts;

fn task(id: &str, label: &str, lane: &str) -> TaskView {
    TaskView {
        id: id.into(),
        label: label.into(),
        state: TaskState::Done,
        lane: Some(lane.into()),
        note: None,
    }
}

/// w25 形态样例:4 任务全 done(2 done + 2 active 变体在用例内改写)。
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
        gates: HashMap::new(),
        git: GitFacts::absent(),
        generated_at: "2026-09-13T08:30:00Z".into(),
    }
}

fn dash_with(tasks: Vec<TaskView>) -> Dashboard {
    Dashboard {
        tasks,
        milestones: Vec::new(),
        warnings: Vec::new(),
        gates: HashMap::new(),
        git: GitFacts::absent(),
        generated_at: "2026-09-13T08:30:00Z".into(),
    }
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

#[test]
fn golden_w25_panel() {
    let dash = w25_dash();
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    let lines: Vec<&str> = plain.lines().collect();
    assert_eq!(
        lines[0], "agentdash",
        "页眉:4/4 全完成非活跃里程碑,仅项目名"
    );
    assert_eq!(
        lines[1], "✓4 ▶0 ·0 ⚑0 · 0 agents · 09-13T08:30",
        "统计行:计数 + agents(模型缺字段恒 0)+ 生成时刻"
    );
    assert_eq!(lines[2], "═".repeat(DEFAULT_PANEL_WIDTH), "区块分隔");
    assert!(plain.contains("在跑 / 健康"));
    assert!(plain.contains("✓ 无活跃/卡死"));
    assert!(plain.contains("轨迹 · 1 里程碑"));
    assert!(
        plain.contains("  W25 并行车道冲刺 ▓▓▓▓▓▓▓▓▓▓ 4/4 done"),
        "里程碑条:满格 ▓ + 计数 + 终态"
    );
    assert!(lines.contains(&"  —"), "全完成:中性占位,不虚报进行中");
    assert!(plain.contains("车道 / 任务"));
    assert!(plain.contains("✓ T1 fiber join 即回收"));
    assert_eq!(
        lines.last().copied(),
        Some("✓ T4 集成收尾"),
        "车道组纵列铺开"
    );
    for line in &lines {
        assert!(
            display_width(line) <= DEFAULT_PANEL_WIDTH,
            "{DEFAULT_PANEL_WIDTH} 列零溢出: {line:?}"
        );
    }
    assert!(
        render_panel(&dash, DEFAULT_PANEL_WIDTH).contains("\x1b[32m"),
        "done 绿"
    );
}

#[test]
fn active_milestone_drives_header_and_footer() {
    let mut dash = w25_dash();
    dash.milestones[0].done = 2;
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    let lines: Vec<&str> = plain.lines().collect();
    assert_eq!(
        lines[0], "agentdash · W25 并行车道冲刺",
        "页眉携带活跃里程碑"
    );
    assert!(lines.contains(&"  进行中"), "活跃里程碑:进行中");
    assert!(
        plain.contains("  W25 并行车道冲刺 ▓▓▓▓▓░░░░░ 2/4 active"),
        "半程条:▓▓▓▓▓░░░░░ 2/4 active"
    );
}

#[test]
fn narrow_46_columns_zero_overflow_and_truncates_cjk_title() {
    let mut dash = w25_dash();
    dash.milestones[0].title = "超长中文标题用于验证显示宽截断逻辑完整性".into();
    let plain = strip_ansi(&render_panel(&dash, 46));
    for line in plain.lines() {
        assert!(display_width(line) <= 46, "46 列零溢出: {line:?}");
    }
    assert!(
        plain.contains("超长中文标题用于验"),
        "按显示宽截断:9 字前缀保留"
    );
    assert!(!plain.contains("超长中文标题用于验证"), "第 10 字起截去");
    assert!(plain.contains(&"─".repeat(46)), "分隔线钳到 46 列");
}

#[test]
fn rich_states_map_to_visual_marks() {
    let tasks = vec![
        task("T1", "done 活", "A"),
        TaskView {
            id: "T2".into(),
            label: "返修轮".into(),
            state: TaskState::FixRound,
            lane: Some("B".into()),
            note: Some("fix round 2/5".into()),
        },
        TaskView {
            id: "T3".into(),
            label: "复核中".into(),
            state: TaskState::Review,
            lane: Some("B".into()),
            note: None,
        },
        TaskView {
            id: "T4".into(),
            label: "挂起".into(),
            state: TaskState::Blocked,
            lane: None,
            note: None,
        },
    ];
    let dash = dash_with(tasks);
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(plain.contains("✓ T1 done 活"));
    assert!(
        plain.contains("⚑ T2 返修轮 · fix round 2/5"),
        "fix-round 视觉 ⚑,note 透传"
    );
    assert!(plain.contains("▶ T3 复核中"), "review 视觉 ▶");
    assert!(plain.contains("· T4 挂起"), "blocked 视觉同 pending 点");
    assert!(
        plain.contains("✓1 ▶2 ·1 ⚑1 "),
        "统计:▶ 含 review+fix-round、⚑ = fix-round、· 含 blocked"
    );
    assert!(plain.contains("无车道"), "未入车道任务落无车道组");
    assert!(plain.contains("轨迹 · 0 里程碑"));
}

#[test]
fn gates_fill_health_section() {
    let mut gates = HashMap::new();
    gates.insert(
        "test".to_owned(),
        GateState::Passed {
            detail: "3 passed".to_owned(),
        },
    );
    gates.insert("build".to_owned(), GateState::Running);
    gates.insert(
        "lint".to_owned(),
        GateState::Failed {
            detail: String::new(),
        },
    );
    let mut dash = dash_with(vec![task("T1", "实现契约", "A")]);
    dash.gates = gates;
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(plain.contains("✓ test · 3 passed"));
    assert!(plain.contains("▶ build"));
    assert!(
        plain.contains("⚑ lint"),
        "失败 gate 视觉 ⚑,空 detail 不带尾注"
    );
    assert!(
        !plain.contains("无活跃/卡死"),
        "有 gate 时健康区列 gate,不打空态"
    );
}

#[test]
fn warnings_render_as_flagged_lines() {
    let mut dash = dash_with(vec![task("T1", "实现契约", "A")]);
    dash.warnings = vec!["corrupt ledger.json: bad".to_owned()];
    let out = render_panel(&dash, DEFAULT_PANEL_WIDTH);
    assert!(out.contains("\x1b[33m⚠ corrupt ledger.json: bad\x1b[0m"));
}

#[test]
fn panel_width_clamps_to_40_120() {
    let dash = w25_dash();
    let narrow = strip_ansi(&render_panel(&dash, 10));
    assert!(
        narrow.lines().any(|line| line == "═".repeat(40))
            && narrow.lines().any(|line| line == "─".repeat(40)),
        "宽度下钳 40"
    );
    let wide = strip_ansi(&render_panel(&dash, 500));
    assert!(
        wide.lines().any(|line| line == "═".repeat(120)),
        "宽度上钳 120"
    );
    for line in wide.lines() {
        assert!(display_width(line) <= 120, "钳后零溢出: {line:?}");
    }
}

#[test]
fn oneline_is_ansi_free_with_done_count() {
    let dash = w25_dash();
    let out = render_oneline(&dash);
    assert!(!out.contains('\x1b'), "无 ANSI");
    assert!(out.contains("✓4"));
    assert_eq!(
        out, "[dash] agentdash ✓4▶0·0 ⚑0 ·0ag",
        "全完成里程碑非 active:无波次段;agents 恒 0"
    );
}

#[test]
fn oneline_carries_active_milestone_and_counts() {
    let mut dash = w25_dash();
    dash.milestones[0] = MilestoneView {
        wave: Some("W25".into()),
        title: "并行".into(),
        done: 2,
        total: 4,
    };
    for t in &mut dash.tasks {
        if t.id != "T1" && t.id != "T2" {
            t.state = TaskState::Active;
        }
    }
    let out = render_oneline(&dash);
    assert_eq!(
        out, "[dash] agentdash W25 ✓2▶2·0 ⚑0 ·0ag",
        "活跃里程碑带波次号"
    );
}
