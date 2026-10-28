//! agentstats 观测面(W11-004,spec §1.4 / §2 D2;W14-07 契约 v1.1 升级):
//! `agentdash stats [PATH] [--host <name>]`——事件流与台账的三张纯文本表:
//!
//! ① 宿主使用率:生效事件按行上 `host` 戳计数(hook `--host` 盖章,见
//!    [`crate::events`] 计数点);无戳/空白戳归 `unknown` 桶——诚实标注,
//!    不猜归属。列宽按东亚终端显示宽([`display_width`])对齐。
//! ② gate 通过率:终态折叠事件按(门名 × 宿主)计 passed / failed /
//!    unknown 三列;`unknown` = exit 不可知的失败折叠(W4-001 D1 的
//!    `(exit unknown)` 路径,证据缺失不与真失败混计);running 在途不算。
//!    折叠仍由 replay 独家产出,本表只对折叠产物计数投影,不重算折叠;
//!    `gate`/`host` 列按 [`display_width`] 并保底 4 列对齐表头。
//! ③ 任务周转:契约 v1.1 起优先按合法 `(started_at, done_at)` 对(`done_at >=
//!    started_at`)折算真周转时长并汇总中位 / 最大 / 均值(样本数),同账缺失或
//!    无效 `started_at` 的完成行诚实标 `(无有效 started_at)`;若全账无任何合法
//!    计时对(v1.0 旧台账),则回退仅列 `done_at` 原串(黄金逐字节不变,不虚构
//!    时长);无 `done_at` 者不列行、以 N/A 汇总。
//!
//! 纯投影(D2):只读 `<PATH>/.agentdash/` 两个约定文件,不写任何文件;
//! 零 ANSI(oneline/digest 先例);退出码恒 0——内容永不影响退出;无生效
//! 事件且无台账 → 单行空态,有台账仍照常渲染 ③ 周转表。仿 digest.rs 先例:
//! 私有辅助 + [`run`] 单入口平铺。

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use crate::contract::{self, TaskState};
use crate::events::{self, EventModel, GateFoldTally};
use crate::model::rfc3339_to_secs;
use crate::render::display_width;

/// 目录约定(spec §4):`<PATH>/.agentdash/` 下的两份数据源。
const EVENTS_NAME: &str = "events.jsonl";
const LEDGER_NAME: &str = "ledger.json";

/// stats 子命令入口:`PATH` 缺省 `.`(调用方收口参数解析),`--host` 过滤
/// ①② 两表(大小写不敏感)。读文件、replay、渲染、打印,恒退 0(D2)。
pub fn run(path: &Path, host: Option<&str>) -> ExitCode {
    let dir = path.join(".agentdash");
    let model = match fs::read_to_string(dir.join(EVENTS_NAME)) {
        Ok(text) => events::replay(text.lines().map(str::to_owned)),
        Err(_) => events::EventModel::default(),
    };
    let ledger = fs::read_to_string(dir.join(LEDGER_NAME)).ok();
    // 空态口径:零生效事件且无台账 → 单行空态,不出表;台账在场则照出 ③ 周转表
    if model.host_events.values().sum::<u64>() == 0 && ledger.is_none() {
        println!("stats: 无生效事件({EVENTS_NAME} 缺失、为空或全为残缺行)");
        return ExitCode::SUCCESS;
    }
    println!("{}", render(&model, ledger.as_deref(), host));
    ExitCode::SUCCESS
}

/// 三表渲染(纯函数,零 ANSI):`host` 给定时 ①② 只留该宿主(大小写不敏感;
/// 两表皆空 → 单行说明,缺项零残留),未给定 `host` 且全仓零生效事件时首行
/// 出空事件提示;③ 台账任务无宿主字段、行不随过滤(表内注明);台账缺失 →
/// ③ 整节零残留,损坏 → 降级说明行。
#[must_use]
pub fn render(model: &EventModel, ledger: Option<&str>, host: Option<&str>) -> String {
    let mut lines: Vec<String> = Vec::new();
    let host = host.map(str::trim).filter(|h| !h.is_empty());
    let hosts = filtered_hosts(model, host);
    let folds = filtered_folds(model, host);

    // 未知宿主:①② 皆空 → 单行说明;无 --host 且全仓零生效事件 → 空事件提示行
    if let Some(name) = host {
        if hosts.is_empty() && folds.is_empty() {
            lines.push(format!("宿主 {name}: 无任何事件记录"));
        }
    } else if model.host_events.values().sum::<u64>() == 0 {
        lines.push(format!(
            "stats: 无生效事件({EVENTS_NAME} 缺失、为空或全为残缺行)"
        ));
    }
    render_usage(&mut lines, &hosts);
    render_folds(&mut lines, &folds);
    render_turnover(&mut lines, ledger, host.is_some());
    lines.join("\n")
}

/// 按东亚终端显示宽右补半角空格(Rust 标准 `{s:<w$}` 按标量值计数,CJK 少补列)。
fn pad_right(s: &str, width: usize) -> String {
    format!("{s}{}", " ".repeat(width.saturating_sub(display_width(s))))
}

/// ① 宿主使用率:行序 = 宿主名升序、`unknown` 桶恒排尾(兜底桶不冒充实宿主)。
fn render_usage(lines: &mut Vec<String>, hosts: &[(String, u64)]) {
    if hosts.is_empty() {
        return; // 缺项零残留:不打空标题
    }
    let width = hosts
        .iter()
        .map(|(name, _)| display_width(name))
        .max()
        .unwrap_or(1);
    lines.push("宿主使用率(生效事件按 host 戳计数;无戳归 unknown)".into());
    for (name, count) in hosts {
        lines.push(format!("  {}  {count}", pad_right(name, width)));
    }
}

/// ② gate 通过率:行序 = 门名升序、同门内宿主名升序(`unknown` 恒尾);
/// 列对齐(gate/host 按显示宽左对齐且保底 4 列对齐表头,计数右对齐),表头即口径注。
fn render_folds(lines: &mut Vec<String>, folds: &[(String, String, GateFoldTally)]) {
    if folds.is_empty() {
        return;
    }
    let gate_w = folds
        .iter()
        .map(|(gate, _, _)| display_width(gate))
        .max()
        .unwrap_or(1)
        .max(4);
    let host_w = folds
        .iter()
        .map(|(_, host, _)| display_width(host))
        .max()
        .unwrap_or(1)
        .max(4);
    lines.push("gate 通过率(终态折叠计数;unknown = exit 不可知折叠)".into());
    lines.push(format!(
        "  {}  {}  {:>6}  {:>6}  {:>7}",
        pad_right("gate", gate_w),
        pad_right("host", host_w),
        "passed",
        "failed",
        "unknown"
    ));
    for (gate, host, tally) in folds {
        lines.push(format!(
            "  {}  {}  {:>6}  {:>6}  {:>7}",
            pad_right(gate, gate_w),
            pad_right(host, host_w),
            tally.passed,
            tally.failed,
            tally.unknown
        ));
    }
}

/// 合法计时周转行(契约 v1.1 `started_at` + `done_at`,且 `done_at >= started_at`)。
struct TimedRow<'a> {
    id: &'a str,
    label: &'a str,
    started_at: &'a str,
    done_at: &'a str,
    dur: u64,
}

/// 秒级周转时长格式化:`<60` → `{secs}s`;`<3600` → 整分 `{m}m` 或 `{m}m{ss:02}s`;
/// `>=3600` → `{h}h{mm:02}m{ss:02}s`。
fn fmt_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3_600 {
        if secs.is_multiple_of(60) {
            format!("{}m", secs / 60)
        } else {
            format!("{}m{:02}s", secs / 60, secs % 60)
        }
    } else {
        format!(
            "{}h{:02}m{:02}s",
            secs / 3_600,
            (secs % 3_600) / 60,
            secs % 60
        )
    }
}

/// ③ 任务周转:至少 1 条合法 `(started_at, done_at)` 计时对时输出中位 / 最大 /
/// 均值(样本数)与起止明细,其余仅有 `done_at` 的完成行附 `(无有效 started_at)`;
/// 全账零合法计时对时保持 v1.0 旧台账黄金输出逐字节不变(仅列完成时刻,不虚构时长);
/// 无 `done_at` 者不列行、以 N/A 汇总收口;`--host` 在场加注;台账缺失整节零残留,
/// 损坏降级说明行。
fn render_turnover(lines: &mut Vec<String>, ledger: Option<&str>, filtered: bool) {
    let Some(text) = ledger else {
        return;
    };
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

    let mut timed: Vec<TimedRow<'_>> = Vec::new();
    let mut untimed: Vec<(&str, &str, &str)> = Vec::new();
    let mut unstamped = 0usize;

    for (id, task) in done {
        let Some(d_str) = task
            .done_at
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            unstamped += 1;
            continue;
        };
        let start_stamp = task
            .started_at
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let s_secs = start_stamp.and_then(rfc3339_to_secs);
        let d_secs = rfc3339_to_secs(d_str);
        if let (Some(s_str), Some(s_ts), Some(d_ts)) = (start_stamp, s_secs, d_secs)
            && d_ts >= s_ts
        {
            timed.push(TimedRow {
                id,
                label: &task.label,
                started_at: s_str,
                done_at: d_str,
                dur: d_ts - s_ts,
            });
        } else {
            untimed.push((id, &task.label, d_str));
        }
    }

    if timed.is_empty() {
        lines.push("任务周转: 契约无 started_at,仅列完成时刻(不虚构时长)".into());
        if filtered {
            lines.push("  (台账任务无宿主字段,本表不随 --host 过滤)".into());
        }
        for (id, label, d_str) in untimed {
            lines.push(format!("  {id} {label} · {d_str}"));
        }
    } else {
        let mut durs: Vec<u64> = timed.iter().map(|row| row.dur).collect();
        durs.sort_unstable();
        let n = durs.len();
        let median = if n % 2 == 1 {
            durs[n / 2]
        } else {
            u64::midpoint(durs[n / 2 - 1], durs[n / 2])
        };
        let max = durs.last().copied().unwrap_or(0);
        let sum: u128 = durs.iter().copied().map(u128::from).sum();
        let count = u128::try_from(n).unwrap_or(1);
        let mean = u64::try_from(sum / count).unwrap_or(u64::MAX);
        lines.push(format!(
            "任务周转: 中位 {} · 最大 {} · 均值 {} (样本 {n})",
            fmt_duration(median),
            fmt_duration(max),
            fmt_duration(mean)
        ));
        if filtered {
            lines.push("  (台账任务无宿主字段,本表不随 --host 过滤)".into());
        }
        for row in timed {
            lines.push(format!(
                "  {} {} · {} ({} → {})",
                row.id,
                row.label,
                fmt_duration(row.dur),
                row.started_at,
                row.done_at
            ));
        }
        for (id, label, d_str) in untimed {
            lines.push(format!("  {id} {label} · {d_str} (无有效 started_at)"));
        }
    }
    if unstamped > 0 {
        lines.push(format!("  N/A(无 done_at): {unstamped}"));
    }
}

/// ① 的过滤 + 排序投影:`host` 给定时只留该宿主行(ASCII 大小写不敏感)。
fn filtered_hosts(model: &EventModel, host: Option<&str>) -> Vec<(String, u64)> {
    let mut rows: Vec<(String, u64)> = model
        .host_events
        .iter()
        .filter(|(name, _)| match host {
            Some(want) => name.eq_ignore_ascii_case(want),
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
            Some(want) => name.eq_ignore_ascii_case(want),
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
