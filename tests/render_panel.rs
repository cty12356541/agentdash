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
        // W3-001 fixture 钉固:项目名走 `GitFacts::root`,不再隐性依赖测试
        // cwd 恰名 agentdash(root 缺省时 project_label 回退 cwd 目录名)
        git: pinned_git(),
        remote: None, // W2-007:模型新增 remote 字段;面板样例默认无远程探测
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
