//! statusline 单行:无 ANSI、零副作用(W1-006,移植自 claude-dash
//! `dashlib/render_oneline.py`)。

use super::{Visual, active_milestone, project_label, visual};
use crate::model::Dashboard;

/// 无 ANSI 单行:`[dash] <project> <活跃里程碑>✓done▶active·rest ⚑stalled ·<n>ag`。
/// 计数口径:▶ 含 active/review/fix-round(承 Python `active` 含 stalled),
/// `rest` 为 pending+blocked。
#[must_use]
pub fn render_oneline(dash: &Dashboard) -> String {
    let mut done = 0;
    let mut active = 0;
    let mut stalled = 0;
    for task in &dash.tasks {
        match visual(task.state) {
            Visual::Done => done += 1,
            Visual::Active => active += 1,
            Visual::Stalled => {
                active += 1;
                stalled += 1;
            }
            Visual::Pending | Visual::Blocked => {}
        }
    }
    let rest = dash.tasks.len() - done - active;
    // TODO(model):Dashboard 无 activity(agent)字段;agent 计数 W1 恒 0
    let agents = 0;
    let ms = match active_milestone(dash) {
        // I7:带上活跃里程碑;无则整体省略
        Some(milestone) => format!("{} ", ms_id(milestone)),
        None => String::new(),
    };
    format!(
        "[dash] {} {ms}✓{done}▶{active}·{rest} ⚑{stalled} ·{agents}ag",
        project_label(dash)
    )
}

/// 里程碑显示 id(台账 wave;未声明时占位 `-`)。
fn ms_id(milestone: &crate::model::MilestoneView) -> &str {
    milestone.wave.as_deref().unwrap_or("-")
}
