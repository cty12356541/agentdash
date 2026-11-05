//! 波次归档引擎(W15-04):将当前活跃波次(`.agentdash/ledger.json` + 时间窗内
//! `events.jsonl.1`/`events.jsonl` 事件流)在 `.agentdash/.lock` 互斥锁保护下原子封存至
//! `.agentdash/waves/<wave>/`,产出六件套(`ledger.json`/`events.jsonl`/`digest.txt`/
//! `stats.txt`/`graph.svg`/`meta.json`)并维护自愈索引 `.agentdash/waves/index.json`。

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::contract::{self, TaskState};
use crate::events::{self, EventModel};
use crate::hook::agy_transcript_outcome;
use crate::model::{self, GateStatus, normalize_wave_id, rfc3339_to_secs, ts_now};
use crate::render::{self, display_width, render_digest, unattested_done};
use crate::stats;

/// 锁文件名(与 hook/writeback 共用 `.agentdash/.lock`)。
const LOCK_NAME: &str = ".lock";
/// 归档严判锁最长等待时长(5s 超时则失败退 1,绝不无锁写多文件)。
const LOCK_ARCHIVE_WAIT: Duration = Duration::from_secs(5);
const LOCK_SLEEP: Duration = Duration::from_millis(2);
const LOCK_STALE: Duration = Duration::from_secs(10);

/// 波次归档元数据 Schema 标识。
pub const WAVE_META_SCHEMA: &str = "agentdash.wave.v1";
/// 波次归档总览索引 Schema 标识。
pub const WAVE_INDEX_SCHEMA: &str = "agentdash.waves.index.v1";

/// `--strict` 预检策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrictPolicy {
    /// 宽松封存:允许半成品波次快照保存现场。
    Permissive,
    /// 严格门禁:要求全任务 `done`、零 `done ?`、零失败/unknown 门、零在跑子代理、零警告。
    Enforce,
}

/// 同名波次覆写策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritePolicy {
    /// 仅新建:目标波次已存在时报错退 1(默认不可变保护)。
    CreateNew,
    /// 强制覆写(`--force`):三步安全目录交换并无损合并历史事件。
    ForceOverwrite,
}

/// 活跃事件流保留策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventRetention {
    /// 保留活跃 `.agentdash/events.jsonl`(默认非破坏性封存)。
    Keep,
    /// 裁剪已归档窗口事件并移除 `events.jsonl.1`(`--prune-events`)。
    Prune,
}

/// 归档操作配置。
#[derive(Debug, Clone)]
pub struct ArchiveOptions {
    /// 显式指定的波次 ID(`--wave <id>`),覆盖 `ledger.wave`。
    pub wave_override: Option<String>,
    /// 是否执行 `--strict` 收口门禁。
    pub strict: StrictPolicy,
    /// 同名归档是否允许覆盖。
    pub write_policy: WritePolicy,
    /// 归档后是否裁剪活跃事件流。
    pub retention: EventRetention,
    /// 是否开启匿名 `completed` 推断配对(默认 `true`,`--no-infer` 为 `false`)。
    pub infer: bool,
}

/// 归档成功回执。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveReceipt {
    /// 归档波次 ID。
    pub wave: String,
    /// 已完成任务数。
    pub tasks_done: usize,
    /// 总任务数。
    pub tasks_total: usize,
    /// 切入本波次归档的事件条数。
    pub events_count: usize,
    /// 归档目录路径(`.agentdash/waves/<wave>`)。
    pub wave_dir: PathBuf,
}

/// 波次归档与查询错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaveError {
    /// 活跃 `ledger.json` 不存在。
    MissingLedger(PathBuf),
    /// 活跃 `ledger.json` 损坏。
    CorruptLedger(String),
    /// 台账未声明 `wave` 且未传 `--wave <id>`。
    MissingWaveId,
    /// 波次 ID 非法(含路径穿越、非法字符或保留名)。
    InvalidWaveId(String),
    /// 同名归档已存在且未传 `--force`。
    AlreadyExists(String),
    /// `--strict` 门禁未通过(携带具体阻塞原因列表)。
    StrictFailed(Vec<String>),
    /// `.agentdash/.lock` 获取超时。
    LockTimeout(PathBuf),
    /// 查询的历史归档波次不存在。
    WaveNotFound {
        /// 请求的波次 ID。
        wave: String,
        /// 期望路径。
        expected: PathBuf,
    },
    /// 文件系统读写失败。
    Io(String),
}

/// `meta.json` 中的时间窗摘要。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WaveWindowMeta {
    /// 窗口起始时间戳(可选)。
    pub started_at: Option<String>,
    /// 窗口结束时间戳(可选)。
    pub ended_at: Option<String>,
    /// 事件活动窗跨度(秒)。
    pub span_secs: u64,
    /// 窗口下界来源(`ledger_started_at` / `prior_wave_watermark` / `stream_full` / `empty`)。
    pub source: String,
}

/// `meta.json` 中的 Git 快照摘要。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WaveGitMeta {
    /// 分支名。
    pub branch: Option<String>,
    /// HEAD 短 SHA。
    pub head_short: Option<String>,
    /// 工作区脏文件计数。
    pub dirty: u32,
}

/// `meta.json` 中的任务计数摘要。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WaveTasksMeta {
    /// 总任务数。
    pub total: usize,
    /// 已完成数。
    pub done: usize,
    /// 进行中数(含 active/review/fix-round)。
    pub active: usize,
    /// 待开始数。
    pub pending: usize,
    /// 阻塞数。
    pub blocked: usize,
    /// 自报无物证(`done ?`)任务数。
    pub unattested_done: usize,
}

/// `meta.json` 中的单门终态条目。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WaveGateSummaryItem {
    /// 验证门名。
    pub name: String,
    /// 终态词(`running` / `passed` / `failed` / `unknown`)。
    pub status: String,
    /// 终态摘要。
    pub detail: String,
}

/// `meta.json` 中的门折叠统计条目。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WaveGateFoldItem {
    /// 验证门名。
    pub gate: String,
    /// 宿主名。
    pub host: String,
    /// 通过次数。
    pub passed: u64,
    /// 有码真失败次数。
    pub failed: u64,
    /// 退出码不可知折叠次数。
    pub unknown: u64,
}

/// `meta.json` 中的验证门汇总块。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WaveGatesMeta {
    /// 面板终态槽列表。
    pub summary: Vec<WaveGateSummaryItem>,
    /// 门折叠统计列表(供 `stats` 无 `events.jsonl` 回填)。
    pub folds: Vec<WaveGateFoldItem>,
    /// 最近一次通过门的 `ts` 原串。
    pub last_gate_passed: Option<String>,
}

/// `meta.json` 中的任务周转时长统计块。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WaveTurnoverMeta {
    /// 合法 `(started_at, done_at)` 样本数。
    pub samples: usize,
    /// 中位周转秒数。
    pub median_secs: u64,
    /// 最大周转秒数。
    pub max_secs: u64,
    /// 均值周转秒数。
    pub mean_secs: u64,
}

/// 单波次归档元数据(`waves/<id>/meta.json`)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchiveMeta {
    /// 恒为 `agentdash.wave.v1`。
    #[serde(rename = "$schema")]
    pub schema: String,
    /// 波次 ID。
    pub wave: String,
    /// 波次标题。
    pub title: String,
    /// 可选 `profile` 声明。
    pub profile: Option<String>,
    /// 归档时刻(本地 RFC 3339)。
    pub archived_at: String,
    /// 事件时间窗摘要。
    pub window: WaveWindowMeta,
    /// Git 状态摘要。
    pub git: WaveGitMeta,
    /// 任务状态汇总。
    pub tasks: WaveTasksMeta,
    /// 验证门汇总与对账锚。
    pub gates: WaveGatesMeta,
    /// 宿主生效事件计数(`host -> count`)。
    pub hosts: BTreeMap<String, u64>,
    /// 任务周转统计(无合法样本时为 `None`)。
    pub turnover: Option<WaveTurnoverMeta>,
    /// 归档事件总行数。
    pub events_count: usize,
    /// 归档 `events.jsonl` 字节数。
    pub events_bytes: u64,
    /// 归档 `events.jsonl` 64 位 FNV-1a 校验指纹。
    pub events_fnv1a64: String,
    /// 归档 `ledger.json` 64 位 FNV-1a 校验指纹。
    pub ledger_fnv1a64: String,
    /// 是否满足 `--strict` 零缺陷口径。
    pub strict_passed: bool,
    /// 契约与事件警告列表。
    pub warnings: Vec<String>,
}

/// `waves/index.json` 中的单条波次索引项。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WaveIndexEntry {
    /// 波次 ID。
    pub wave: String,
    /// 波次标题。
    pub title: String,
    /// 归档时刻。
    pub archived_at: String,
    /// 窗口起始时刻。
    pub window_start: Option<String>,
    /// 窗口结束时刻。
    pub window_end: Option<String>,
    /// Git HEAD 短 SHA。
    pub head_short: Option<String>,
    /// Git 分支。
    pub branch: Option<String>,
    /// 完成任务数。
    pub tasks_done: usize,
    /// 总任务数。
    pub tasks_total: usize,
    /// 归档事件条数。
    pub events_count: usize,
    /// 是否通过严格门禁。
    pub strict_passed: bool,
    /// 相对 `.agentdash/` 的路径(`waves/<wave>`)。
    pub path: String,
    /// 目录是否损坏降级(仅内存列表渲染用,正常条目为 `false`)。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub corrupt: bool,
}

/// `.agentdash/waves/index.json` 总览索引。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchiveIndex {
    /// 恒为 `agentdash.waves.index.v1`。
    #[serde(rename = "$schema")]
    pub schema: String,
    /// 索引最后更新时间。
    pub updated_at: String,
    /// 最新归档的波次 ID。
    pub latest: Option<String>,
    /// 按自然波次序排列的波次列表。
    pub waves: Vec<WaveIndexEntry>,
}

/// 校验并归一化波次 ID(`1..=64` 字节,首字符 `[A-Za-z0-9]`,其余 `[A-Za-z0-9._-]`,
/// 禁尾点、禁 `..`、禁 Windows 保留设备名)。
///
/// # Errors
/// 输入为空、超长、含非法字符、含路径穿越或命中 Windows 保留设备名时返回 [`WaveError::InvalidWaveId`]。
pub fn validate_wave_id(raw: &str) -> Result<String, WaveError> {
    normalize_wave_id(raw).ok_or_else(|| WaveError::InvalidWaveId(raw.to_owned()))
}

/// 自然波次排序比较器:将数字段按数值大小比较(`W1 < W2 < W9 < W10 < W14`,
/// `W-PI06 < W-PI10`),非数字段按 ASCII 大小写不敏感比较,随后优先比较是否有剩余后缀字节,
/// 最终以总字节长与原始字节序消歧(保证严格全序传递性)。
#[must_use]
pub fn natural_wave_cmp(a: &str, b: &str) -> Ordering {
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    let (mut ia, mut ib) = (0usize, 0usize);
    while ia < ab.len() && ib < bb.len() {
        if ab[ia].is_ascii_digit() && bb[ib].is_ascii_digit() {
            let start_a = ia;
            while ia < ab.len() && ab[ia].is_ascii_digit() {
                ia += 1;
            }
            let start_b = ib;
            while ib < bb.len() && bb[ib].is_ascii_digit() {
                ib += 1;
            }
            let trim_a = ab[start_a..ia]
                .iter()
                .position(|&c| c != b'0')
                .map_or(&[][..], |p| &ab[start_a + p..ia]);
            let trim_b = bb[start_b..ib]
                .iter()
                .position(|&c| c != b'0')
                .map_or(&[][..], |p| &bb[start_b + p..ib]);
            match trim_a.len().cmp(&trim_b.len()) {
                Ordering::Equal => match trim_a.cmp(trim_b) {
                    Ordering::Equal => {}
                    non_eq => return non_eq,
                },
                non_eq => return non_eq,
            }
        } else {
            let ca = ab[ia].to_ascii_uppercase();
            let cb = bb[ib].to_ascii_uppercase();
            match ca.cmp(&cb) {
                Ordering::Equal => {
                    ia += 1;
                    ib += 1;
                }
                non_eq => return non_eq,
            }
        }
    }
    (ia < ab.len())
        .cmp(&(ib < bb.len()))
        .then_with(|| ab.len().cmp(&bb.len()))
        .then_with(|| a.cmp(b))
}

/// 解析 `<repo>/.agentdash/waves/<wave_id>` 目录路径(先精确匹配,再回退大小写不敏感唯一匹配)。
///
/// # Errors
/// 波次 ID 语法非法返回 [`WaveError::InvalidWaveId`];目录不存在返回 [`WaveError::WaveNotFound`]。
pub fn resolve_wave_dir(repo: &Path, wave_id: &str) -> Result<PathBuf, WaveError> {
    let valid = validate_wave_id(wave_id)?;
    let waves_root = repo.join(".agentdash").join("waves");
    let exact = waves_root.join(&valid);
    if let Ok(entries) = fs::read_dir(&waves_root) {
        let mut ci_matches: Vec<PathBuf> = Vec::new();
        for path in entries.filter_map(Result::ok).map(|e| e.path()) {
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name == valid {
                return Ok(path);
            }
            if name.eq_ignore_ascii_case(&valid) {
                ci_matches.push(path);
            }
        }
        if ci_matches.len() == 1 {
            return Ok(ci_matches.remove(0));
        }
    }
    Err(WaveError::WaveNotFound {
        wave: valid,
        expected: exact,
    })
}

/// 64 位 FNV-1a 校验指纹(零外部依赖,输出 `0x{:016x}`)。
#[must_use]
pub fn fnv1a64(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("0x{hash:016x}")
}

/// 封存 `<repo>/.agentdash/` 当前活跃波次至 `<repo>/.agentdash/waves/<wave>/`。
///
/// # Errors
/// 台账缺失/损坏、波次 ID 缺失/非法、同名波次已存在(未传 `--force`)、`--strict` 校验未过、
/// 锁超时或磁盘读写失败时返回对应 [`WaveError`]。
#[allow(clippy::too_many_lines)]
pub fn archive_wave(repo: &Path, opts: &ArchiveOptions) -> Result<ArchiveReceipt, WaveError> {
    if let Some(ref raw_override) = opts.wave_override {
        validate_wave_id(raw_override)?;
    }
    let dash_dir = repo.join(".agentdash");
    let ledger_path = dash_dir.join("ledger.json");
    if !ledger_path.is_file() {
        return Err(WaveError::MissingLedger(ledger_path));
    }

    with_archive_lock(&dash_dir, || {
        let raw_ledger_text =
            fs::read_to_string(&ledger_path).map_err(|e| WaveError::Io(e.to_string()))?;
        let parsed_ledger = contract::parse_ledger(&raw_ledger_text)
            .map_err(|e| WaveError::CorruptLedger(e.to_string()))?;

        let raw_wave_id = opts
            .wave_override
            .as_deref()
            .or(parsed_ledger.wave.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or(WaveError::MissingWaveId)?;
        let wave_id = validate_wave_id(raw_wave_id)?;

        let waves_root = dash_dir.join("waves");
        let existing_dir = find_existing_wave_dir(&waves_root, &wave_id);
        if existing_dir.is_some() && opts.write_policy == WritePolicy::CreateNew {
            return Err(WaveError::AlreadyExists(wave_id));
        }

        // Step 0: 冲刷本回合已有明确 exit(或可从 Antigravity transcript 恢复)的 pending_gate.json 槽位
        let unresolved_pending = flush_ready_pending_gates(&dash_dir);

        // Step 1 & 2: 双代事件合并与三层波次窗口切片
        let archived_at = ts_now();
        let seal_now_secs = rfc3339_to_secs(&archived_at).unwrap_or(0);
        let merged_lines = read_and_merge_generations(&dash_dir);
        let target_upper_secs = estimate_target_upper_secs(&parsed_ledger, existing_dir.as_deref());
        let prev_wm = prior_sibling_watermark(&waves_root, &wave_id, target_upper_secs);
        let (window_start_secs, window_end_secs, window_source) =
            compute_wave_window(&parsed_ledger, prev_wm, seal_now_secs, &merged_lines);

        let mut sliced_lines = slice_events_with_open_dispatches(
            &merged_lines,
            window_start_secs,
            window_end_secs,
            prev_wm,
            opts.infer,
        );

        // Step 3: --force 重封存时按多重集保序合并旧归档事件并按时间戳稳定排序
        if let Some(ref prev_dir) = existing_dir
            && let Ok(old_ev_text) = fs::read_to_string(prev_dir.join("events.jsonl"))
        {
            sliced_lines = merge_existing_and_sliced_lines(&old_ev_text, &sliced_lines);
        }

        let events_content = if sliced_lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", sliced_lines.join("\n"))
        };

        // Step 4: 规范化归档台账文本(若补充/覆盖了 wave 字段则写回 JSON 对象)
        let archive_ledger_text = ensure_ledger_wave_field(&raw_ledger_text, &wave_id)?;

        // Step 5: 在临时目录预生成产物并构造归档 Dashboard 校验 --strict
        fs::create_dir_all(&waves_root).map_err(|e| WaveError::Io(e.to_string()))?;
        let pid = std::process::id();
        let staging_dir = waves_root.join(format!(".tmp-{wave_id}-{pid}"));
        let _ = fs::remove_dir_all(&staging_dir);
        fs::create_dir_all(&staging_dir).map_err(|e| WaveError::Io(e.to_string()))?;

        let write_res = populate_and_commit_staging(
            repo,
            &dash_dir,
            &waves_root,
            &staging_dir,
            existing_dir.as_deref(),
            &wave_id,
            &archived_at,
            &archive_ledger_text,
            &events_content,
            &sliced_lines,
            &window_source,
            unresolved_pending,
            window_end_secs,
            opts,
        );
        if write_res.is_err() {
            let _ = fs::remove_dir_all(&staging_dir);
        }
        write_res
    })
}

#[derive(Debug, Clone)]
struct IndexedEventLine {
    raw: String,
    effective_ts: u64,
    seq: usize,
    parsed: Option<Value>,
}

/// 在临时目录写入六件套、执行 `--strict` 校验、回读验证 JSON 并执行三步原子目录交换。
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn populate_and_commit_staging(
    repo: &Path,
    dash_dir: &Path,
    waves_root: &Path,
    staging_dir: &Path,
    existing_dir: Option<&Path>,
    wave_id: &str,
    archived_at: &str,
    archive_ledger_text: &str,
    events_content: &str,
    sliced_lines: &[String],
    window_source: &str,
    unresolved_pending: usize,
    window_end_secs: u64,
    opts: &ArchiveOptions,
) -> Result<ArchiveReceipt, WaveError> {
    fs::write(staging_dir.join("ledger.json"), archive_ledger_text)
        .map_err(|e| WaveError::Io(e.to_string()))?;

    let had_live_events = dash_dir.join("events.jsonl").is_file()
        || dash_dir.join("events.jsonl.1").is_file()
        || !sliced_lines.is_empty();
    if had_live_events {
        fs::write(staging_dir.join("events.jsonl"), events_content)
            .map_err(|e| WaveError::Io(e.to_string()))?;
    }

    let parsed_ledger = contract::parse_ledger(archive_ledger_text)
        .map_err(|e| WaveError::CorruptLedger(e.to_string()))?;
    let event_model = events::replay_infer(sliced_lines.iter().cloned(), opts.infer);
    let mut wave_dash = model::merge_wave_opts(repo, staging_dir, opts.infer);
    archived_at.clone_into(&mut wave_dash.generated_at);

    if !had_live_events {
        fs::write(staging_dir.join("events.jsonl"), events_content)
            .map_err(|e| WaveError::Io(e.to_string()))?;
    }

    let strict_reasons = evaluate_strict_violations(&parsed_ledger, &wave_dash, unresolved_pending);
    let strict_passed = strict_reasons.is_empty();
    if opts.strict == StrictPolicy::Enforce && !strict_passed {
        return Err(WaveError::StrictFailed(strict_reasons));
    }

    let digest_text = format!("{}\n", render_digest(&wave_dash));
    let stats_text = format!(
        "{}\n",
        stats::render(&event_model, Some(archive_ledger_text), None)
    );
    let svg_text = render::graph::render_graph_svg(&wave_dash);

    fs::write(staging_dir.join("digest.txt"), &digest_text)
        .map_err(|e| WaveError::Io(e.to_string()))?;
    fs::write(staging_dir.join("stats.txt"), &stats_text)
        .map_err(|e| WaveError::Io(e.to_string()))?;
    fs::write(staging_dir.join("graph.svg"), &svg_text)
        .map_err(|e| WaveError::Io(e.to_string()))?;

    let meta = build_archive_meta(
        wave_id,
        archived_at,
        &parsed_ledger,
        &wave_dash,
        &event_model,
        sliced_lines,
        events_content.as_bytes(),
        archive_ledger_text.as_bytes(),
        window_source,
        strict_passed,
    );
    let meta_pretty = format!(
        "{}\n",
        serde_json::to_string_pretty(&meta).map_err(|e| WaveError::Io(e.to_string()))?
    );
    fs::write(staging_dir.join("meta.json"), &meta_pretty)
        .map_err(|e| WaveError::Io(e.to_string()))?;

    let check_ledger = fs::read_to_string(staging_dir.join("ledger.json"))
        .ok()
        .and_then(|t| contract::parse_ledger(&t).ok());
    let check_meta = fs::read_to_string(staging_dir.join("meta.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<ArchiveMeta>(&t).ok());
    if check_ledger.is_none() || check_meta.is_none() {
        return Err(WaveError::Io(
            "staging verification failed for ledger.json or meta.json".to_owned(),
        ));
    }

    let final_dir = waves_root.join(wave_id);
    swap_staging_into_final(waves_root, staging_dir, existing_dir, &final_dir, wave_id)?;

    let _ = rebuild_and_write_index(waves_root, archived_at);

    if opts.retention == EventRetention::Prune {
        prune_live_events(dash_dir, window_end_secs);
    }

    Ok(ArchiveReceipt {
        wave: wave_id.to_owned(),
        tasks_done: meta.tasks.done,
        tasks_total: meta.tasks.total,
        events_count: meta.events_count,
        wave_dir: final_dir,
    })
}

/// 读取或自愈重建 `<repo>/.agentdash/waves/index.json`(遵循 `AD-ERR-001` 降级不失败)。
#[must_use]
pub fn list_waves(repo: &Path) -> ArchiveIndex {
    let waves_root = repo.join(".agentdash").join("waves");
    if !waves_root.is_dir() {
        return ArchiveIndex {
            schema: WAVE_INDEX_SCHEMA.to_owned(),
            updated_at: ts_now(),
            latest: None,
            waves: Vec::new(),
        };
    }
    let index_path = waves_root.join("index.json");
    let dir_wave_names = scan_wave_dir_names(&waves_root);
    if let Ok(raw) = fs::read_to_string(&index_path)
        && let Ok(parsed) = serde_json::from_str::<ArchiveIndex>(&raw)
        && index_matches_dirs(&parsed, &dir_wave_names)
    {
        return parsed;
    }
    rebuild_and_write_index(&waves_root, &ts_now())
}

/// 渲染 `agentdash archive --list` 的纯文本对齐表(零 ANSI,CJK 显示宽对齐)。
#[must_use]
pub fn render_wave_list(index: &ArchiveIndex, empty_msg: &str) -> String {
    if index.waves.is_empty() {
        return empty_msg.to_owned();
    }
    let pad = |s: &str, w: usize| format!("{s}{}", " ".repeat(w.saturating_sub(display_width(s))));
    let rows: Vec<(String, String, String, String, String, String)> = index
        .waves
        .iter()
        .map(|entry| {
            if entry.corrupt {
                (
                    entry.wave.clone(),
                    "-".to_owned(),
                    "-".to_owned(),
                    "⚠".to_owned(),
                    "-".to_owned(),
                    "(corrupt wave archive)".to_owned(),
                )
            } else {
                let ratio = format!("{}/{}", entry.tasks_done, entry.tasks_total);
                let evs = format!("{}ev", entry.events_count);
                let strict = if entry.strict_passed {
                    "strict✓".to_owned()
                } else {
                    "-".to_owned()
                };
                let stamp = render::clock_slice(&entry.archived_at);
                (
                    entry.wave.clone(),
                    ratio,
                    evs,
                    strict,
                    stamp,
                    entry.title.clone(),
                )
            }
        })
        .collect();

    let w_wave = rows
        .iter()
        .map(|r| display_width(&r.0))
        .max()
        .unwrap_or(4)
        .max(4);
    let w_ratio = rows
        .iter()
        .map(|r| display_width(&r.1))
        .max()
        .unwrap_or(5)
        .max(5);
    let w_ev = rows
        .iter()
        .map(|r| display_width(&r.2))
        .max()
        .unwrap_or(6)
        .max(6);
    let w_strict = rows
        .iter()
        .map(|r| display_width(&r.3))
        .max()
        .unwrap_or(6)
        .max(6);
    let w_stamp = rows
        .iter()
        .map(|r| display_width(&r.4))
        .max()
        .unwrap_or(11)
        .max(11);

    let mut out = Vec::with_capacity(rows.len() + 1);
    out.push(format!(
        "{}  {}  {}  {}  {}  title",
        pad("wave", w_wave),
        pad("tasks", w_ratio),
        pad("events", w_ev),
        pad("strict", w_strict),
        pad("archived_at", w_stamp),
    ));
    for (wave, ratio, evs, strict, stamp, title) in rows {
        out.push(format!(
            "{}  {}  {}  {}  {}  {title}",
            pad(&wave, w_wave),
            pad(&ratio, w_ratio),
            pad(&evs, w_ev),
            pad(&strict, w_strict),
            pad(&stamp, w_stamp),
        ));
    }
    out.join("\n")
}

// ---------------------------------------------------------------- 内部实现辅助

struct FileLockGuard {
    path: PathBuf,
}

impl Drop for FileLockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// 获取 `.agentdash/.lock` 排他锁(严判模式:5s 内拿不到则报错退 1,不无锁并发写)。
fn with_archive_lock<T>(
    dash_dir: &Path,
    f: impl FnOnce() -> Result<T, WaveError>,
) -> Result<T, WaveError> {
    fs::create_dir_all(dash_dir).map_err(|e| WaveError::Io(e.to_string()))?;
    let lock_path = dash_dir.join(LOCK_NAME);
    if is_stale_lock(&lock_path) {
        let _ = fs::remove_file(&lock_path);
    }
    let deadline = Instant::now() + LOCK_ARCHIVE_WAIT;
    let acquired = loop {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(_) => break true,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                if is_stale_lock(&lock_path) {
                    let _ = fs::remove_file(&lock_path);
                    continue;
                }
                if Instant::now() >= deadline {
                    break false;
                }
                std::thread::sleep(LOCK_SLEEP);
            }
            Err(_) => break false,
        }
    };
    if !acquired {
        return Err(WaveError::LockTimeout(lock_path));
    }
    let _guard = FileLockGuard { path: lock_path };
    f()
}

fn is_stale_lock(lock_path: &Path) -> bool {
    fs::metadata(lock_path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|mtime| SystemTime::now().duration_since(mtime).ok())
        .is_some_and(|age| age >= LOCK_STALE)
}

fn find_existing_wave_dir(waves_root: &Path, wave_id: &str) -> Option<PathBuf> {
    if let Ok(entries) = fs::read_dir(waves_root) {
        for path in entries.filter_map(Result::ok).map(|e| e.path()) {
            if path.is_dir()
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.eq_ignore_ascii_case(wave_id))
            {
                return Some(path);
            }
        }
    }
    None
}

/// 逆序查找 `events.jsonl` / `events.jsonl.1` 中与 `gate` 匹配的最近一条 `running` 门事件的 `host`。
fn infer_running_gate_host(dash_dir: &Path, gate: &str, session: Option<&str>) -> Option<String> {
    for name in ["events.jsonl", "events.jsonl.1"] {
        let Ok(text) = fs::read_to_string(dash_dir.join(name)) else {
            continue;
        };
        let mut fallback: Option<String> = None;
        for line in text.lines().rev() {
            let Ok(val) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            if val.get("kind").and_then(Value::as_str) != Some("gate")
                || val.get("state").and_then(Value::as_str) != Some("running")
                || val.get("gate").and_then(Value::as_str) != Some(gate)
            {
                continue;
            }
            let Some(host) = val
                .get("host")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            let line_sid = val
                .get("session_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if session.is_some() && line_sid == session {
                return Some(host.to_owned());
            }
            if fallback.is_none() {
                fallback = Some(host.to_owned());
            }
        }
        if fallback.is_some() {
            return fallback;
        }
    }
    None
}

/// 冲刷 `pending_gate.json` 中已带明确退出码(或可从 Antigravity `transcript` 补查)的槽位到 `events.jsonl`,
/// 返回剩余未决(`exit == null`)槽位数。
#[allow(clippy::too_many_lines)]
fn flush_ready_pending_gates(dash_dir: &Path) -> usize {
    let pending_path = dash_dir.join("pending_gate.json");
    let Some(raw) = fs::read_to_string(&pending_path).ok() else {
        return 0;
    };
    let slots = match serde_json::from_str::<Value>(&raw).ok() {
        Some(Value::Array(items)) => items,
        Some(obj @ Value::Object(_)) => vec![obj],
        _ => {
            let _ = fs::remove_file(&pending_path);
            return 0;
        }
    };
    let mut remaining = Vec::new();
    let mut appended_lines = Vec::new();
    for slot in slots {
        let gate = slot.get("gate").and_then(Value::as_str).unwrap_or_default();
        if gate.is_empty() {
            continue;
        }
        let mut exit = slot.get("exit").and_then(Value::as_i64);
        let mut detail = slot
            .get("detail")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if exit.is_none()
            && let Some(tp) = slot
                .get("transcript")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            && let Some((recovered_code, recovered_detail)) =
                agy_transcript_outcome(tp, slot.get("step_idx").and_then(Value::as_i64))
        {
            exit = Some(recovered_code);
            if !recovered_detail.is_empty() || detail.is_empty() {
                detail = recovered_detail;
            }
        }
        if let Some(code) = exit {
            let state = if code == 0 { "passed" } else { "failed" };
            let session = slot
                .get("session")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let host = slot
                .get("host")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .or_else(|| infer_running_gate_host(dash_dir, gate, session));
            let mut ev = json!({
                "ts": ts_now(),
                "kind": "gate",
                "gate": gate,
                "state": state,
                "exit": code,
                "detail": detail,
            });
            if let Some(h) = host {
                ev["host"] = Value::String(h);
            }
            if let Some(sid) = session {
                ev["session_id"] = Value::String(sid.to_owned());
            }
            appended_lines.push(ev.to_string());
        } else {
            remaining.push(slot);
        }
    }
    let mut write_ok = true;
    if !appended_lines.is_empty() {
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dash_dir.join("events.jsonl"))
        {
            for line in appended_lines {
                if writeln!(f, "{line}").is_err() {
                    write_ok = false;
                }
            }
            if f.flush().is_err() {
                write_ok = false;
            }
        } else {
            write_ok = false;
        }
    }
    if write_ok {
        if remaining.is_empty() {
            let _ = fs::remove_file(&pending_path);
        } else if let Ok(text) = serde_json::to_string(&remaining) {
            let _ = fs::write(&pending_path, text);
        }
    }
    remaining.len()
}

/// 合并读取 `events.jsonl.1` 与 `events.jsonl`,仅消除 `.1` 尾部与当前文件头部的精确前后缀重叠(上限 64 行),
/// 保留所有合法同秒相同事件行,并按 `(effective_ts, seq)` 稳定排序。
fn read_and_merge_generations(dash_dir: &Path) -> Vec<IndexedEventLine> {
    let dot1_text = fs::read_to_string(dash_dir.join("events.jsonl.1")).unwrap_or_default();
    let curr_text = fs::read_to_string(dash_dir.join("events.jsonl")).unwrap_or_default();

    let dot1_lines: Vec<&str> = dot1_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let curr_lines: Vec<&str> = curr_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();

    let max_k = 64.min(dot1_lines.len()).min(curr_lines.len());
    let overlap = (1..=max_k)
        .rev()
        .find(|&k| dot1_lines[dot1_lines.len() - k..] == curr_lines[..k])
        .unwrap_or(0);

    let mut last_seen_ts = 0u64;
    let mut indexed: Vec<IndexedEventLine> = dot1_lines
        .into_iter()
        .chain(curr_lines.into_iter().skip(overlap))
        .enumerate()
        .map(|(seq, raw)| {
            let parsed = serde_json::from_str::<Value>(raw).ok();
            let ts_secs = parsed
                .as_ref()
                .and_then(|v| v.get("ts"))
                .and_then(Value::as_str)
                .map(str::trim)
                .and_then(rfc3339_to_secs);
            if let Some(secs) = ts_secs {
                last_seen_ts = secs;
            }
            IndexedEventLine {
                raw: raw.to_owned(),
                effective_ts: ts_secs.unwrap_or(last_seen_ts),
                seq,
                parsed,
            }
        })
        .collect();

    indexed.sort_by_key(|item| (item.effective_ts, item.seq));
    indexed
}

/// 估计当前封存波次自身的时间上界(用于过滤乱序重封存时的未来波次水线)。
fn estimate_target_upper_secs(
    ledger: &contract::Ledger,
    existing_dir: Option<&Path>,
) -> Option<u64> {
    let task_max = ledger
        .tasks
        .values()
        .flat_map(|t| [t.started_at.as_deref(), t.done_at.as_deref()])
        .flatten()
        .map(str::trim)
        .filter_map(rfc3339_to_secs)
        .max();
    if task_max.is_some() {
        return task_max;
    }
    let dir = existing_dir?;
    let raw = fs::read_to_string(dir.join("meta.json")).ok()?;
    let meta = serde_json::from_str::<ArchiveMeta>(&raw).ok()?;
    meta.window
        .ended_at
        .as_deref()
        .map(str::trim)
        .and_then(rfc3339_to_secs)
        .or_else(|| rfc3339_to_secs(meta.archived_at.trim()))
}

/// 扫描前序已归档兄弟波次的最高结束水线(秒),排除 `target_wave_id` 自身及自然序在其后的未来波次。
fn prior_sibling_watermark(
    waves_root: &Path,
    target_wave_id: &str,
    target_upper_secs: Option<u64>,
) -> Option<u64> {
    let entries = fs::read_dir(waves_root).ok()?;
    let mut max_wm: Option<u64> = None;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.')
            || name.eq_ignore_ascii_case(target_wave_id)
            || natural_wave_cmp(name, target_wave_id) != Ordering::Less
        {
            continue;
        }
        let wm = fs::read_to_string(path.join("meta.json"))
            .ok()
            .and_then(|raw| serde_json::from_str::<ArchiveMeta>(&raw).ok())
            .and_then(|meta| {
                meta.window
                    .ended_at
                    .as_deref()
                    .map(str::trim)
                    .and_then(rfc3339_to_secs)
                    .or_else(|| rfc3339_to_secs(meta.archived_at.trim()))
            });
        if let Some(secs) = wm
            && target_upper_secs.is_none_or(|upper| secs <= upper)
        {
            max_wm = Some(max_wm.map_or(secs, |cur| cur.max(secs)));
        }
    }
    max_wm
}

/// 计算三层波次时间窗 `(window_start_secs, window_end_secs, source)`。
fn compute_wave_window(
    ledger: &contract::Ledger,
    prev_watermark: Option<u64>,
    seal_now_secs: u64,
    merged_lines: &[IndexedEventLine],
) -> (Option<u64>, u64, String) {
    let started_secs: Vec<u64> = ledger
        .tasks
        .values()
        .filter_map(|t| t.started_at.as_deref())
        .map(str::trim)
        .filter_map(rfc3339_to_secs)
        .collect();
    let done_secs: Vec<u64> = ledger
        .tasks
        .values()
        .filter_map(|t| t.done_at.as_deref())
        .map(str::trim)
        .filter_map(rfc3339_to_secs)
        .collect();

    let min_started = started_secs.iter().copied().min();
    let min_done = done_secs.iter().copied().min();
    let max_started = started_secs.iter().copied().max();
    let max_done = done_secs.iter().copied().max();
    let max_event = merged_lines.iter().map(|l| l.effective_ts).max();

    let window_end_secs = [Some(seal_now_secs), max_done, max_started, max_event]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(0);

    let (window_start_secs, source) = if let Some(min_s) = min_started
        && min_done.is_none_or(|d| d >= min_s)
    {
        let grace = min_s.saturating_sub(60);
        let start = match prev_watermark {
            Some(wm) => grace.max(wm.saturating_add(1)).min(min_s),
            None => grace,
        };
        (Some(start), "ledger_started_at".to_owned())
    } else if let Some(wm) = prev_watermark {
        let wm_next = wm.saturating_add(1);
        let earliest_task = [min_started, min_done].into_iter().flatten().min();
        let start = earliest_task.map_or(wm_next, |t| wm_next.min(t));
        (Some(start), "prior_wave_watermark".to_owned())
    } else if merged_lines.is_empty() {
        (None, "empty".to_owned())
    } else {
        (None, "stream_full".to_owned())
    };

    (window_start_secs, window_end_secs, source)
}

#[derive(Debug, Clone)]
struct PreDispatch {
    who: String,
    raw: String,
    inferred: bool,
}

/// 按时间窗过滤事件行,并仅前插在 `window_start_secs` 前已 `dispatched` 且在窗口内闭合的子代理事件。
#[allow(clippy::too_many_lines)]
fn slice_events_with_open_dispatches(
    merged_lines: &[IndexedEventLine],
    window_start_secs: Option<u64>,
    window_end_secs: u64,
    prev_watermark: Option<u64>,
    infer: bool,
) -> Vec<String> {
    let Some(start_secs) = window_start_secs else {
        return merged_lines
            .iter()
            .filter(|l| l.effective_ts <= window_end_secs)
            .map(|l| l.raw.clone())
            .collect();
    };

    let lower_guard = prev_watermark.map_or(0, |wm| wm.saturating_add(1));
    let mut pre_dispatches: Vec<PreDispatch> = Vec::new();
    let mut in_window_items: Vec<&IndexedEventLine> = Vec::new();

    for item in merged_lines {
        if item.effective_ts < start_secs {
            if item.effective_ts >= lower_guard
                && let Some(ref val) = item.parsed
                && val.get("kind").and_then(Value::as_str) == Some("agent")
            {
                match val.get("event").and_then(Value::as_str) {
                    Some("dispatched") => {
                        if let Some(who) = val
                            .get("who")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                        {
                            if let Some(existing) = pre_dispatches.iter_mut().find(|d| d.who == who)
                            {
                                existing.inferred = false;
                            } else {
                                pre_dispatches.push(PreDispatch {
                                    who: who.to_owned(),
                                    raw: item.raw.clone(),
                                    inferred: false,
                                });
                            }
                        }
                    }
                    Some("completed") => {
                        if let Some(who) = val
                            .get("who")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                        {
                            pre_dispatches.retain(|d| d.who != who);
                        } else if infer
                            && let Some(slot) = pre_dispatches.iter_mut().find(|d| !d.inferred)
                        {
                            slot.inferred = true;
                        }
                    }
                    _ => {}
                }
            }
        } else if item.effective_ts <= window_end_secs {
            in_window_items.push(item);
        }
    }

    let active_pre: Vec<PreDispatch> = pre_dispatches.into_iter().filter(|d| !d.inferred).collect();
    if active_pre.is_empty() {
        return in_window_items.into_iter().map(|i| i.raw.clone()).collect();
    }

    // 模拟窗口内 agent 事件,仅前插在窗口内真正被 completed 闭合的前序派发
    let mut needed_who: HashSet<String> = HashSet::new();
    let mut sim_open: Vec<(String, bool)> =
        active_pre.iter().map(|d| (d.who.clone(), true)).collect();
    for item in &in_window_items {
        let Some(ref val) = item.parsed else {
            continue;
        };
        if val.get("kind").and_then(Value::as_str) != Some("agent") {
            continue;
        }
        match val.get("event").and_then(Value::as_str) {
            Some("dispatched") => {
                if let Some(who) = val
                    .get("who")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    && !sim_open.iter().any(|(w, _)| w == who)
                {
                    sim_open.push((who.to_owned(), false));
                }
            }
            Some("completed") => {
                if let Some(who) = val
                    .get("who")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    if let Some(pos) = sim_open.iter().position(|(w, _)| w == who) {
                        let (w, from_pre) = sim_open.remove(pos);
                        if from_pre {
                            needed_who.insert(w);
                        }
                    }
                } else if infer && !sim_open.is_empty() {
                    let (w, from_pre) = sim_open.remove(0);
                    if from_pre {
                        needed_who.insert(w);
                    }
                }
            }
            _ => {}
        }
    }

    let mut out = Vec::with_capacity(needed_who.len() + in_window_items.len());
    for d in active_pre {
        if needed_who.contains(&d.who) {
            out.push(d.raw);
        }
    }
    out.extend(in_window_items.into_iter().map(|i| i.raw.clone()));
    out
}

/// `--force` 重封存时按多重集保序合并旧归档事件与新切片事件,并按 `(effective_ts, seq)` 稳定排序。
fn merge_existing_and_sliced_lines(old_text: &str, sliced: &[String]) -> Vec<String> {
    let old_lines: Vec<&str> = old_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let mut old_counts: HashMap<&str, usize> = HashMap::new();
    for &line in &old_lines {
        *old_counts.entry(line).or_insert(0) += 1;
    }

    let mut combined: Vec<String> = old_lines.iter().map(|&s| s.to_owned()).collect();
    for line in sliced {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(count) = old_counts.get_mut(trimmed)
            && *count > 0
        {
            *count -= 1;
            continue;
        }
        combined.push(trimmed.to_owned());
    }

    let mut last_seen_ts = 0u64;
    let mut indexed: Vec<(u64, usize, String)> = combined
        .into_iter()
        .enumerate()
        .map(|(seq, raw)| {
            let ts_secs = serde_json::from_str::<Value>(&raw).ok().and_then(|v| {
                v.get("ts")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .and_then(rfc3339_to_secs)
            });
            if let Some(secs) = ts_secs {
                last_seen_ts = secs;
            }
            (ts_secs.unwrap_or(last_seen_ts), seq, raw)
        })
        .collect();
    indexed.sort_by_key(|(ts, seq, _)| (*ts, *seq));
    indexed.into_iter().map(|(_, _, raw)| raw).collect()
}

/// 确保归档的 `ledger.json` 含顶层 `"wave": "<wave_id>"`(原已一致则保留原串字节)。
fn ensure_ledger_wave_field(raw_text: &str, wave_id: &str) -> Result<String, WaveError> {
    let mut val: Value =
        serde_json::from_str(raw_text).map_err(|e| WaveError::CorruptLedger(e.to_string()))?;
    let Some(obj) = val.as_object_mut() else {
        return Err(WaveError::CorruptLedger(
            "top-level JSON is not an object".to_owned(),
        ));
    };
    if obj.get("wave").and_then(Value::as_str) == Some(wave_id) {
        return Ok(raw_text.to_owned());
    }
    obj.insert("wave".to_owned(), Value::String(wave_id.to_owned()));
    let pretty = serde_json::to_string_pretty(&val).map_err(|e| WaveError::Io(e.to_string()))?;
    Ok(format!("{pretty}\n"))
}

/// 检查 `--strict` 归档门禁违规项(空列表表示全通过)。
fn evaluate_strict_violations(
    ledger: &contract::Ledger,
    dash: &model::Dashboard,
    unresolved_pending: usize,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if ledger.tasks.is_empty() {
        reasons.push("ledger.json has zero tasks".to_owned());
    }
    let incomplete: Vec<&String> = ledger
        .tasks
        .iter()
        .filter(|(_, t)| t.state != TaskState::Done)
        .map(|(id, _)| id)
        .collect();
    if !incomplete.is_empty() {
        let mut ids: Vec<&str> = incomplete.into_iter().map(String::as_str).collect();
        ids.sort_unstable();
        reasons.push(format!("incomplete tasks: {}", ids.join(", ")));
    }
    if unresolved_pending > 0 {
        reasons.push(format!(
            "{unresolved_pending} in-flight pending gate(s) with unknown exit"
        ));
    }
    for gate in &dash.gates {
        match &gate.status {
            GateStatus::Failed { detail } => {
                reasons.push(format!("gate `{}` failed ({detail})", gate.name));
            }
            GateStatus::Unknown { detail } => {
                reasons.push(format!("gate `{}` exit unknown ({detail})", gate.name));
            }
            GateStatus::Running => {
                reasons.push(format!("gate `{}` is still running", gate.name));
            }
            GateStatus::Passed { .. } => {}
        }
    }
    if dash.running_agents() > 0 {
        reasons.push(format!(
            "{} running subagent(s) unclosed",
            dash.running_agents()
        ));
    }
    let suspect: Vec<&str> = dash
        .tasks
        .iter()
        .filter(|t| unattested_done(t, dash))
        .map(|t| t.id.as_str())
        .collect();
    if !suspect.is_empty() {
        reasons.push(format!(
            "unattested done tasks (done_at > last_gate_passed): {}",
            suspect.join(", ")
        ));
    }
    for w in &dash.warnings {
        reasons.push(format!("warning: {w}"));
    }
    reasons
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn build_archive_meta(
    wave_id: &str,
    archived_at: &str,
    ledger: &contract::Ledger,
    dash: &model::Dashboard,
    event_model: &EventModel,
    sliced_lines: &[String],
    events_bytes: &[u8],
    ledger_bytes: &[u8],
    window_source: &str,
    strict_passed: bool,
) -> ArchiveMeta {
    let mut first_ts: Option<String> = None;
    let mut last_ts: Option<String> = None;
    for raw in sliced_lines {
        if let Ok(val) = serde_json::from_str::<Value>(raw)
            && let Some(ts) = val
                .get("ts")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| rfc3339_to_secs(s).is_some())
        {
            if first_ts.is_none() {
                first_ts = Some(ts.to_owned());
            }
            last_ts = Some(ts.to_owned());
        }
    }
    let mut task_starts: Vec<&str> = ledger
        .tasks
        .values()
        .filter_map(|t| t.started_at.as_deref())
        .map(str::trim)
        .filter(|s| rfc3339_to_secs(s).is_some())
        .collect();
    task_starts.sort_by_key(|s| rfc3339_to_secs(s).unwrap_or(0));
    let mut task_dones: Vec<&str> = ledger
        .tasks
        .values()
        .filter_map(|t| t.done_at.as_deref())
        .map(str::trim)
        .filter(|s| rfc3339_to_secs(s).is_some())
        .collect();
    task_dones.sort_by_key(|s| rfc3339_to_secs(s).unwrap_or(0));

    let window_started = first_ts.or_else(|| task_starts.first().copied().map(str::to_owned));
    let window_ended = [last_ts.as_deref(), task_dones.last().copied()]
        .into_iter()
        .flatten()
        .max_by_key(|s| rfc3339_to_secs(s).unwrap_or(0))
        .map(str::to_owned)
        .or_else(|| Some(archived_at.to_owned()));

    let head_short = dash.git.head_short.clone().or_else(|| {
        dash.git
            .recent
            .first()
            .and_then(|line| line.split_whitespace().next())
            .map(str::to_owned)
    });

    let (mut done, mut active, mut pending, mut blocked) = (0usize, 0usize, 0usize, 0usize);
    for spec in ledger.tasks.values() {
        match spec.state {
            TaskState::Done => done += 1,
            TaskState::Active | TaskState::Review | TaskState::FixRound => active += 1,
            TaskState::Pending => pending += 1,
            TaskState::Blocked => blocked += 1,
        }
    }
    let unattested = dash
        .tasks
        .iter()
        .filter(|t| unattested_done(t, dash))
        .count();

    let summary: Vec<WaveGateSummaryItem> = dash
        .gates
        .iter()
        .map(|g| WaveGateSummaryItem {
            name: g.name.clone(),
            status: g.status.as_label().to_owned(),
            detail: g.status.detail().to_owned(),
        })
        .collect();

    let mut folds: Vec<WaveGateFoldItem> = event_model
        .gate_folds
        .iter()
        .map(|((gate, host), tally)| WaveGateFoldItem {
            gate: gate.clone(),
            host: host.clone(),
            passed: tally.passed,
            failed: tally.failed,
            unknown: tally.unknown,
        })
        .collect();
    folds.sort_by(|a, b| a.gate.cmp(&b.gate).then_with(|| a.host.cmp(&b.host)));

    let hosts: BTreeMap<String, u64> = event_model
        .host_events
        .iter()
        .map(|(k, v)| (k.clone(), *v))
        .collect();

    let turnover = compute_turnover_meta(ledger);

    ArchiveMeta {
        schema: WAVE_META_SCHEMA.to_owned(),
        wave: wave_id.to_owned(),
        title: ledger.title.clone(),
        profile: ledger.profile.clone(),
        archived_at: archived_at.to_owned(),
        window: WaveWindowMeta {
            started_at: window_started,
            ended_at: window_ended,
            span_secs: dash.event_span_secs.unwrap_or(0),
            source: window_source.to_owned(),
        },
        git: WaveGitMeta {
            branch: dash.git.branch.clone(),
            head_short,
            dirty: dash.git.dirty,
        },
        tasks: WaveTasksMeta {
            total: ledger.tasks.len(),
            done,
            active,
            pending,
            blocked,
            unattested_done: unattested,
        },
        gates: WaveGatesMeta {
            summary,
            folds,
            last_gate_passed: dash.last_gate_passed.clone(),
        },
        hosts,
        turnover,
        events_count: sliced_lines.len(),
        events_bytes: u64::try_from(events_bytes.len()).unwrap_or(u64::MAX),
        events_fnv1a64: fnv1a64(events_bytes),
        ledger_fnv1a64: fnv1a64(ledger_bytes),
        strict_passed,
        warnings: dash.warnings.clone(),
    }
}

fn compute_turnover_meta(ledger: &contract::Ledger) -> Option<WaveTurnoverMeta> {
    let mut durs: Vec<u64> = ledger
        .tasks
        .values()
        .filter(|t| t.state == TaskState::Done)
        .filter_map(|t| {
            let s = rfc3339_to_secs(t.started_at.as_deref()?.trim())?;
            let d = rfc3339_to_secs(t.done_at.as_deref()?.trim())?;
            (d >= s).then_some(d - s)
        })
        .collect();
    if durs.is_empty() {
        return None;
    }
    durs.sort_unstable();
    let n = durs.len();
    let median_secs = if n % 2 == 1 {
        durs[n / 2]
    } else {
        u64::midpoint(durs[n / 2 - 1], durs[n / 2])
    };
    let max_secs = durs.last().copied().unwrap_or(0);
    let sum: u128 = durs.iter().copied().map(u128::from).sum();
    let count = u128::try_from(n).unwrap_or(1);
    let mean_secs = u64::try_from(sum / count).unwrap_or(u64::MAX);
    Some(WaveTurnoverMeta {
        samples: n,
        median_secs,
        max_secs,
        mean_secs,
    })
}

/// 三步回滚安全目录交换(兼容 Windows 非空目录不可直接 `rename` 覆盖的限制)。
fn swap_staging_into_final(
    waves_root: &Path,
    staging_dir: &Path,
    existing_dir: Option<&Path>,
    final_dir: &Path,
    wave_id: &str,
) -> Result<(), WaveError> {
    if let Some(old_dir) = existing_dir {
        let backup_dir = waves_root.join(format!(".old-{wave_id}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&backup_dir);
        fs::rename(old_dir, &backup_dir).map_err(|e| WaveError::Io(e.to_string()))?;
        if let Err(err) = fs::rename(staging_dir, final_dir) {
            let _ = fs::rename(&backup_dir, old_dir);
            return Err(WaveError::Io(err.to_string()));
        }
        let _ = fs::remove_dir_all(&backup_dir);
    } else {
        fs::rename(staging_dir, final_dir).map_err(|e| WaveError::Io(e.to_string()))?;
    }
    Ok(())
}

fn scan_wave_dir_names(waves_root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(waves_root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort_by(|a, b| natural_wave_cmp(a, b));
    names
}

fn index_matches_dirs(index: &ArchiveIndex, dir_names: &[String]) -> bool {
    if index.schema != WAVE_INDEX_SCHEMA || index.waves.len() != dir_names.len() {
        return false;
    }
    index
        .waves
        .iter()
        .zip(dir_names.iter())
        .all(|(entry, dir)| entry.wave == *dir && !entry.corrupt)
}

/// 扫描 `.agentdash/waves/*/` 重建 `ArchiveIndex` 并原子写回 `waves/index.json`。
fn rebuild_and_write_index(waves_root: &Path, updated_at: &str) -> ArchiveIndex {
    let dir_names = scan_wave_dir_names(waves_root);
    let mut waves = Vec::with_capacity(dir_names.len());
    for name in dir_names {
        let dir = waves_root.join(&name);
        if let Ok(raw_meta) = fs::read_to_string(dir.join("meta.json"))
            && let Ok(meta) = serde_json::from_str::<ArchiveMeta>(&raw_meta)
        {
            waves.push(WaveIndexEntry {
                wave: meta.wave.clone(),
                title: meta.title,
                archived_at: meta.archived_at,
                window_start: meta.window.started_at,
                window_end: meta.window.ended_at,
                head_short: meta.git.head_short,
                branch: meta.git.branch,
                tasks_done: meta.tasks.done,
                tasks_total: meta.tasks.total,
                events_count: meta.events_count,
                strict_passed: meta.strict_passed,
                path: format!("waves/{name}"),
                corrupt: false,
            });
            continue;
        }
        if let Ok(raw_ledger) = fs::read_to_string(dir.join("ledger.json"))
            && let Ok(ledger) = contract::parse_ledger(&raw_ledger)
        {
            let done = ledger
                .tasks
                .values()
                .filter(|t| t.state == TaskState::Done)
                .count();
            let ev_count = fs::read_to_string(dir.join("events.jsonl"))
                .map_or(0, |t| t.lines().filter(|l| !l.trim().is_empty()).count());
            waves.push(WaveIndexEntry {
                wave: name.clone(),
                title: ledger.title,
                archived_at: updated_at.to_owned(),
                window_start: None,
                window_end: None,
                head_short: None,
                branch: None,
                tasks_done: done,
                tasks_total: ledger.tasks.len(),
                events_count: ev_count,
                strict_passed: false,
                path: format!("waves/{name}"),
                corrupt: false,
            });
            continue;
        }
        waves.push(WaveIndexEntry {
            wave: name.clone(),
            title: "(corrupt wave archive)".to_owned(),
            archived_at: String::new(),
            window_start: None,
            window_end: None,
            head_short: None,
            branch: None,
            tasks_done: 0,
            tasks_total: 0,
            events_count: 0,
            strict_passed: false,
            path: format!("waves/{name}"),
            corrupt: true,
        });
    }

    waves.sort_by(|a, b| natural_wave_cmp(&a.wave, &b.wave));
    let latest = waves
        .iter()
        .filter(|w| !w.corrupt)
        .max_by(|a, b| {
            let ta = rfc3339_to_secs(&a.archived_at).unwrap_or(0);
            let tb = rfc3339_to_secs(&b.archived_at).unwrap_or(0);
            ta.cmp(&tb).then_with(|| natural_wave_cmp(&a.wave, &b.wave))
        })
        .map(|w| w.wave.clone());

    let index = ArchiveIndex {
        schema: WAVE_INDEX_SCHEMA.to_owned(),
        updated_at: updated_at.to_owned(),
        latest,
        waves,
    };

    if let Ok(pretty) = serde_json::to_string_pretty(&index) {
        let tmp = waves_root.join(".tmp-index.json");
        let dst = waves_root.join("index.json");
        if fs::write(&tmp, format!("{pretty}\n")).is_ok() && fs::rename(&tmp, &dst).is_err() {
            let _ = fs::remove_file(&dst);
            let _ = fs::rename(&tmp, &dst);
        }
    }

    index
}

/// 裁剪活跃 `.agentdash/events.jsonl`(保留 `ts > window_end_secs` 的事件及尚未闭合的在跑子代理/门)并删除 `events.jsonl.1`。
#[allow(clippy::too_many_lines)]
fn prune_live_events(dash_dir: &Path, window_end_secs: u64) {
    let _ = fs::remove_file(dash_dir.join("events.jsonl.1"));
    let ev_path = dash_dir.join("events.jsonl");
    let Ok(raw) = fs::read_to_string(&ev_path) else {
        return;
    };
    let unresolved_gates: HashSet<String> = fs::read_to_string(dash_dir.join("pending_gate.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| s.get("gate").and_then(Value::as_str).map(str::to_owned))
        .collect();

    let mut open_agents: Vec<(String, String)> = Vec::new();
    let mut open_gates: HashMap<String, String> = HashMap::new();
    let mut post_window: Vec<String> = Vec::new();
    let mut last_seen_ts = 0u64;

    for line in raw.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let val = serde_json::from_str::<Value>(line).ok();
        let ts_secs = val.as_ref().and_then(|v| {
            v.get("ts")
                .and_then(Value::as_str)
                .map(str::trim)
                .and_then(rfc3339_to_secs)
        });
        if let Some(secs) = ts_secs {
            last_seen_ts = secs;
        }
        let eff_ts = ts_secs.unwrap_or(last_seen_ts);
        if eff_ts > window_end_secs {
            post_window.push(line.to_owned());
            continue;
        }
        if let Some(ref v) = val {
            match v.get("kind").and_then(Value::as_str) {
                Some("agent") => match v.get("event").and_then(Value::as_str) {
                    Some("dispatched") => {
                        if let Some(who) = v
                            .get("who")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                        {
                            if let Some(ex) = open_agents.iter_mut().find(|(w, _)| w == who) {
                                line.clone_into(&mut ex.1);
                            } else {
                                open_agents.push((who.to_owned(), line.to_owned()));
                            }
                        }
                    }
                    Some("completed") => {
                        if let Some(who) = v
                            .get("who")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                        {
                            open_agents.retain(|(w, _)| w != who);
                        } else if !open_agents.is_empty() {
                            open_agents.remove(0);
                        }
                    }
                    _ => {}
                },
                Some("gate") => {
                    if let Some(gate) = v
                        .get("gate")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        if v.get("state").and_then(Value::as_str) == Some("running") {
                            if unresolved_gates.contains(gate) {
                                open_gates.insert(gate.to_owned(), line.to_owned());
                            }
                        } else {
                            open_gates.remove(gate);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let mut kept: Vec<String> = open_agents.into_iter().map(|(_, l)| l).collect();
    let mut gate_lines: Vec<(String, String)> = open_gates.into_iter().collect();
    gate_lines.sort_by(|a, b| a.0.cmp(&b.0));
    kept.extend(gate_lines.into_iter().map(|(_, l)| l));
    kept.extend(post_window);

    let new_content = if kept.is_empty() {
        String::new()
    } else {
        format!("{}\n", kept.join("\n"))
    };
    let tmp = dash_dir.join("events.jsonl.tmp");
    if fs::write(&tmp, new_content).is_ok() && fs::rename(&tmp, &ev_path).is_err() {
        let _ = fs::remove_file(&ev_path);
        let _ = fs::rename(&tmp, &ev_path);
    }
}
