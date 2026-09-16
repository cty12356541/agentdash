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

use contract::TaskState;
use model::{AgentView, Dashboard, GateView, MilestoneView, TaskView};
use render::{DEFAULT_PANEL_WIDTH, display_width, render_oneline, render_panel};
use sources::git::GitFacts;
use sources::remote::RemoteFacts;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

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

fn agent(who: &str, task: Option<&str>, since: &str) -> AgentView {
    agent_host(who, task, since, None)
}

fn agent_host(who: &str, task: Option<&str>, since: &str, host: Option<&str>) -> AgentView {
    AgentView {
        who: who.into(),
        task: task.map(str::to_owned),
        since: since.into(),
        host: host.map(str::to_owned),
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
        agents: Vec::new(), // W2-001:模型新增 agents 字段;面板样例默认无在跑 agent
        gates: Vec::new(),
        barriers: Vec::new(), // W1-007:模型新增 barriers 字段;面板样例不用屏障
        // W3-001 fixture 钉固:项目名走 `GitFacts::root`,不再隐性依赖测试
        // cwd 恰名 agentdash(root 缺省时 project_label 回退 cwd 目录名)
        git: pinned_git(),
        remote: None,          // W2-007:模型新增 remote 字段;面板样例默认无远程探测
        event_span_secs: None, // W3-006:速度线事件活动窗;样例默认无
        // W3-004 D3:项目名上模型,渲染层只读不猜
        project: "agentdash".into(),
        event_tail: Vec::new(), // W4-002:详情事件尾;渲染组样例默认无
        events_present: false,  // W5-001:物证窗;渲染组样例默认无
        last_gate_passed: None,
        generated_at: "2026-09-13T08:30:00Z".into(),
    }
}

fn dash_with(tasks: Vec<TaskView>) -> Dashboard {
    Dashboard {
        tasks,
        milestones: Vec::new(),
        warnings: Vec::new(),
        agents: Vec::new(),
        gates: Vec::new(),
        barriers: Vec::new(),
        // 同 w25_dash:钉固 root 去 cwd 依赖(W3-001)
        git: pinned_git(),
        remote: None,
        event_span_secs: None, // W3-006:速度线事件活动窗;样例默认无
        project: "agentdash".into(),
        event_tail: Vec::new(), // W4-002:详情事件尾;渲染组样例默认无
        events_present: false,  // W5-001:物证窗;渲染组样例默认无
        last_gate_passed: None,
        generated_at: "2026-09-13T08:30:00Z".into(),
    }
}

/// 钉固 git 事实:仅带 `root`(渲染层项目名来源),其余字段全空——手工
/// 构造的 `Dashboard` 不走 git 源,项目名断言因此与测试 cwd 名解耦。
fn pinned_git() -> GitFacts {
    GitFacts {
        root: Some("agentdash".to_owned()),
        ..GitFacts::absent()
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
        lines[1], "✓4 ▶0 ·0 ⚑0 ⊘0 · 0 agents · 09-13T08:30",
        "统计行:计数 + ⊘ 槽(W3-001)+ agents(模型缺字段恒 0)+ 生成时刻"
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
            fix_round: Some((2, 5)),
            since: None,
            done_at: None,
        },
        TaskView {
            id: "T3".into(),
            label: "复核中".into(),
            state: TaskState::Review,
            lane: Some("B".into()),
            note: None,
            fix_round: None,
            since: None,
            done_at: None,
        },
        TaskView {
            id: "T4".into(),
            label: "挂起".into(),
            state: TaskState::Blocked,
            lane: None,
            note: None,
            fix_round: None,
            since: None,
            done_at: None,
        },
    ];
    let dash = dash_with(tasks);
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    let lines: Vec<&str> = plain.lines().collect();
    assert!(
        lines.contains(&"✓ T1 done 活"),
        "无 note/fix_round 任务行零尾缀: {plain}"
    );
    assert!(
        plain.contains("⚑ T2 返修轮 R2/5"),
        "fix-round 视觉 ⚑,R<N>/<M> 尾缀;note 原文已解析不重复(W2-3b)"
    );
    assert!(
        lines.contains(&"▶ T3 复核中"),
        "review 视觉 ▶,无 fix_round 不加尾缀"
    );
    assert!(plain.contains("⊘ T4 挂起"), "blocked 视觉独立符号 ⊘(W2-3b)");
    assert!(
        plain.contains("✓1 ▶2 ·1 ⚑1 "),
        "统计:▶ 含 review+fix-round、⚑ = fix-round、· 含 blocked"
    );
    assert!(plain.contains("无车道"), "未入车道任务落无车道组");
    assert!(plain.contains("轨迹 · 0 里程碑"));
}

#[test]
fn gates_fill_health_section() {
    let mut dash = dash_with(vec![task("T1", "实现契约", "A")]);
    dash.gates = vec![
        GateView {
            name: "build".into(),
            state: "running".into(),
            detail: String::new(),
        },
        GateView {
            name: "lint".into(),
            state: "failed".into(),
            detail: String::new(),
        },
        GateView {
            name: "test".into(),
            state: "passed".into(),
            detail: "3 passed".into(),
        },
    ];
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(plain.contains("▶ build"), "running gate 视觉 ▶");
    assert!(
        plain.contains("✗ lint"),
        "failed gate 视觉 ✗,空 detail 不带尾注"
    );
    assert!(plain.contains("✓ test · 3 passed"));
    assert!(
        !plain.contains("无活跃/卡死"),
        "有 gate 时健康区列 gate,不打空态"
    );
    assert!(
        plain.contains("· 无活跃"),
        "无在跑 agent 时 agents 位打占位"
    );
}

#[test]
fn gate_detail_truncates_with_ellipsis_within_width() {
    let mut dash = dash_with(vec![task("T1", "实现契约", "A")]);
    let long = "x".repeat(200);
    dash.gates = vec![GateView {
        name: "test".into(),
        state: "failed".into(),
        detail: long.clone(),
    }];
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    for line in plain.lines() {
        assert!(
            display_width(line) <= DEFAULT_PANEL_WIDTH,
            "零溢出: {line:?}"
        );
    }
    assert!(plain.contains('✗'));
    assert!(plain.contains('…'), "超长 detail 截断以 … 收尾");
    assert!(!plain.contains(&long), "detail 不整串上板");
}

#[test]
fn agents_block_lists_running_agents_with_task_and_since() {
    let mut dash = w25_dash();
    dash.agents = vec![
        agent("alice", None, "2026-09-13T08:30:00Z"),
        agent("bob", Some("写 panel 区块"), "2026-09-13T09:00:00Z"),
    ];
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(
        plain.contains("▶ alice · 09-13T08:30"),
        "无注记 agent 行 = who + since 切片: {plain}"
    );
    assert!(plain.contains("▶ bob · 写 panel 区块 · 09-13T09:00"));
    assert!(plain.contains("· 2 agents"), "页眉 agents 计数接真值");
    assert!(!plain.contains("无活跃"), "有在跑 agent 不打空态");
    for line in plain.lines() {
        assert!(
            display_width(line) <= DEFAULT_PANEL_WIDTH,
            "零溢出: {line:?}"
        );
    }
}

#[test]
fn agents_without_since_render_who_only() {
    let mut dash = dash_with(vec![task("T1", "实现契约", "A")]);
    dash.agents = vec![agent("carol", None, "")];
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(plain.contains("▶ carol"), "缺 task/since 只列 who: {plain}");
    assert!(!plain.contains("· 无活跃"), "有在跑 agent 不打占位");
}

#[test]
fn warnings_render_as_flagged_lines() {
    let mut dash = dash_with(vec![task("T1", "实现契约", "A")]);
    dash.warnings = vec!["corrupt ledger.json: bad".to_owned()];
    let out = render_panel(&dash, DEFAULT_PANEL_WIDTH);
    assert!(out.contains("\x1b[33m⚠ corrupt ledger.json: bad\x1b[0m"));
}

/// W2-002:每条警告独立一行、`⚠ ` 前缀;超宽警告按显示宽截断以 … 收尾,不顶穿版面。
#[test]
fn warnings_truncate_each_line_within_width() {
    let mut dash = dash_with(vec![task("T1", "实现契约", "A")]);
    let long = format!("ledger: {}", "x".repeat(200));
    dash.warnings = vec![
        long.clone(),
        "ledger: unknown `$schema` `v0`".to_owned(),
        "events.jsonl line 2: invalid JSON".to_owned(),
    ];
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    for line in plain.lines() {
        assert!(
            display_width(line) <= DEFAULT_PANEL_WIDTH,
            "警告行零溢出: {line:?}"
        );
    }
    assert!(
        plain.contains("⚠ ledger: unknown `$schema` `v0`"),
        "短警告整行上板"
    );
    assert!(plain.contains("⚠ ledger: xxx"), "长警告以 ⚠ 前缀起步");
    assert!(plain.contains('…'), "截断行以 … 收尾");
    assert!(!plain.contains(&long), "长警告不整串上板");
}

/// W2-002:契约任务 `since` 距 `generated_at` 超 2h(常量阈值,严格大于)→ 行尾 ⚑;
/// 恰到 2h、不足、无戳、坏戳均不打。
#[test]
fn stale_since_flags_task_row_after_threshold() {
    let tasks = vec![
        {
            let mut stale = task("T1", "停滞任务", "A");
            stale.state = TaskState::Active; // ⚑ 收紧后仅非 done(裁定 2026-09-14)
            stale.since = Some("2026-09-13T06:00:00Z".into()); // 2.5h 前
            stale
        },
        {
            let mut boundary = task("T2", "临界任务", "A");
            boundary.since = Some("2026-09-13T06:30:00Z".into()); // 恰 2h
            boundary
        },
        {
            let mut fresh = task("T3", "新鲜任务", "A");
            fresh.since = Some("2026-09-13T08:00:00Z".into()); // 30m 前
            fresh
        },
        {
            let mut cross = task("T4", "跨日任务", "A");
            cross.state = TaskState::Active;
            cross.since = Some("2026-09-12T20:00:00Z".into()); // 前一日 12.5h
            cross
        },
        {
            let mut broken = task("T5", "坏戳任务", "A");
            broken.since = Some("not-a-time".into()); // 解析不了不虚报
            broken
        },
        task("T6", "无戳任务", "A"), // since None
    ];
    let dash = dash_with(tasks); // generated_at = 2026-09-13T08:30:00Z
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    let lines: Vec<&str> = plain.lines().collect();
    assert!(
        lines.contains(&"▶ T1 停滞任务 ⚑"),
        "超 2h 停滞任务行尾 ⚑: {plain}"
    );
    assert!(
        lines.contains(&"✓ T2 临界任务"),
        "恰 2h 不打 ⚑(严格大于阈值)"
    );
    assert!(lines.contains(&"✓ T3 新鲜任务"));
    assert!(
        lines.contains(&"▶ T4 跨日任务 ⚑"),
        "跨日时刻差按历法日差计算"
    );
    assert!(lines.contains(&"✓ T5 坏戳任务"), "坏 since 不虚报 ⚑");
    assert!(lines.contains(&"✓ T6 无戳任务"));
}

/// W3-004:速度线真实吞吐口径——done 任务数 / 跨度小时数,≥2 里程碑**且**
/// 跨度 > 0 才打(`速度 N.N tasks/h`);轨迹区多里程碑逐行。W3-006 起
/// 事件窗优先,本例 `event_span_secs` 缺省(`None`)正好钉死任务 `since`
/// 跨度的回退半边:fixture 事件流(两条 dispatched,`first_seen` 相距 5h)
/// 回放出任务时间戳,21 done / 5h = 4.2 钉死数值。
#[test]
fn speed_line_is_tasks_per_hour_over_since_span() {
    const EVENTS: &str = concat!(
        r#"{"kind":"agent","event":"dispatched","who":"alice","ts":"2026-09-13T08:00:00Z"}"#,
        "\n",
        r#"{"kind":"agent","event":"dispatched","who":"bob","ts":"2026-09-13T13:00:00Z"}"#,
        "\n",
    );
    let model = events::replay(EVENTS.lines().map(str::to_owned));
    let stamps: Vec<String> = model
        .agents
        .iter()
        .map(|agent| agent.first_seen.clone())
        .collect();
    assert_eq!(stamps.len(), 2, "fixture 事件各携 first_seen(08:00/13:00Z)");

    // 21 个 done 任务,since 在两端戳间取值 → 跨度恰 5h
    let tasks: Vec<TaskView> = (0..21)
        .map(|i| {
            let mut item = task(&format!("T{i:02}"), "冲刺任务", "A");
            item.state = TaskState::Done;
            item.since = Some(stamps[i % stamps.len()].clone());
            item
        })
        .collect();
    let ms = |wave: &str, title: &str, done: usize, total: usize| MilestoneView {
        wave: Some(wave.into()),
        title: title.into(),
        done,
        total,
    };
    let mut dash = dash_with(tasks);
    dash.milestones = vec![ms("W1", "里程碑甲", 10, 10), ms("W2", "里程碑乙", 11, 11)];

    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(
        plain.contains("  速度 4.2 tasks/h"),
        "21 done / 5h = 4.2 tasks/h: {plain}"
    );
    assert!(
        plain.contains("  W1 里程碑甲 ▓▓▓▓▓▓▓▓▓▓ 10/10 done"),
        "多里程碑逐行(第 1 块): {plain}"
    );
    assert!(
        plain.contains("  W2 里程碑乙 ▓▓▓▓▓▓▓▓▓▓ 11/11 done"),
        "多里程碑逐行(第 2 块): {plain}"
    );
    assert!(plain.contains("轨迹 · 2 里程碑"), "完成里程碑计数入区块头");
    for line in plain.lines() {
        assert!(
            display_width(line) <= DEFAULT_PANEL_WIDTH,
            "零溢出: {line:?}"
        );
    }
}

/// W3-004 负断言:速度线生效条件不满足一律不打(不虚报)——单里程碑、
/// 双里程碑但任务 since 全同戳(跨度 0)、双里程碑但无可解析时间戳。
/// 事件窗实参为缺省 `None`,同钉 W3-006 回退半边的负路径。
#[test]
fn speed_line_hidden_unless_two_milestones_and_positive_span() {
    let ms = |wave: &str| MilestoneView {
        wave: Some(wave.into()),
        title: "波".into(),
        done: 1,
        total: 2,
    };
    let stamped = |since: Option<&str>| {
        let mut item = task("T1", "任务", "A");
        item.state = TaskState::Done;
        item.since = since.map(str::to_owned);
        item
    };

    // 单里程碑(即便跨度 > 0 也不打)
    let mut dash = dash_with(vec![
        stamped(Some("2026-09-13T08:00:00Z")),
        stamped(Some("2026-09-13T13:00:00Z")),
    ]);
    dash.milestones = vec![ms("W1")];
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(!plain.contains("速度"), "单里程碑不打速度线: {plain}");

    // 双里程碑但任务 since 全同戳 → 跨度 0
    dash.milestones = vec![ms("W1"), ms("W2")];
    dash.tasks = vec![
        stamped(Some("2026-09-13T13:00:00Z")),
        stamped(Some("2026-09-13T13:00:00Z")),
    ];
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(!plain.contains("速度"), "零跨度不打速度线: {plain}");

    // 双里程碑但任务无时间戳
    dash.tasks = vec![stamped(None), stamped(None)];
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(!plain.contains("速度"), "无时间戳不打速度线: {plain}");
}

/// W3-006:速度行点亮自事件活动窗——任务无 `since`(真实契约形态:恒同
/// 台账 mtime),事件窗 1h、21 done → `  速度 21.0 tasks/h`。W2-002 的
/// 「多里程碑面板含速度线」承诺就此真实可达。
#[test]
fn speed_line_renders_from_event_activity_window() {
    let ms = |wave: &str, done: usize, total: usize| MilestoneView {
        wave: Some(wave.into()),
        title: "波".into(),
        done,
        total,
    };
    let mut dash = dash_with(vec![task("T1", "任务甲", "A"), task("T2", "任务乙", "A")]);
    dash.milestones = vec![ms("W1", 10, 10), ms("W2", 11, 11)];
    dash.event_span_secs = Some(3_600); // 活动窗 1h

    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(
        plain.contains("  速度 21.0 tasks/h"),
        "21 done / 1h 活动窗 = 21.0 tasks/h: {plain}"
    );
    for line in plain.lines() {
        assert!(
            display_width(line) <= DEFAULT_PANEL_WIDTH,
            "零溢出: {line:?}"
        );
    }
}

/// W3-006 负断言(事件窗半边):活动窗缺失(`None`)或退化(全同刻
/// `Some(0)`)且任务无 `since` → 不打;单里程碑即便活动窗在场也不打。
#[test]
fn speed_line_hidden_without_event_window_or_gate() {
    let ms = |wave: &str| MilestoneView {
        wave: Some(wave.into()),
        title: "波".into(),
        done: 1,
        total: 2,
    };
    let unstamped = || task("T1", "任务", "A"); // since 恒 None

    // 双里程碑 + 无活动窗 + 任务无戳
    let mut dash = dash_with(vec![unstamped(), unstamped()]);
    dash.milestones = vec![ms("W1"), ms("W2")];
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(!plain.contains("速度"), "无活动窗无戳不打: {plain}");

    // 双里程碑 + 全同刻窗(Some(0))→ 退化窗不虚报
    dash.event_span_secs = Some(0);
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(!plain.contains("速度"), "全同刻窗不打: {plain}");

    // 单里程碑 + 合格活动窗 → 门槛不过仍不打
    dash.milestones = vec![ms("W1")];
    dash.event_span_secs = Some(3_600);
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(!plain.contains("速度"), "单里程碑不打: {plain}");
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

#[test]
fn oneline_counts_running_agents() {
    let mut dash = w25_dash();
    dash.agents = vec![agent("alice", None, ""), agent("bob", Some("任务乙"), "")];
    assert_eq!(
        render_oneline(&dash),
        "[dash] agentdash ✓4▶0·0 ⚑0 ·2ag",
        "·Nag 接 agents 真值"
    );
}

// ---------- W2-005 车道折叠行(panel 消费折叠视图的伪任务) ----------

/// 折叠伪任务面板契约:单任务空 id 车道组 → `▸ 车道名 (N done)` 单行,
/// 车道头与逐任务行不再出现。
#[test]
fn collapsed_lane_renders_single_marker_line() {
    let tasks = vec![
        TaskView {
            id: String::new(), // 折叠哨兵(tui 折叠视图发出)
            label: "(2 done)".into(),
            state: TaskState::Done,
            lane: Some("alpha".into()),
            note: None,
            fix_round: None,
            since: None,
            done_at: None,
        },
        task("T9", "进行中任务", "beta"),
    ];
    let plain = strip_ansi(&render_panel(&dash_with(tasks), DEFAULT_PANEL_WIDTH));
    assert!(
        plain.contains("▸ alpha (2 done)"),
        "折叠车道单行:▸ 名 (N done): {plain}"
    );
    assert!(
        !plain.lines().any(|line| line == "alpha"),
        "被折叠车道不再有独立车道头行"
    );
    assert!(plain.contains("✓ T9 进行中任务"), "未折叠车道照常逐行");
}

// ---------- W2-007 面板 PR 区块(消费 Dashboard.remote) ----------

#[test]
fn pr_block_renders_remote_facts() {
    let mut dash = dash_with(vec![task("T1", "实现契约", "A")]);
    dash.remote = Some(RemoteFacts {
        pr_number: 12,
        pr_title: "feat: 过滤与折叠".into(),
        checks: vec![
            ("lint".into(), "SUCCESS".into()),
            ("test".into(), "FAILURE".into()),
            ("build".into(), "PENDING".into()),
        ],
    });
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(plain.contains("PR / 远程"), "独立区块标题: {plain}");
    assert!(plain.contains("#12 feat: 过滤与折叠"), "PR 号 + 标题");
    assert!(plain.contains("✓ lint SUCCESS"), "SUCCESS → ✓");
    assert!(plain.contains("✗ test FAILURE"), "FAILURE → ✗");
    assert!(plain.contains("▶ build PENDING"), "PENDING → ▶");
    for line in plain.lines() {
        assert!(
            display_width(line) <= DEFAULT_PANEL_WIDTH,
            "PR 区块零溢出: {line:?}"
        );
    }
}

#[test]
fn pr_block_absent_without_remote_and_placeholder_without_pr() {
    let dash = dash_with(vec![task("T1", "实现契约", "A")]);
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(
        !plain.contains("PR / 远程"),
        "remote 缺省(探测降级 None)不打区块: {plain}"
    );

    let mut empty_facts = dash_with(vec![task("T1", "实现契约", "A")]);
    empty_facts.remote = Some(RemoteFacts::default());
    let plain = strip_ansi(&render_panel(&empty_facts, DEFAULT_PANEL_WIDTH));
    assert!(
        plain.contains("PR / 远程") && plain.contains("· 无关联 PR"),
        "探测成功但无 PR:占位不白板"
    );
}

// ---------- W2-3b 验证发现修复批(F1 缺失源警告 / F3 项目名 / F6 blocked+note) ----------
//
// 本组走真实 merge 链路(挂载的 model + sources 合并),需要受控 fixture 仓:
// 目录名即项目名断言的期望值,helper 与 tests/merge.rs 同约定(各测试二进制
// 独立编译,无命名冲突)。

fn run_git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .expect("git should be on PATH for tests");
    assert!(
        status.success(),
        "git {args:?} failed in {}",
        repo.display()
    );
}

fn next_dir(name: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "agentdash-w2-3b-{name}-{}-{serial}",
        std::process::id()
    ))
}

fn cleanup(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

/// 受控 fixture 仓:git init + 本地身份 + main 分支 + 1 次提交(目录名可断言)。
fn fixture_repo(name: &str) -> PathBuf {
    let repo = next_dir(name);
    fs::create_dir_all(&repo).expect("create fixture dir");
    run_git(&repo, &["init"]);
    run_git(&repo, &["config", "user.name", "agentdash-test"]);
    run_git(
        &repo,
        &["config", "user.email", "agentdash-test@example.com"],
    );
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    run_git(&repo, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    fs::write(repo.join("a.txt"), "one\n").expect("write a.txt");
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "one"]);
    repo
}

/// W2-3b F1(AD-ERR-001):契约**缺失**(有 events 无 ledger.json)→
/// 面板出 `⚠ missing ledger.json: …` 警告行,措辞对齐损坏路径风格。
#[test]
fn missing_ledger_renders_warning_line_in_panel() {
    let repo = fixture_repo("missing-ledger");
    let dir = repo.join(".agentdash");
    fs::create_dir_all(&dir).expect("create .agentdash");
    fs::write(
        dir.join("events.jsonl"),
        r#"{"kind":"gate","gate":"review","state":"passed","detail":"ok"}"#,
    )
    .expect("write events.jsonl");

    let dash = model::merge(&repo);
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));

    assert!(
        plain.contains("⚠ missing ledger.json"),
        "面板必须携带契约缺失警告行: {plain}"
    );
    assert!(
        !plain.contains("no data sources"),
        "其余源在场不打全无引导: {plain}"
    );
    cleanup(&repo);
}

/// W2-3b F3:项目名 = git 仓根目录名(去硬编码);panel 页眉与 oneline 同源。
#[test]
fn project_label_derives_git_repo_root_name() {
    let repo = fixture_repo("proj-repo");
    let expected = repo
        .file_name()
        .expect("fixture dir name")
        .to_string_lossy()
        .into_owned();

    let dash = model::merge(&repo);
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    let lines: Vec<&str> = plain.lines().collect();

    assert_eq!(lines[0], expected, "panel 页眉项目名 = 仓根目录名: {plain}");
    assert_eq!(
        render_oneline(&dash),
        format!("[dash] {expected} ✓0▶0·1 ⚑0 ·0ag"),
        "oneline 项目名 = 仓根目录名"
    );
    cleanup(&repo);
}

/// W2-3b F6①:blocked 任务行用独立符号 ⊘,与 pending 的 · 可区分。
#[test]
fn blocked_renders_with_distinct_glyph() {
    let mut blocked = task("B1", "阻塞任务", "A");
    blocked.state = TaskState::Blocked;
    let mut pending = task("P1", "待办任务", "A");
    pending.state = TaskState::Pending;
    let tasks = vec![blocked, pending];
    let plain = strip_ansi(&render_panel(&dash_with(tasks), DEFAULT_PANEL_WIDTH));
    let lines: Vec<&str> = plain.lines().collect();

    assert!(
        lines.contains(&"⊘ B1 阻塞任务"),
        "blocked 行携带独立符号 ⊘: {plain}"
    );
    assert!(lines.contains(&"· P1 待办任务"), "pending 行仍为 ·");
    assert!(
        !plain.contains("· B1"),
        "blocked 不得再与 pending 同点: {plain}"
    );
}

/// W2-3b F6②:note 解析出 `fix_round` 后行内只出 R<N>/<M> 尾缀,
/// 不再重复渲染 note 原文。
#[test]
fn fix_round_note_not_duplicated_on_task_row() {
    let tasks = vec![TaskView {
        id: "T1".into(),
        label: "返修轮".into(),
        state: TaskState::FixRound,
        lane: Some("A".into()),
        note: Some("fix round 2/5".into()),
        fix_round: Some((2, 5)),
        since: None,
        done_at: None,
    }];
    let plain = strip_ansi(&render_panel(&dash_with(tasks), DEFAULT_PANEL_WIDTH));

    assert!(plain.contains("R2/5"), "R 尾缀照常渲染: {plain}");
    assert!(
        !plain.contains("fix round 2/5"),
        "已解析出尾缀,note 原文不得重复上板: {plain}"
    );
}

// ---------- W3-001 终审微修批(图例 ⊘ 槽 / fix_round 残余 note / F1 负路径) ----------

/// 图例 ⊘ 槽(W3-001):统计行在 ⚑ 后新增 `⊘{blocked}` 槽,blocked 计数
/// 独立可读;`·{resting}` 口径不变(仍为 pending + blocked 双计)。
#[test]
fn legend_gains_blocked_slot_and_resting_unchanged() {
    let mut blocked = task("B1", "阻塞任务", "A");
    blocked.state = TaskState::Blocked;
    let mut pending = task("P1", "待办任务", "A");
    pending.state = TaskState::Pending;
    let dash = dash_with(vec![blocked, pending]);
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    let lines: Vec<&str> = plain.lines().collect();
    assert_eq!(
        lines[1], "✓0 ▶0 ·2 ⚑0 ⊘1 · 0 agents · 09-13T08:30",
        "统计行:⊘ 只计 blocked,· 仍计 pending+blocked(口径不变): {plain}"
    );
}

/// `fix_round` 仅抑匹配前缀(W3-001):note 为 `fix round N/M <残余>` 时,
/// 行内出 `R<N>/<M> <残余>`——匹配前缀折叠为轮次尾缀,残余 note 照常上板。
#[test]
fn fix_round_suppresses_only_matched_prefix_and_keeps_residual() {
    let tasks = vec![TaskView {
        id: "T1".into(),
        label: "返修轮".into(),
        state: TaskState::FixRound,
        lane: Some("A".into()),
        note: Some("fix round 2/5 auth bug".into()),
        fix_round: Some((2, 5)),
        since: None,
        done_at: None,
    }];
    let plain = strip_ansi(&render_panel(&dash_with(tasks), DEFAULT_PANEL_WIDTH));

    assert!(
        plain.contains("⚑ T1 返修轮 R2/5 auth bug"),
        "匹配前缀折叠为 R 尾缀,残余 note 保留上板: {plain}"
    );
    assert!(
        !plain.contains("fix round"),
        "匹配前缀本身不得重复上板: {plain}"
    );
}

/// F1 负路径(W3-001):有台账、无 events → 不出 `missing ledger.json`
/// 警告(缺失警告只针对台账缺失;events 缺失不告警),面板照常渲染台账任务。
#[test]
fn ledger_without_events_renders_no_missing_warning() {
    const LEDGER: &str = r#"{
      "$schema": "agentdash.tasklog.v1",
      "title": "ledger only",
      "tasks": {"1": {"label": "唯一任务", "state": "active"}}
    }"#;
    let repo = fixture_repo("ledger-no-events");
    let dir = repo.join(".agentdash");
    fs::create_dir_all(&dir).expect("create .agentdash");
    fs::write(dir.join("ledger.json"), LEDGER).expect("write ledger.json");

    let dash = model::merge(&repo);
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));

    assert!(
        !plain.contains("missing ledger.json"),
        "台账在场不出缺失警告(events 缺失不告警): {plain}"
    );
    assert!(plain.contains("▶ 1 唯一任务"), "台账任务照常上板: {plain}");
    cleanup(&repo);
}

// ---------- W3-002 面板屏障行(after → unlocks 紧凑一行) ----------

/// W3-002:台账屏障上面板——车道区之后按声明序每屏障一行
/// `  ⇕ <after 逗号表> → <unlocks 逗号表>`;after/unlocks 任务 id 逐字
/// 出现在对应行,语义与 graph 的 after→unlocks 边同向。
#[test]
fn barrier_lines_render_after_lane_section() {
    const LEDGER: &str = r#"{
      "$schema": "agentdash.tasklog.v1",
      "wave": "W30",
      "title": "屏障样例",
      "tasks": {
        "02": {"label": "先手任务甲", "state": "done"},
        "04": {"label": "先手任务乙", "state": "done"},
        "05": {"label": "后手任务", "state": "pending"},
        "06": {"label": "再后手任务", "state": "pending"}
      },
      "barriers": [
        {"id": "B1", "after": ["02", "04"], "unlocks": ["05"]},
        {"id": "B2", "after": ["05"], "unlocks": ["06"]}
      ]
    }"#;
    let repo = fixture_repo("panel-barriers");
    let dir = repo.join(".agentdash");
    fs::create_dir_all(&dir).expect("create .agentdash");
    fs::write(dir.join("ledger.json"), LEDGER).expect("write ledger.json");

    let dash = model::merge(&repo);
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    let lines: Vec<&str> = plain.lines().collect();

    let b1 = lines
        .iter()
        .position(|line| *line == "  ⇕ 02,04 → 05")
        .unwrap_or_else(|| panic!("屏障 B1 行(after 02,04 → unlocks 05)应上板: {plain}"));
    let b2 = lines
        .iter()
        .position(|line| *line == "  ⇕ 05 → 06")
        .unwrap_or_else(|| panic!("屏障 B2 行(after 05 → unlocks 06)应上板: {plain}"));
    let lane_header = lines
        .iter()
        .position(|line| *line == "车道 / 任务")
        .expect("车道区头在场");
    let last_task = lines
        .iter()
        .rposition(|line| *line == "· 06 再后手任务")
        .expect("台账任务行在场");
    assert!(
        b1 > lane_header && b1 > last_task && b2 > last_task,
        "屏障行在车道/任务区之后: B1={b1} B2={b2} 车道头={lane_header} 末任务={last_task}"
    );
    assert!(b1 < b2, "屏障按台账声明序排列: {plain}");
    for line in plain.lines() {
        assert!(
            display_width(line) <= DEFAULT_PANEL_WIDTH,
            "屏障行零溢出: {line:?}"
        );
    }
    cleanup(&repo);
}

/// W3-002 负断言:无屏障的面板零残留——不打 ⇕ 行、不打屏障区块标题,
/// 也不虚占版面。
#[test]
fn no_barrier_residue_without_barriers() {
    let dash = dash_with(vec![task("T1", "实现契约", "A")]); // barriers 恒空
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(!plain.contains('⇕'), "无屏障不得出 ⇕ 行: {plain}");
    assert!(
        !plain.lines().any(|line| line.contains(" → ")),
        "无屏障不得出箭头行: {plain}"
    );
}

// ---------- W3-004 project 上模型(D3)+ 页眉/图题 elide + 黄金不变量 ----------

/// W3-004 D3:页眉项目名读 [`Dashboard::project`](模型字段),渲染层
/// cwd 回退退役——模型给什么页眉打什么,不再自猜。
#[test]
fn header_project_reads_model_field() {
    let mut dash = dash_with(vec![task("T1", "实现契约", "A")]);
    dash.project = "myrepo".into();
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    let lines: Vec<&str> = plain.lines().collect();
    assert_eq!(lines[0], "myrepo", "页眉首行 = 模型 project 字段: {plain}");
    assert_eq!(
        render_oneline(&dash),
        "[dash] myrepo ✓1▶0·0 ⚑0 ·0ag",
        "oneline 项目名同源"
    );
}

/// W3-004 D3:超长仓名按现有截断策略钳制——panel 页眉与 graph 标题都以
/// `…` 收尾且不破宽度(oneline 为 statusline,无宽度概念,不钳)。
#[test]
fn oversized_project_elides_in_header_and_graph_title() {
    let long = format!("超宽仓{}", "x".repeat(200));
    let mut dash = dash_with(vec![task("T1", "实现契约", "A")]);
    dash.project = long.clone();

    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    for line in plain.lines() {
        assert!(
            display_width(line) <= DEFAULT_PANEL_WIDTH,
            "panel 零溢出: {line:?}"
        );
    }
    assert!(plain.contains('…'), "超宽项目名以 … 收尾: {plain}");
    assert!(!plain.contains(&long), "超宽项目名不整串上板");

    let graph = strip_ansi(&render::graph::render_graph(&dash, 72));
    for (idx, line) in graph.lines().enumerate() {
        assert!(
            display_width(line) <= 72,
            "graph 标题零溢出: {idx} {line:?}"
        );
    }
    let title = graph.lines().next().unwrap_or_default();
    assert!(title.contains('…'), "graph 标题超宽以 … 收尾: {title}");
    // 整行 elide(现有截断策略):项目名独宽超限时尾缀随截断丢失;
    // 正常宽度下 `project · DAG` 完整形态由 v1 黄金测试钉死
}

/// W3-004 黄金不变量:v1 台账(无 `milestones`)的 panel / oneline / graph
/// 输出与扩展前**逐字节一致**(黄金串由扩展前二进制捕获;panel 仅把行尾
/// 运行时刻钟归一为 `MM-DDTHH:MM`,它本就随刷新时刻漂移,不属行为面)。
#[test]
fn v1_ledger_without_milestones_renders_byte_identical() {
    const GOLDEN_LEDGER: &str = r#"{
  "$schema": "agentdash.tasklog.v1",
  "wave": "W9",
  "title": "v1 黄金对照样例",
  "profile": "sdd",
  "lanes": [
    {"name": "甲-实现", "tasks": ["01", "02"]},
    {"name": "乙-验证", "tasks": ["03"]}
  ],
  "tasks": {
    "01": {"label": "契约解析", "state": "done"},
    "02": {"label": "模型合并", "state": "active", "note": "fix round 2/5 边界样例"},
    "03": {"label": "渲染对照", "state": "pending"}
  },
  "barriers": [
    {"id": "B1", "after": ["01"], "unlocks": ["03"]}
  ]
}"#;

    // 固定名 fixture 仓(目录名 = project 名,进黄金串);先清残再建
    let repo = std::env::temp_dir().join("ad-golden-v1");
    let _ = fs::remove_dir_all(&repo);
    fs::create_dir_all(repo.join(".agentdash")).expect("create golden fixture");
    run_git(&repo, &["init"]);
    run_git(&repo, &["config", "user.name", "agentdash-test"]);
    run_git(
        &repo,
        &["config", "user.email", "agentdash-test@example.com"],
    );
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    run_git(&repo, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    fs::write(repo.join("seed.txt"), "seed\n").expect("write seed");
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "one"]);
    fs::write(repo.join(".agentdash").join("ledger.json"), GOLDEN_LEDGER)
        .expect("write golden ledger");

    let dash = model::merge(&repo);
    let sep = |ch: char, width: usize| std::iter::repeat_n(ch, width).collect::<String>();
    let sep64 = sep('═', 64);
    let dash64 = sep('─', 64);
    let sep72 = sep('─', 72);

    // panel:黄金逐行(行 1 时钟归一);逐字节对比,零 diff
    let panel = strip_ansi(&render_panel(&dash, 64));
    let mut lines: Vec<String> = panel.lines().map(str::to_owned).collect();
    assert!(lines.len() >= 3, "panel 至少三行: {panel}");
    let clock_row = &mut lines[1];
    let cut = clock_row
        .char_indices()
        .rev()
        .nth(10)
        .map_or(0, |(idx, _)| idx);
    clock_row.truncate(cut);
    clock_row.push_str("MM-DDTHH:MM");
    let expected_panel = format!(
        "ad-golden-v1 · W9 v1 黄金对照样例\n\
         ✓1 ▶1 ·1 ⚑0 ⊘0 · 0 agents · MM-DDTHH:MM\n\
         {sep64}\n\
         在跑 / 健康\n\
         ✓ 无活跃/卡死\n\
         {dash64}\n\
         轨迹 · 0 里程碑\n\
         \x20 W9 v1 黄金对照样例 ▓▓▓░░░░░░░ 1/3 active\n\
         \x20 进行中\n\
         {dash64}\n\
         车道 / 任务\n\
         甲-实现\n\
         ✓ 01 契约解析\n\
         ▶ 02 模型合并 R2/5 边界样例\n\
         乙-验证\n\
         · 03 渲染对照\n\
         \x20 ⇕ 01 → 03"
    );
    assert_eq!(
        lines.join("\n"),
        expected_panel,
        "v1 台账 panel 必须与扩展前逐字节一致(仅时钟归一)"
    );

    // oneline:无时钟,整行黄金
    assert_eq!(
        render_oneline(&dash),
        "[dash] ad-golden-v1 W9 ✓1▶1·1 ⚑0 ·0ag",
        "v1 台账 oneline 逐字节一致"
    );

    // graph:无时钟,整图黄金
    let graph = strip_ansi(&render::graph::render_graph(&dash, 72));
    let expected_graph = format!(
        "ad-golden-v1 · DAG\n\
         {sep72}\n\
         ┌───────────────┐\n\
         │ ✓ 01 契约解析 │\n\
         └───────┬───────┘\n\
         \x20       │\n\
         \x20       └──────────────────│\n\
         ┌───────▼───────┐  ┌───────▼───────┐\n\
         │ ▶ 02 模型合并 │  │ · 03 渲染对照 │\n\
         └───────────────┘  └───────────────┘\n\
         {sep72}"
    );
    assert_eq!(
        graph, expected_graph,
        "v1 台账 graph 必须与扩展前逐字节一致"
    );
    let _ = fs::remove_dir_all(&repo);
}

// ------------------------------------------------------------ 物证 ? 标记(W5-001)

#[test]
fn unattested_done_marks_question_only_in_witness_window() {
    let done = |done_at: Option<&str>| TaskView {
        id: "T1".into(),
        label: "x".into(),
        state: TaskState::Done,
        lane: None,
        note: None,
        fix_round: None,
        since: None,
        done_at: done_at.map(str::to_owned),
    };
    // 自报晚于通过门 → ?
    let mut dash = dash_with(vec![done(Some("2026-09-13T12:00:00Z"))]);
    dash.events_present = true;
    dash.last_gate_passed = Some("2026-09-13T09:00:00Z".into());
    let out = strip_ansi(&render::render_panel(&dash, 64));
    assert!(out.contains("✓ T1 x ?"), "自报无物证应打 ?: {out}");

    // 自报早于通过门 → 不打
    dash.tasks[0].done_at = Some("2026-09-13T08:00:00Z".into());
    let out = strip_ansi(&render::render_panel(&dash, 64));
    assert!(!out.contains("T1 x ?"), "有物证不打 ?: {out}");

    // 无 done_at(人工维护,不可断言)→ 不打
    dash.tasks[0].done_at = None;
    let out = strip_ansi(&render::render_panel(&dash, 64));
    assert!(!out.contains("T1 x ?"), "无自报戳不打 ?: {out}");

    // 无事件窗(纯契约)→ 不打(无证可查不作怀疑)
    dash.tasks[0].done_at = Some("2026-09-13T12:00:00Z".into());
    dash.events_present = false;
    let out = strip_ansi(&render::render_panel(&dash, 64));
    assert!(!out.contains("T1 x ?"), "无事件窗不打 ?: {out}");
}

#[test]
fn agent_line_shows_host_tag() {
    // W7-001:在跑行显示宿主归属 [host]
    let mut dash = dash_with(vec![]);
    dash.agents = vec![agent_host(
        "explore",
        Some("摸底"),
        "2026-09-13T08:30:00Z",
        Some("codex"),
    )];
    let out = strip_ansi(&render::render_panel(&dash, 64));
    assert!(
        out.contains("▶ explore [codex] · 摸底"),
        "在跑行应含宿主标: {out}"
    );
}

// ---------- W10-001 多仓聚合:单仓字节钉 + 精要视图(render_brief) ----------

/// W10-001 字节钉(黄金不变量):单仓全面板输出**含 ANSI 逐字节**等于改造前
/// 捕获的黄金串(捕获自基线 e985029 现行代码 + 同一 w25 fixture,见任务报告)
/// ——任何触及 panel 内部或单仓渲染路由的改动在此炸出。
#[test]
fn single_path_panel_byte_pin_w10() {
    const PIN: &str = "agentdash\n\
\x1b[32m✓4\x1b[0m \x1b[34m▶0\x1b[0m \x1b[90m·0\x1b[0m \x1b[33m⚑0\x1b[0m \x1b[90m⊘0\x1b[0m · 0 agents · 09-13T08:30\n\
════════════════════════════════════════════════════════════════\n\
\x1b[1m在跑 / 健康\x1b[0m\n\
\x1b[32m✓ 无活跃/卡死\x1b[0m\n\
────────────────────────────────────────────────────────────────\n\
\x1b[1m轨迹 · 1 里程碑\x1b[0m\n\
\x20\x20\x1b[32mW25 并行车道冲刺 ▓▓▓▓▓▓▓▓▓▓ 4/4 done\x1b[0m\n\
\x20\x20—\n\
────────────────────────────────────────────────────────────────\n\
\x1b[1m车道 / 任务\x1b[0m\n\
\x1b[1mA\x1b[0m\n\
\x1b[32m✓ T1 fiber join 即回收\x1b[0m\n\
\x1b[1mB\x1b[0m\n\
\x1b[32m✓ T2 scope by_id 索引\x1b[0m\n\
\x1b[1mC\x1b[0m\n\
\x1b[32m✓ T3 orphan 上界\x1b[0m\n\
\x1b[1mD\x1b[0m\n\
\x1b[32m✓ T4 集成收尾\x1b[0m";
    assert_eq!(
        render_panel(&w25_dash(), DEFAULT_PANEL_WIDTH),
        PIN,
        "单仓全面板必须与 W10 改造前逐字节一致(含 ANSI)"
    );
}

/// W10-001 精要视图四要素:统计行(与全面板同口径)+ 在跑 agent + 失败门 +
/// blocked 任务 + ⚠ 警告齐上板;passed 门、非 blocked 任务、轨迹/车道区块、
/// 分隔线全部零残留(缺项零残留,不打空标题)。
#[test]
fn brief_view_renders_four_elements_with_zero_residue() {
    let mut dash = w25_dash();
    dash.milestones[0].done = 2; // W25 转 active → 页眉携带
    dash.tasks[1].state = TaskState::Active; // T2 在跑(非 blocked,不上精要)
    dash.tasks[2].state = TaskState::Blocked; // T3 blocked(上精要)
    dash.agents = vec![agent("alice", Some("冲 W10"), "2026-09-13T08:30:00Z")];
    dash.gates = vec![
        GateView {
            name: "build".into(),
            state: "passed".into(),
            detail: String::new(),
        },
        GateView {
            name: "test".into(),
            state: "failed".into(),
            detail: "2 failed".into(),
        },
    ];
    dash.warnings = vec!["ledger: 未知字段".to_owned()];

    let plain = strip_ansi(&render::render_brief(&dash, DEFAULT_PANEL_WIDTH));
    let lines: Vec<&str> = plain.lines().collect();
    assert_eq!(
        lines,
        vec![
            "agentdash · W25 并行车道冲刺",
            "✓2 ▶1 ·1 ⚑0 ⊘1 · 1 agents · 09-13T08:30",
            "▶ alice · 冲 W10 · 09-13T08:30",
            "✗ test · 2 failed",
            "⊘ T3 orphan 上界",
            "⚠ ledger: 未知字段",
        ],
        "精要视图六行:页眉/统计/在跑/失败门/blocked/⚠,零残留: {plain}"
    );
    for line in plain.lines() {
        assert!(
            display_width(line) <= DEFAULT_PANEL_WIDTH,
            "精要块零溢出: {line:?}"
        );
    }
    assert!(!plain.contains('═') && !plain.contains('─'), "无区块分隔线");
    assert!(
        !plain.contains("车道 / 任务") && !plain.contains("轨迹"),
        "无全面板区块头"
    );
}

/// W10-001 空仓零残留:无 agent / 门 / blocked / 警告的仓,精要视图只出
/// 页眉 + 统计两行,不打任何空态占位。
#[test]
fn brief_view_empty_repo_renders_two_lines_only() {
    let dash = w25_dash(); // 全 done、无活跃里程碑、无 agent/门/警告
    let plain = strip_ansi(&render::render_brief(&dash, DEFAULT_PANEL_WIDTH));
    let lines: Vec<&str> = plain.lines().collect();
    assert_eq!(
        lines,
        vec!["agentdash", "✓4 ▶0 ·0 ⚑0 ⊘0 · 0 agents · 09-13T08:30",],
        "空仓仅页眉+统计两行: {plain}"
    );
}
