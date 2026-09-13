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

fn agent(who: &str, task: Option<&str>, since: &str) -> AgentView {
    AgentView {
        who: who.into(),
        task: task.map(str::to_owned),
        since: since.into(),
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
        git: GitFacts::absent(),
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
            fix_round: Some((2, 5)),
            since: None,
        },
        TaskView {
            id: "T3".into(),
            label: "复核中".into(),
            state: TaskState::Review,
            lane: Some("B".into()),
            note: None,
            fix_round: None,
            since: None,
        },
        TaskView {
            id: "T4".into(),
            label: "挂起".into(),
            state: TaskState::Blocked,
            lane: None,
            note: None,
            fix_round: None,
            since: None,
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
        plain.contains("⚑ T2 返修轮 · fix round 2/5 R2/5"),
        "fix-round 视觉 ⚑,note 透传 + R<N>/<M> 尾缀"
    );
    assert!(
        lines.contains(&"▶ T3 复核中"),
        "review 视觉 ▶,无 fix_round 不加尾缀"
    );
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

/// W2-002:速度线以里程碑计数近似吞吐(最近 3 波 done 均值,银行家舍入);
/// 单里程碑信息不足不打,≥2 里程碑才出现。
#[test]
fn speed_line_needs_two_milestones_and_averages_last_three() {
    // 单里程碑:无速度线
    let dash = w25_dash();
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(!plain.contains("速度"), "单里程碑不打速度线: {plain}");

    let wave = |wave: &str, done: usize| MilestoneView {
        wave: Some(wave.into()),
        title: "波".into(),
        done,
        total: 9,
    };

    // 双里程碑:均值 (4+1)/2 = 2.5 → 五成双 → 2
    let mut dash = w25_dash();
    dash.milestones.push(wave("W26", 1));
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(
        plain.contains("速度 2 任务/波次"),
        "双里程碑显示最近波次均值 (4+1)/2→2: {plain}"
    );

    // 三里程碑:窗口=全部 → (4+4+1)/3 = 3
    dash.milestones.insert(0, wave("W24", 4));
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(
        plain.contains("速度 3 任务/波次"),
        "三里程碑均值 (4+4+1)/3=3: {plain}"
    );

    // 四里程碑:窗口只取最近 3 波 (4+4+1)/3=3;全平均 (9+4+4+1)/4=4.5 会得 4
    dash.milestones.insert(0, wave("W23", 9));
    let plain = strip_ansi(&render_panel(&dash, DEFAULT_PANEL_WIDTH));
    assert!(
        plain.contains("速度 3 任务/波次"),
        "窗口只取最近 3 波,陈旧波次不入均: {plain}"
    );
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
