//! 远程源观察者(W2-007 / T7):对仓库目录做只读 PR 探测,采集 PR 号、标题
//! 与 checks 状态(`gh pr view --json` + `gh pr checks --json`)。
//!
//! 约定:目录无 `.git`、gh 不可用、命令非零退出(无 PR 等)或单命令超时,
//! 一律降级为 [`None`]——远程是增强不是依赖,绝不 panic、绝不拖住渲染。
//!
//! 缓存:命中写 `<repo>/.agentdash/cache/gh.json`,TTL 120s(mtime 距今小于
//! TTL 直接读缓存,不发起子进程);写缓存经临时文件 + `rename` 原子替换,
//! 损坏缓存按未命中处理照走实时探测并在成功后覆写。
//!
//! 测试面:探测命令经 [`fetch_with`] 注入(命令执行器 `Fn(&[&str]) ->
//! Option<String>`),真 gh 不进单测;[`fetch`] 恒用真实执行器。

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

/// 仓库内缓存落点(相对仓库根):`<repo>/.agentdash/cache/gh.json`。
const CACHE_DIR: &str = ".agentdash";
const CACHE_SUBDIR: &str = "cache";
const CACHE_NAME: &str = "gh.json";
/// 缓存新鲜度阈值:mtime 距今小于该值直接读缓存,不发起子进程。
const CACHE_TTL: Duration = Duration::from_mins(2);
/// 单条 gh 命令的超时上限;超时按"不可用"处理,不拖住仪表盘。
const GH_TIMEOUT: Duration = Duration::from_secs(5);

/// 一次远程探测的全部观测结果。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::module_name_repetitions)] // 名称由 W2-007 接口契约固定
pub struct RemoteFacts {
    /// 当前分支关联的 PR 号(`gh pr view --json number`)。
    pub pr_number: u64,
    /// PR 标题(`gh pr view --json title`)。
    pub pr_title: String,
    /// checks 列表 `(name, state)`,原样透传 gh 词表(如 `SUCCESS` / `FAILURE`
    /// / `PENDING`),不做翻译;`gh pr checks` 失败时为空(PR 存在但无 checks)。
    pub checks: Vec<(String, String)>,
}

/// 对 `repo` 目录做一次只读远程探测(真实 gh 执行器)。任何失败降级 [`None`]。
#[must_use]
pub fn fetch(repo: &Path) -> Option<RemoteFacts> {
    fetch_with(repo, |args| run_gh(repo, args))
}

/// [`fetch`] 的可注入变体:`run` 接收 gh 子命令参数(如 `["pr", "view", ...]`),
/// 返回 UTF-8 stdout;`None` 表示命令失败(gh 缺失 / 非零退出 / 超时)。
/// 缓存语义与 [`fetch`] 完全一致。
pub fn fetch_with(repo: &Path, run: impl Fn(&[&str]) -> Option<String>) -> Option<RemoteFacts> {
    // gh 依赖 git 仓做 PR 关联:非 git 目录直接不探测
    if !repo.join(".git").exists() {
        return None;
    }
    let cache = cache_path(repo);
    if let Some(facts) = read_fresh_cache(&cache) {
        return Some(facts);
    }
    let facts = probe(&run)?;
    write_cache(&cache, &facts);
    Some(facts)
}

/// 缓存落点:`<repo>/.agentdash/cache/gh.json`。
fn cache_path(repo: &Path) -> std::path::PathBuf {
    repo.join(CACHE_DIR).join(CACHE_SUBDIR).join(CACHE_NAME)
}

/// 读新鲜缓存:文件存在、mtime 距今小于 [`CACHE_TTL`] 且内容可解析为
/// [`RemoteFacts`] 才命中;损坏缓存(mtime 新鲜但解析失败)按未命中处理,
/// 交回实时探测。其余路径(缺文件 / 取不到 mtime / 时钟偏差)一律 [`None`]。
fn read_fresh_cache(path: &Path) -> Option<RemoteFacts> {
    let meta = fs::metadata(path).ok()?;
    let age = SystemTime::now()
        .duration_since(meta.modified().ok()?)
        .ok()?;
    if age >= CACHE_TTL {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// 写缓存:临时文件同目录落盘后 `rename` 原子替换(Windows 侧 std 映射
/// `MoveFileEx(REPLACE_EXISTING)`,读者要么见旧文件要么见新文件)。任何
/// IO 失败静默放弃——缓存缺失只损失下次命中,不影响本次返回。
fn write_cache(path: &Path, facts: &RemoteFacts) {
    let Some(parent) = path.parent() else { return };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let tmp = parent.join(format!("{}.tmp.{}", CACHE_NAME, std::process::id()));
    let Ok(mut file) = File::create(&tmp) else {
        return;
    };
    let Ok(text) = serde_json::to_string(facts) else {
        let _ = fs::remove_file(&tmp);
        return;
    };
    if file.write_all(text.as_bytes()).is_err() || file.flush().is_err() {
        let _ = fs::remove_file(&tmp);
        return;
    }
    drop(file);
    if fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

/// 实时探测:`gh pr view --json number,title` 必须成功(无 PR / gh 缺失 /
/// 超时 → [`None`]);`gh pr checks --json name,state` 失败只损失 checks
/// (PR 存在但尚无 checks 是常态,不算探测失败)。
fn probe(run: &impl Fn(&[&str]) -> Option<String>) -> Option<RemoteFacts> {
    let view = run(&["pr", "view", "--json", "number,title"])?;
    let value: serde_json::Value = serde_json::from_str(&view).ok()?;
    // 字段类型不符(数字缺失/标题非串)按损坏输出处理 → None
    let pr_number = value.get("number")?.as_u64()?;
    let pr_title = value.get("title")?.as_str()?.to_owned();
    let checks = run(&["pr", "checks", "--json", "name,state"])
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .map_or_else(Vec::new, parse_checks);
    Some(RemoteFacts {
        pr_number,
        pr_title,
        checks,
    })
}

/// 解析 `gh pr checks --json` 输出:兼容对象包裹(`{"checks":[...]}`)与
/// 顶层裸数组两种形态;单条目缺 `name`/`state` 字段跳过,绝不上抛。
fn parse_checks(value: serde_json::Value) -> Vec<(String, String)> {
    let entries = match value {
        serde_json::Value::Array(entries) => entries,
        serde_json::Value::Object(obj) => match obj.get("checks") {
            Some(serde_json::Value::Array(entries)) => entries.clone(),
            _ => return Vec::new(),
        },
        _ => return Vec::new(),
    };
    entries
        .into_iter()
        .filter_map(|entry| {
            let name = entry.get("name")?.as_str()?.to_owned();
            let state = entry.get("state")?.as_str()?.to_owned();
            Some((name, state))
        })
        .collect()
}

/// 在 `repo` 目录运行 `gh <args>`:退出码为 0 时返回 UTF-8 解码后的 stdout,
/// 否则(spawn 失败即无 gh、超时、非零退出)返回 `None`。执行器收口
/// `super::run_capture`(W4-004,承 git.rs 同形)。
fn run_gh(repo: &Path, args: &[&str]) -> Option<String> {
    super::run_capture(repo, "gh", args, GH_TIMEOUT)
}
