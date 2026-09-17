//! 渲染层(W1-006):把 [`Dashboard`](crate::model::Dashboard) 折叠为三个终端视图。
//!
//! - [`oneline::render_oneline`]:无 ANSI 单行(statusline);
//! - [`panel::render_panel`]:面板——页眉统计 → 健康 → PR → 轨迹 → 车道;
//! - [`digest::render_digest`]:离场摘要——纯文本无 ANSI(W10-002);
//! - [`graph::render_graph`]:框化节点 DAG(┌─┐ 框、┬ 出线桩、▼ 入线箭头)。
//!
//! 算法移植自 claude-dash `dashlib/render_oneline.py` / `render_panel.py` /
//! `render_graph.py`,以其实测黄金断言语义为准;输入面收敛为
//! `model::Dashboard`。模型暂缺的展示字段(activity)以空呈现并留 TODO;
//! velocity(速度线)已随 W3-004 入模(`model::velocity`),barriers 随
//! W1-007、project 随 W3-004(D3)入模。

mod digest;
pub mod graph;
mod oneline;
mod panel;

// graph 面不做平铺 re-export:本模块被 bin 与各集成测试目标分别挂载,各目标
// 消费面不同构(如 render_panel 目标不消费 graph 组),平铺必有逐目标未用
// import;消费方一律走 `graph::` 路径。panel/oneline 为既有渲染目标全消费,
// 保持平铺(digest 测试目标 W10-002 起不消费 oneline,窄域放行承下)
#[allow(unused_imports)]
pub use oneline::render_oneline;
// render_brief(W10-001):多仓聚合的每仓精要块。仅 bin(panel 多仓路由)与
// render_panel 测试目标消费,其余挂载目标不消费——平铺 re-export 会逐目标
// 报未用 import,窄域放行(与上面 graph 面不平铺是同一消费面分化,反向取用)
#[allow(unused_imports)]
pub use panel::{DEFAULT_PANEL_WIDTH, render_brief, render_panel, render_panel_rows};
// render_digest / digest_needs_attention(W10-002):离场摘要 + --strict 判据。
// 仅 bin(render digest 路由)与 digest 测试目标消费,其余挂载目标不消费——
// 平铺 re-export 会逐目标报未用 import,窄域放行(承 render_brief 同理)
#[allow(unused_imports)]
pub use digest::{digest_needs_attention, render_digest};

use crate::contract::TaskState;
use crate::model::{Dashboard, MilestoneView, TaskView, rfc3339_to_secs};

// ANSI 调色(承 Python `C` 表;oneline 视图不用)。
pub(crate) const C_DONE: &str = "\x1b[32m";
pub(crate) const C_ACTIVE: &str = "\x1b[34m";
pub(crate) const C_PENDING: &str = "\x1b[90m";
pub(crate) const C_STALLED: &str = "\x1b[33m";
pub(crate) const C_WARN: &str = "\x1b[33m";
pub(crate) const C_BOLD: &str = "\x1b[1m";
pub(crate) const C_END: &str = "\x1b[0m";

/// 宽度钳下限(Brief:40..120);低于它的终端出不了框化视图——形态退化
/// 判定(AD-ERR-004,见 `tui::output_form`)以它为线。
pub const MIN_WIDTH: usize = 40;
/// 宽度钳上限(Brief:40..120)。
pub const MAX_WIDTH: usize = 120;

/// 宽度钳(Brief:40..120);panel/graph 的宽度参数一律先经此钳位。
pub(crate) const fn clamp_width(width: usize) -> usize {
    if width < MIN_WIDTH {
        MIN_WIDTH
    } else if width > MAX_WIDTH {
        MAX_WIDTH
    } else {
        width
    }
}

// 项目名(W2-3b 落地原 TODO(model) 去硬编码;W3-004 D3 起退役):回退链
// 上收 `model`(git 仓根名 → cwd 目录名 → `"agentdash"`),合并时算好进
// `Dashboard::project`,渲染层只读字段不再自猜。原 `project_label` /
// `dir_name` 已删,消费方一律读 `dash.project`。

/// 物证交叉核对(W5-001):done 且自带 `done_at`、事件源在场,且(**无任何
/// 通过门**或 **`done_at` 晚于最近通过门**)→ `true`(自报无物证,任务行
/// `?`)。无 `done_at`(人工维护,不可断言)、事件源不在场(无证可查不作
/// 怀疑,承 W3 速度线同款克制)或时刻不可解析 → `false`。
pub(crate) fn unattested_done(task: &TaskView, dash: &Dashboard) -> bool {
    if task.state != TaskState::Done {
        return false;
    }
    let Some(done_at) = task.done_at.as_deref().and_then(rfc3339_to_secs) else {
        return false;
    };
    if !dash.events_present {
        return false;
    }
    match dash.last_gate_passed.as_deref().and_then(rfc3339_to_secs) {
        Some(passed) => done_at > passed,
        None => true,
    }
}

/// 折叠车道伪任务判别(W2-005):tui 折叠视图把完成车道折叠成单条伪任务,
/// 以**空 id** 为哨兵——合法台账 id 与 git 短 SHA 均非空,空 id 唯一标识
/// 折叠行;panel/graph 据此出 `▸ 车道名 (N done)` 单行而非逐任务铺开。
pub(crate) fn is_lane_marker(task: &TaskView) -> bool {
    task.id.is_empty()
}

/// 任务五态视觉映射(承 claude-dash 五态):done ✓ 绿 / active ▶ 蓝 /
/// stalled ⚑ 黄 / pending · 灰 / blocked ⊘ 灰(W2-3b 起 blocked 独立符号,
/// 不再与 pending 同点)。agentdash 富态归并:`Review` 归 active(复核
/// 进行中),`FixRound` 归 stalled(返修待办)。
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
    /// 单字符标记:`✓▶·⚑` 经 Python `unicodedata` 核对均为窄字符(1 列);
    /// `⊘`(blocked,U+2298)EAW=A,与 `⚑` 同策略按 1 列计。
    pub(crate) fn mark(self) -> &'static str {
        match self {
            Self::Done => "✓",
            Self::Active => "▶",
            Self::Stalled => "⚑",
            Self::Pending => "·",
            Self::Blocked => "⊘",
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

/// `now_iso[5:16]` 同位切片(panel 与 tui 详情栏同一算法;W5-003 双份收敛):
/// UTC/本地时刻的 `MM-DDTHH:MM` 段(不足则原样返回)。
pub(crate) fn clock_slice(rfc3339: &str) -> String {
    if rfc3339.chars().count() >= 16 {
        rfc3339.chars().skip(5).take(11).collect()
    } else {
        rfc3339.to_owned()
    }
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

/// 按显示宽截断到 `budget` 列内的最长前缀(panel 与详情栏同一算法;
/// W4-003:panel 私有副本已并入本公共版,双轨收敛完成)。
pub(crate) fn truncate_width(text: &str, budget: usize) -> String {
    for cut in (0..=text.chars().count()).rev() {
        let prefix: String = text.chars().take(cut).collect();
        if display_width(&prefix) <= budget {
            return prefix;
        }
    }
    String::new()
}

/// 截断并在发生截断时以 `…`(1 列)收尾;总宽仍不超 `budget`。
pub(crate) fn elide(text: &str, budget: usize) -> String {
    let cut = truncate_width(text, budget);
    if cut.chars().count() == text.chars().count() {
        return cut;
    }
    let mut out = truncate_width(text, budget.saturating_sub(1));
    if display_width(&out) < budget {
        out.push('…');
    }
    out
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
