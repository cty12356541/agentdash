//! agentstats 观测面(W11-004,spec §1.4 / §2 D2):`agentdash stats [PATH]
//! [--host <name>]`——事件流的三张纯文本表:
//!
//! ① 宿主使用率:生效事件按行上 `host` 戳计数(hook `--host` 盖章,见
//!    [`crate::events`] 计数点);无戳/空白戳归 `unknown` 桶——诚实标注,
//!    不猜归属。
//! ② gate 通过率:终态折叠事件按(门名 × 宿主)计 passed / failed /
//!    unknown 三列;`unknown` = exit 不可知的失败折叠(W4-001 D1 的
//!    `(exit unknown)` 路径,证据缺失不与真失败混计);running 在途不算。
//!    折叠仍由 replay 独家产出,本表只对折叠产物计数投影,不重算折叠。
//! ③ 任务周转:契约只有 `done_at` 没有 `started_at`(`contract.rs` 全文
//!    无任何起始字段),时长不可算、不虚构——逐行列出 done 任务的完成时刻
//!    原串 + 计数,无 `done_at` 者不列行、以 N/A 汇总;口径局限写进表头。
//!
//! 纯投影(D2):只读 `<PATH>/.agentdash/` 两个约定文件,不写任何文件;
//! 零 ANSI(oneline/digest 先例);退出码恒 0——内容永不影响退出;无生效
//! 事件 → 单行空态。仿 digest.rs 先例:私有辅助 + [`run`] 单入口平铺。

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use crate::contract::{self, TaskState};
use crate::events::{self, EventModel, GateFoldTally};

/// 目录约定(spec §4):`<PATH>/.agentdash/` 下的两份数据源。
const EVENTS_NAME: &str = "events.jsonl";
const LEDGER_NAME: &str = "ledger.json";

/// stats 子命令入口:`PATH` 缺省 `.`(调用方收口参数解析),`--host` 过滤
/// ①② 两表。读文件、replay、渲染、打印,恒退 0(D2)。
pub fn run(path: &Path, host: Option<&str>) -> ExitCode {
    let dir = path.join(".agentdash");
    let model = match fs::read_to_string(dir.join(EVENTS_NAME)) {
        Ok(text) => events::replay(text.lines().map(str::to_owned)),
        Err(_) => events::EventModel::default(),
    };
    // 空态口径:零生效事件(文件缺失、为空或全为残缺)→ 单行空态,不出表
    if model.host_events.values().sum::<u64>() == 0 {
        println!("stats: 无生效事件({EVENTS_NAME} 缺失、为空或全为残缺行)");
        return ExitCode::SUCCESS;
    }
    let ledger = fs::read_to_string(dir.join(LEDGER_NAME)).ok();
    println!("{}", render(&model, ledger.as_deref(), host));
    ExitCode::SUCCESS
}

/// 三表渲染(纯函数,零 ANSI):`host` 给定时 ①② 只留该宿主(两表皆空 →
/// 单行说明,缺项零残留),③ 台账任务无宿主字段、行不随过滤(表内注明);
/// 台账缺失 → ③ 整节零残留,损坏 → 降级说明行。
#[must_use]
pub fn render(model: &EventModel, ledger: Option<&str>, host: Option<&str>) -> String {
    let mut lines: Vec<String> = Vec::new();
    let hosts = filtered_hosts(model, host);
    let folds = filtered_folds(model, host);

    // 未知宿主:①② 皆空 → 单行说明,不让"空"哑然无解释
    if let Some(name) = host
        && hosts.is_empty()
        && folds.is_empty()
    {
        lines.push(format!("宿主 {name}: 无任何事件记录"));
    }
    render_usage(&mut lines, &hosts);
    render_folds(&mut lines, &folds);
    render_turnover(&mut lines, ledger, host.is_some());
    lines.join("\n")
}

/// ① 宿主使用率:行序 = 宿主名升序、`unknown` 桶恒排尾(兜底桶不冒充实宿主)。
fn render_usage(lines: &mut Vec<String>, hosts: &[(String, u64)]) {
    if hosts.is_empty() {
        return; // 缺项零残留:不打空标题
    }
    let width = hosts.iter().map(|(name, _)| name.len()).max().unwrap_or(1);
    lines.push("宿主使用率(生效事件按 host 戳计数;无戳归 unknown)".into());
    for (name, count) in hosts {
        lines.push(format!("  {name:<width$}  {count}"));
    }
}

/// ② gate 通过率:行序 = 门名升序、同门内宿主名升序(`unknown` 恒尾);
/// 列对齐(gate/host 左对齐,计数右对齐),表头即口径注。
fn render_folds(lines: &mut Vec<String>, folds: &[(String, String, GateFoldTally)]) {
    if folds.is_empty() {
        return;
    }
    let gate_w = folds
        .iter()
        .map(|(gate, _, _)| gate.len())
        .max()
        .unwrap_or(1);
    let host_w = folds
        .iter()
        .map(|(_, host, _)| host.len())
        .max()
        .unwrap_or(1);
    lines.push("gate 通过率(终态折叠计数;unknown = exit 不可知折叠)".into());
    lines.push(format!(
        "  {:<gate_w$}  {:<host_w$}  {:>6}  {:>6}  {:>7}",
        "gate", "host", "passed", "failed", "unknown"
    ));
    for (gate, host, tally) in folds {
        lines.push(format!(
            "  {gate:<gate_w$}  {host:<host_w$}  {:>6}  {:>6}  {:>7}",
            tally.passed, tally.failed, tally.unknown
        ));
    }
}

/// ③ 任务周转:表头自带口径局限声明(契约无 `started_at`,仅列完成时刻,
/// 不虚构时长);行 = done 任务的 `id label · done_at` 原串(按 id 升序),
/// 无 `done_at` 者不列行、以 N/A 汇总收口;`--host` 在场加注(台账无宿主
/// 字段,过滤不适用);台账缺失整节零残留,损坏降级说明行。
fn render_turnover(lines: &mut Vec<String>, ledger: Option<&str>, filtered: bool) {
    let Some(text) = ledger else {
        return;
    };
    lines.push("任务周转: 契约无 started_at,仅列完成时刻(不虚构时长)".into());
    if filtered {
        lines.push("  (台账任务无宿主字段,本表不随 --host 过滤)".into());
    }
    let Ok(ledger) = contract::parse_ledger(text) else {
        lines.push("周转: ledger.json 损坏,无法统计".into());
        return;
    };
    let mut done: Vec<(&String, &contract::TaskSpec)> = ledger
        .tasks
        .iter()
        .filter(|(_, task)| task.state == TaskState::Done)
        .collect();
    done.sort_by(|a, b| a.0.cmp(b.0)); // HashMap 序不定,按 id 排序保证输出确定
    let mut unstamped = 0usize;
    for (id, task) in done {
        match task
            .done_at
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(stamp) => lines.push(format!("  {id} {} · {stamp}", task.label)),
            None => unstamped += 1,
        }
    }
    if unstamped > 0 {
        lines.push(format!("  N/A(无 done_at): {unstamped}"));
    }
}

/// ① 的过滤 + 排序投影:`host` 给定时只留该宿主行。
fn filtered_hosts(model: &EventModel, host: Option<&str>) -> Vec<(String, u64)> {
    let mut rows: Vec<(String, u64)> = model
        .host_events
        .iter()
        .filter(|(name, _)| match host {
            Some(want) => name.as_str() == want,
            None => true,
        })
        .map(|(name, count)| (name.clone(), *count))
        .collect();
    rows.sort_by(|a, b| host_sort_key(&a.0).cmp(&host_sort_key(&b.0)));
    rows
}

/// ② 的过滤 + 排序投影:同上,键为(门名, 宿主)。
fn filtered_folds(model: &EventModel, host: Option<&str>) -> Vec<(String, String, GateFoldTally)> {
    let mut rows: Vec<(String, String, GateFoldTally)> = model
        .gate_folds
        .iter()
        .filter(|((_, name), _)| match host {
            Some(want) => name.as_str() == want,
            None => true,
        })
        .map(|((gate, name), tally)| (gate.clone(), name.clone(), tally.clone()))
        .collect();
    rows.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| host_sort_key(&a.1).cmp(&host_sort_key(&b.1)))
    });
    rows
}

/// 宿主排序键:名字升序,`unknown` 桶恒排尾。
fn host_sort_key(name: &str) -> (bool, &str) {
    (name == events::UNKNOWN_HOST, name)
}
