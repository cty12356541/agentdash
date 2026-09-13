//! 渲染层(W1-006):把 [`Dashboard`](crate::model::Dashboard) 折叠为三个终端视图。
//!
//! - [`oneline::render_oneline`]:无 ANSI 单行(statusline);
//! - [`panel::render_panel`]:面板——页眉统计 → 健康 → 轨迹 → 车道;
//! - [`graph::render_graph_with`]:框化节点 DAG(┌─┐ 框、┬ 出线桩、▼ 入线箭头)。
//!
//! 算法移植自 claude-dash `dashlib/render_oneline.py` / `render_panel.py` /
//! `render_graph.py`,以其实测黄金断言语义为准;输入面收敛为
//! `model::Dashboard`。模型暂缺的展示字段(activity / velocity / barriers)
//! 以 0 / 空呈现并留 TODO,不扩 model(扩字段属后续车道)。

mod graph;
mod oneline;
mod panel;

// W1 过渡:重导出面由接线车道(main 命令)消费,在那之前 bin 目标视为未用;
// 接线落地后删除本 allow。
#[allow(unused_imports)]
pub use graph::{
    BarrierEdges, Cell, DEFAULT_GRAPH_WIDTH, hit_test, layout_layers, render_graph,
    render_graph_with,
};
#[allow(unused_imports)]
pub use oneline::render_oneline;
#[allow(unused_imports)]
pub use panel::{DEFAULT_PANEL_WIDTH, render_panel};

use crate::contract::TaskState;
use crate::model::{Dashboard, MilestoneView};

// ANSI 调色(承 Python `C` 表;oneline 视图不用)。
pub(crate) const C_DONE: &str = "\x1b[32m";
pub(crate) const C_ACTIVE: &str = "\x1b[34m";
pub(crate) const C_PENDING: &str = "\x1b[90m";
pub(crate) const C_STALLED: &str = "\x1b[33m";
pub(crate) const C_WARN: &str = "\x1b[33m";
pub(crate) const C_BOLD: &str = "\x1b[1m";
pub(crate) const C_END: &str = "\x1b[0m";

/// 宽度钳(Brief:40..120);panel/graph 的宽度参数一律先经此钳位。
pub(crate) const fn clamp_width(width: usize) -> usize {
    if width < 40 {
        40
    } else if width > 120 {
        120
    } else {
        width
    }
}

/// 项目名占位(TODO(model):`Dashboard` 无 project 字段;W1 以 crate 名代之,
/// 后续扩字段后改取仓库名)。
pub(crate) fn project_label(_dash: &Dashboard) -> &'static str {
    "agentdash"
}

/// 任务五态视觉映射(承 claude-dash 五态):done ✓ 绿 / active ▶ 蓝 /
/// stalled ⚑ 黄 / pending·blocked · 灰。agentdash 富态归并:
/// `Review` 归 active(复核进行中),`FixRound` 归 stalled(返修待办)。
pub(crate) fn visual(state: TaskState) -> Visual {
    match state {
        TaskState::Done => Visual::Done,
        TaskState::Active | TaskState::Review => Visual::Active,
        TaskState::FixRound => Visual::Stalled,
        TaskState::Pending => Visual::Pending,
        TaskState::Blocked => Visual::Blocked,
    }
}

/// 视觉态:标记符与 ANSI 色的一对一组合。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Visual {
    /// 完成。
    Done,
    /// 进行中。
    Active,
    /// 卡死/返修待办。
    Stalled,
    /// 未开始。
    Pending,
    /// 阻塞。
    Blocked,
}

impl Visual {
    /// 单字符标记(`✓▶·⚑` 经 Python `unicodedata` 核对均为窄字符,占 1 列)。
    pub(crate) fn mark(self) -> &'static str {
        match self {
            Self::Done => "✓",
            Self::Active => "▶",
            Self::Stalled => "⚑",
            Self::Pending | Self::Blocked => "·",
        }
    }

    /// ANSI 前景色。
    pub(crate) fn color(self) -> &'static str {
        match self {
            Self::Done => C_DONE,
            Self::Active => C_ACTIVE,
            Self::Stalled => C_STALLED,
            Self::Pending | Self::Blocked => C_PENDING,
        }
    }
}

/// 里程碑三态(承 Python done|active|planned):全部任务终态 done 即完成;
/// 有完成量进行中;否则 planned。
pub(crate) fn milestone_state(milestone: &MilestoneView) -> &'static str {
    if milestone.is_complete() {
        "done"
    } else if milestone.done > 0 {
        "active"
    } else {
        "planned"
    }
}

/// 首个 active 里程碑(页眉与"进行中"判定)。
pub(crate) fn active_milestone(dash: &Dashboard) -> Option<&MilestoneView> {
    dash.milestones
        .iter()
        .find(|milestone| milestone_state(milestone) == "active")
}

/// 银行家舍入(承 Python `round` 的五成双语义):`numer / denom` 四舍六入。
pub(crate) fn round_half_even(numer: usize, denom: usize) -> usize {
    let (quotient, remainder) = (numer / denom, numer % denom);
    match 2 * remainder {
        double if double > denom => quotient + 1,
        double if double == denom => quotient + (quotient & 1),
        _ => quotient,
    }
}

/// 显示宽:东亚宽字符(EAW W/F)占 2 列——框宽/列布局必须按显示列算,
/// 码点数会把中文标签的框算窄、文字顶穿右边框。
///
/// EAW 子集表覆盖 W1 所需区段(CJK 与全角形);框线/标记符(─┬▼✓▶·⚑ 等)
/// 属 EAW A/N 窄字符,已经 Python `unicodedata` 逐一核对为 1 列。
#[must_use]
pub fn display_width(text: &str) -> usize {
    text.chars().map(|ch| if is_wide(ch) { 2 } else { 1 }).sum()
}

/// EAW W/F 判定(子集;全表待引入 `unicode-width` 依赖后替换——TODO(deps))。
fn is_wide(ch: char) -> bool {
    matches!(
        u32::from(ch),
        0x1100..=0x115F // Hangul Jamo
            | 0x2E80..=0x303E // CJK 部首/康熙部件/符号
            | 0x3041..=0x33FF // 平假名..CJK 兼容
            | 0x3400..=0x4DBF // 扩展 A
            | 0x4E00..=0x9FFF // CJK 统一表意
            | 0xA000..=0xA4CF // 彝文
            | 0xA960..=0xA97F // Hangul 扩展 A
            | 0xAC00..=0xD7A3 // Hangul 音节
            | 0xF900..=0xFAFF // CJK 兼容表意
            | 0xFE10..=0xFE19 // 竖排形
            | 0xFE30..=0xFE6F // CJK 兼容形
            | 0xFF01..=0xFF60 // 全角形
            | 0xFFE0..=0xFFE6 // 全角符号
            | 0x1F300..=0x1F64F // emoji(宽)
            | 0x1F900..=0x1F9FF // emoji 补充
            | 0x20000..=0x2FFFD // 扩展 B..F
            | 0x30000..=0x3FFFD // 扩展 G..
    )
}
