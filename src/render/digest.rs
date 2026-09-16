//! 离场摘要(W10-002):纯文本、零 ANSI 的"你不在时发生了什么"单视图
//! (`render digest [--strict] [PATH]`)。行格式与 panel 同源——统计正文/
//! 门行/agent 行直用 [`super::panel`] 的纯文本正文,计数单源
//! [`super::panel::counts`];oneline 是无 ANSI 先例,本模块同样不触色彩
//! 常量。退出码不在此判定:[`digest_needs_attention`] 供 cmd 层收口。

use std::fmt::Write as _;

use super::panel::{agent_body, counts, gate_body, head_line, stats_body};
use super::{Visual, clock_slice, unattested_done, visual};
use crate::model::Dashboard;

/// 离场摘要:节序固定 = 页眉(项目 · 活跃里程碑)+ 统计一行 → 失败门
/// (✗ 名 · detail)→ 在跑 agent(▶)→ blocked 任务(⊘)→ done 任务
/// (✓,带 `done_at` 者附时刻切片,自报无物证者附 `?`,判定沿
/// [`unattested_done`])→ ⚠ 警告。缺项零残留(不打空标题/空态);无宽度
/// 概念(纯文本不截断、不分块、无分隔线),`--format`/多仓均不入本视图。
#[must_use]
pub fn render_digest(dash: &Dashboard) -> String {
    let mut lines: Vec<String> = Vec::new();

    // 页眉统计:head_line + stats_body 与 panel/精要视图同口径同格式
    lines.push(head_line(dash));
    lines.push(stats_body(
        &counts(&dash.tasks),
        dash.running_agents(),
        &clock_slice(&dash.generated_at),
    ));

    // 失败门:✗ 名 · detail(与精要视图同过滤:仅 failed 态上摘要)
    for gate in &dash.gates {
        if gate.state == "failed" {
            lines.push(gate_body(gate));
        }
    }

    // agent 行:▶ who [host] · task · MM-DDTHH:MM;W11-003 推断配对行随行
    // 带 `▶⇢✓ … (inferred)` 显式标注(正文与 panel 同源)
    for agent in &dash.agents {
        lines.push(agent_body(agent));
    }

    // blocked 任务:⊘ id label(极简行,note/轮次不入摘要)
    for task in &dash.tasks {
        if visual(task.state) == Visual::Blocked {
            lines.push(format!("⊘ {} {}", task.id, task.label));
        }
    }

    // done 任务:✓ id label[ · MM-DDTHH:MM][ ?]——done_at 切片承 agent 行
    // 口径,无 done_at 仍列入(仅标签);物证怀疑沿 unattested_done 语义
    for task in &dash.tasks {
        if visual(task.state) != Visual::Done {
            continue;
        }
        let mut line = format!("✓ {} {}", task.id, task.label);
        if let Some(done_at) = task.done_at.as_deref().filter(|stamp| !stamp.is_empty()) {
            let _ = write!(line, " · {}", clock_slice(done_at));
        }
        if unattested_done(task, dash) {
            line.push_str(" ?");
        }
        lines.push(line);
    }

    // ⚠ 警告:原样透传(警告行自身不得携带 ANSI)
    for warning in &dash.warnings {
        lines.push(format!("⚠ {warning}"));
    }

    lines.join("\n")
}

/// `--strict` 判据(本仓首个内容性退出码的数据面):存在 failed gate
/// **或** blocked 任务 → `true`(调用方退 1,否则 0)。谓词与
/// [`render_digest`] 的节过滤同源;渲染归渲染,退出码在 cmd 层收口。
#[must_use]
pub fn digest_needs_attention(dash: &Dashboard) -> bool {
    dash.gates.iter().any(|gate| gate.state == "failed")
        || dash
            .tasks
            .iter()
            .any(|task| visual(task.state) == Visual::Blocked)
}
