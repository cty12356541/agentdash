//! 终端字符 DAG:拓扑分层布局 + box-drawing 连接符 + 鼠标命中几何(W1-006,
//! 移植自 claude-dash `dashlib/render_graph.py`)。
//!
//! 分层 = 最长路径层号(同车道 id 序链 + 屏障边);环保险:迭代上限后剩余
//! 节点并入末层并列,不崩溃(结构缺陷降级不白屏)。[`layout_layers`] 是几何
//! 唯一源:render 画图与 [`hit_test`] 点击命中共用同一 [`Cell`] 表,后续
//! watch 车道亦经它消费。
//!
//! 路由策略(与"节点+边真图"承诺一致):
//! - 父框底中心 ┬ 出线;子框顶中心以 ▼ 入线(替换顶框 ─)
//! - 相邻层:层间两行,│ 直落或 └─┐/┌─┘ 角折
//! - 跨多层:竖线沿父中心列穿过中间层行(仅写入空白,遇节点框让路),在
//!   子框顶上方一格的汇流行合并为水平段;父列在段中用 ┴ 三通、段端用
//!   └/┘ 角折,子中心列以 │ 接 ▼ 入框顶

use std::collections::{HashMap, HashSet};

use std::cmp::Reverse;

use super::{C_END, clamp_width, display_width, project_label, visual};
use crate::model::{Dashboard, TaskView};

/// 节点单元格内 label 截断宽(承 Python `_CELL_LABEL`)。
const CELL_LABEL: usize = 16;
/// 同层节点列间距。
const CELL_GAP: usize = 2;
/// 输出头两行:标题 + 分隔线。
const HEAD_LINES: usize = 2;
/// 每节点框 3 行(顶/文字/底)、层间空档 2 行。
const BOX_ROW: usize = 3;
const ZONE_ROW: usize = 2;
const LAYER_PITCH: usize = BOX_ROW + ZONE_ROW;
/// 画布宽(字符列表行缓冲)。
const CANVAS_W: usize = 240;
/// graph 视图默认宽(分隔线宽度)。
pub const DEFAULT_GRAPH_WIDTH: usize = 72;

/// 着色文字覆盖:(行, 列, 着色文字, 纯文本显示宽)。
type Overlay = (usize, usize, String, usize);

/// 文字行 → 该行节点框的列区间表(路由让路判定用)。
type BoxRanges = HashMap<usize, Vec<(usize, usize)>>;

/// 图侧屏障边(布局/渲染输入;W1-007 起模型携带同构的
/// `model::BarrierEdges`,默认入口 [`render_graph`] 自动换形消费,
/// 显式传参路径保留给测试与手工构造)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BarrierEdges {
    /// 前置任务 id 集(after)。
    pub after: Vec<String>,
    /// 放行任务 id 集(unlocks)。
    pub unlocks: Vec<String>,
}

/// 布局单元格:节点框几何(ANSI 剥离后的显示列坐标,0 基)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// 任务 id。
    pub id: String,
    /// 任务 label(未截断原文)。
    pub label: String,
    /// 节点框左缘列。
    pub x: usize,
    /// 框外沿宽(含边框,显示列)。
    pub width: usize,
    /// 节点文字所在输出行;框占 line±1。
    pub line: usize,
}

impl Cell {
    /// 框中心列(承 Python `x + width // 2`)。
    #[must_use]
    pub fn center(&self) -> usize {
        self.x + self.width / 2
    }
}

/// child -> 去重父表(首见序;承 Python set 语义但保序,输出确定性)。
#[derive(Debug, Default)]
struct Edges {
    /// 子节点首见序(Python dict 插入序的等价物)。
    order: Vec<String>,
    /// child -> 父 id 表。
    parents: HashMap<String, Vec<String>>,
}

impl Edges {
    fn add(&mut self, child: &str, parent: &str) {
        let parents = self.parents.entry(child.to_owned()).or_default();
        if !parents.iter().any(|p| p == parent) {
            parents.push(parent.to_owned());
        }
        if !self.order.iter().any(|c| c == child) {
            self.order.push(child.to_owned());
        }
    }

    fn get(&self, child: &str) -> &[String] {
        self.parents.get(child).map_or(&[], Vec::as_slice)
    }
}

/// 边收集:同车道按 id 序为链;屏障 after → unlocks(两端未知 id 过滤)。
fn edges_of(dash: &Dashboard, barriers: &[BarrierEdges]) -> Edges {
    let mut edges = Edges::default();
    // 同车道任务按 id 序成链(车道按首见序分组)
    let mut lanes: Vec<(Option<&str>, Vec<&TaskView>)> = Vec::new();
    for task in &dash.tasks {
        match lanes
            .iter_mut()
            .find(|(lane, _)| *lane == task.lane.as_deref())
        {
            Some((_, members)) => members.push(task),
            None => lanes.push((task.lane.as_deref(), vec![task])),
        }
    }
    for (_, members) in &mut lanes {
        members.sort_unstable_by(|a, b| a.id.cmp(&b.id));
    }
    for (_, members) in &lanes {
        for pair in members.windows(2) {
            edges.add(&pair[1].id, &pair[0].id);
        }
    }
    let ids: HashSet<&str> = dash.tasks.iter().map(|t| t.id.as_str()).collect();
    for barrier in barriers {
        for parent in &barrier.after {
            for child in &barrier.unlocks {
                if ids.contains(parent.as_str()) && ids.contains(child.as_str()) {
                    edges.add(child, parent);
                }
            }
        }
    }
    edges
}

/// 最长路径分层;环内节点(迭代上限未解析)并入末层并列,保证全量输出。
fn compute_layers<'a>(dash: &'a Dashboard, edges: &Edges) -> Vec<Vec<&'a TaskView>> {
    let mut layer: HashMap<&str, usize> = HashMap::new();
    for _ in 0..=dash.tasks.len() {
        let mut progressed = false;
        for task in &dash.tasks {
            if layer.contains_key(task.id.as_str()) {
                continue;
            }
            let parents = edges.get(&task.id);
            let ready = parents.iter().all(|p| layer.contains_key(p.as_str()));
            if !ready {
                continue;
            }
            let level = parents
                .iter()
                .filter_map(|p| layer.get(p.as_str()))
                .copied()
                .max()
                .map_or(0, |max| max + 1);
            layer.insert(task.id.as_str(), level);
            progressed = true;
        }
        if !progressed {
            break;
        }
    }
    let last = layer.values().copied().max().map_or(0, |max| max + 1);
    for task in &dash.tasks {
        layer.entry(task.id.as_str()).or_insert(last);
    }
    let mut layers: Vec<Vec<&TaskView>> = vec![Vec::new(); last + 1];
    for task in &dash.tasks {
        if let Some(level) = layer.get(task.id.as_str())
            && let Some(bucket) = layers.get_mut(*level)
        {
            bucket.push(task);
        }
    }
    let non_empty: Vec<Vec<&TaskView>> = layers
        .into_iter()
        .filter(|bucket| !bucket.is_empty())
        .collect();
    if !non_empty.is_empty() {
        non_empty
    } else if dash.tasks.is_empty() {
        vec![Vec::new()]
    } else {
        dash.tasks.iter().map(|t| vec![t]).collect()
    }
}

/// 节点单元格文字:`<mark> <id> <label[:16 字符]>`。
fn cell_text(task: &TaskView) -> String {
    format!(
        "{} {} {}",
        visual(task.state).mark(),
        task.id,
        task.label.chars().take(CELL_LABEL).collect::<String>()
    )
}

/// 着色单元格文字(框内文字行经覆盖层写入)。
fn colored_cell(task: &TaskView) -> String {
    format!("{}{}{}", visual(task.state).color(), cell_text(task), C_END)
}

/// 几何唯一源:每层节点的 (x, width, line)。
///
/// 节点带单线框(┌─┐│└┘):line 指向文字行,框占 line±1;同层节点以
/// `CELL_GAP` 间隔左→右排布。
#[must_use]
pub fn layout_layers(dash: &Dashboard, barriers: &[BarrierEdges]) -> Vec<Vec<Cell>> {
    let edges = edges_of(dash, barriers);
    let layers = compute_layers(dash, &edges);
    layout_rows(&layers)
}

fn layout_rows(layers: &[Vec<&TaskView>]) -> Vec<Vec<Cell>> {
    let mut rows = Vec::new();
    let mut line = HEAD_LINES + 1; // 首层文字行:标题+分隔线之后,留框顶
    for layer in layers {
        let mut cells = Vec::new();
        let mut col = 0;
        for task in layer {
            let text = cell_text(task);
            let width = display_width(&text) + 4; // 左右内边距各 1 + 边框各 1(显示列)
            cells.push(Cell {
                id: task.id.clone(),
                label: task.label.clone(),
                x: col,
                width,
                line,
            });
            col += width + CELL_GAP;
        }
        rows.push(cells);
        line += LAYER_PITCH;
    }
    rows
}

/// 默认入口:消费 `Dashboard.barriers`(W1-007 起模型携带屏障,双轨收敛——
/// 默认版把模型屏障换形后委托 [`render_graph_with`];需要显式屏障集的
/// 调用方仍可用后者)。
#[must_use]
pub fn render_graph(dash: &Dashboard, width: usize) -> String {
    let barriers: Vec<BarrierEdges> = dash
        .barriers
        .iter()
        .map(|barrier| BarrierEdges {
            after: barrier.after.clone(),
            unlocks: barrier.unlocks.clone(),
        })
        .collect();
    render_graph_with(dash, &barriers, width)
}

/// 框化节点 DAG:任意跨层边均路由(竖穿 + 角折 + ▼ 入框顶)。
#[must_use]
pub fn render_graph_with(dash: &Dashboard, barriers: &[BarrierEdges], width: usize) -> String {
    let width = clamp_width(width);
    let edges = edges_of(dash, barriers);
    let layers = compute_layers(dash, &edges);
    let rows = layout_rows(&layers);
    let by_id: HashMap<&str, &TaskView> = dash
        .tasks
        .iter()
        .map(|task| (task.id.as_str(), task))
        .collect();
    let cell_of: HashMap<&str, &Cell> = rows
        .iter()
        .flatten()
        .map(|cell| (cell.id.as_str(), cell))
        .collect();
    let out_edge_parents: HashSet<&str> = edges
        .parents
        .values()
        .flatten()
        .map(String::as_str)
        .collect();

    let mut canvas =
        vec![vec![' '; CANVAS_W]; HEAD_LINES + 1 + LAYER_PITCH * layers.len().max(1) + 2];
    let (overlays, box_rows) = draw_node_boxes(&mut canvas, &rows, &by_id, &out_edge_parents);
    let in_box = |row: usize, col: usize| -> bool {
        box_rows
            .get(&row)
            .is_some_and(|ranges| ranges.iter().any(|(x0, x1)| *x0 <= col && col < *x1))
    };
    route_child_edges(&mut canvas, &edges, &cell_of, &in_box);
    assemble_output(&canvas, overlays, project_label(dash), width)
}

/// 步骤 1:画节点框(顶/底横线 + 四角 + 文字行侧框),有出边的框底打 ┬ 出线桩;
/// 返回着色文字覆盖与"文字行 → 框列区间"表(路由让路判定用)。
fn draw_node_boxes(
    canvas: &mut [Vec<char>],
    rows: &[Vec<Cell>],
    by_id: &HashMap<&str, &TaskView>,
    out_edge_parents: &HashSet<&str>,
) -> (Vec<Overlay>, BoxRanges) {
    let mut overlays: Vec<Overlay> = Vec::new();
    let mut box_rows: BoxRanges = HashMap::new();
    for cells in rows {
        for cell in cells {
            let task = by_id[cell.id.as_str()];
            let (top, txt, bot) = (cell.line - 1, cell.line, cell.line + 1);
            for x in cell.x..cell.x + cell.width {
                put(canvas, top, x, '─', false);
                put(canvas, bot, x, '─', false);
            }
            put(canvas, top, cell.x, '┌', false);
            put(canvas, top, cell.x + cell.width - 1, '┐', false);
            put(canvas, bot, cell.x, '└', false);
            put(canvas, bot, cell.x + cell.width - 1, '┘', false);
            if out_edge_parents.contains(cell.id.as_str()) {
                put(canvas, bot, cell.center(), '┬', false); // 出线桩
            }
            put(canvas, txt, cell.x, '│', false);
            put(canvas, txt, cell.x + cell.width - 1, '│', false);
            overlays.push((
                txt,
                cell.x + 2, // 内边距 1 + 边框 1
                colored_cell(task),
                display_width(&cell_text(task)),
            ));
            box_rows
                .entry(txt)
                .or_default()
                .push((cell.x, cell.x + cell.width));
        }
    }
    (overlays, box_rows)
}

/// 步骤 2:边路由(任意层距;按子节点统一汇流)——各父框底中心垂直下落
/// (跨层时穿过中间层空白带,遇框让路),在子框顶上方一格的汇流行合并为
/// 水平段;父列在段中用 ┴ 三通、段端用 └/┘ 角折,子中心列以 │ 接 ▼ 入框顶。
fn route_child_edges(
    canvas: &mut [Vec<char>],
    edges: &Edges,
    cell_of: &HashMap<&str, &Cell>,
    in_box: &impl Fn(usize, usize) -> bool,
) {
    for cid in &edges.order {
        let Some(cell) = cell_of.get(cid.as_str()).copied() else {
            continue;
        };
        let mut drop_cols = Vec::new();
        for pid in edges.get(cid) {
            let Some(parent) = cell_of.get(pid.as_str()).copied() else {
                continue;
            };
            let z_final = cell.line.saturating_sub(2);
            for row in parent.line + 2..z_final {
                if !in_box(row, parent.center()) {
                    put(canvas, row, parent.center(), '│', true);
                }
            }
            drop_cols.push(parent.center());
        }
        if drop_cols.is_empty() {
            continue;
        }
        let z = cell.line - 2;
        let lo = drop_cols
            .iter()
            .copied()
            .chain([cell.center()])
            .min()
            .expect("非空");
        let hi = drop_cols
            .iter()
            .copied()
            .chain([cell.center()])
            .max()
            .expect("非空");
        for x in lo..=hi {
            put(canvas, z, x, '─', true);
        }
        for x in drop_cols {
            // 父列接合符(覆盖自家水平段)
            if x == cell.center() {
                put(canvas, z, x, '│', false);
            } else if lo < x && x < hi {
                put(canvas, z, x, '┴', false);
            } else if x == lo {
                put(canvas, z, x, '└', false);
            } else {
                put(canvas, z, x, '┘', false);
            }
        }
        if cell.center() < CANVAS_W && canvas[z][cell.center()] == '─' {
            canvas[z][cell.center()] = '│'; // 子中心是段端且无父列:接下笔
        }
        put(canvas, cell.line - 1, cell.center(), '▼', false); // 箭头嵌子框顶
    }
}

/// 步骤 3:输出——纯字符行 + 着色覆盖(canvas 行号即最终输出行号:行 0/1 由
/// 标题/分隔线占据,框从行 2 起;覆盖按纯文本显示宽从右往左切片,ANSI 长度
/// 不会吃掉框右缘),首尾各补一条宽度分隔线。
fn assemble_output(
    canvas: &[Vec<char>],
    overlays: Vec<Overlay>,
    project: &str,
    width: usize,
) -> String {
    let mut lines = vec![format!("{project} · DAG"), "─".repeat(width)];
    let mut by_row: HashMap<usize, Vec<(usize, String, usize)>> = HashMap::new();
    for (row, col, text, plen) in overlays {
        by_row.entry(row).or_default().push((col, text, plen));
    }
    for splices in by_row.values_mut() {
        // 从右往左覆盖,低列切片不受高列位移影响
        splices.sort_by_key(|(col, _, _)| Reverse(*col));
    }
    let mut last = canvas.len() - 1;
    while last >= HEAD_LINES && canvas[last].iter().all(|ch| *ch == ' ') {
        last -= 1; // 裁尾部全空行(画布过量分配)
    }
    for (offset, row_chars) in canvas[HEAD_LINES..=last].iter().enumerate() {
        let row = HEAD_LINES + offset;
        let mut chars = row_chars.clone();
        if let Some(splices) = by_row.get(&row) {
            for (col, text, plen) in splices {
                if *col < chars.len() {
                    let end = (*col + *plen).min(chars.len());
                    chars.splice(*col..end, text.chars());
                }
            }
        }
        let line: String = chars.into_iter().collect();
        lines.push(line.trim_end().to_owned());
    }
    lines.push("─".repeat(width));
    lines.join("\n")
}

/// 终端 1 基 (col,row) → 命中的任务 id;命中区 = 节点框三行整框;空白 `None`。
#[must_use]
pub fn hit_test(layers: &[Vec<Cell>], col: usize, row_1based: usize) -> Option<String> {
    let row = row_1based.checked_sub(1)?;
    for cells in layers {
        for cell in cells {
            if cell.line - 1 <= row
                && row <= cell.line + 1
                && cell.x <= col
                && col < cell.x + cell.width
            {
                return Some(cell.id.clone());
            }
        }
    }
    None
}

/// 写一个画布格;`only_space` 时仅写入空白格(连线遇节点框/已有连线让路)。
fn put(canvas: &mut [Vec<char>], row: usize, col: usize, ch: char, only_space: bool) -> bool {
    if row < canvas.len() && col < CANVAS_W && (!only_space || canvas[row][col] == ' ') {
        canvas[row][col] = ch;
        true
    } else {
        false
    }
}
