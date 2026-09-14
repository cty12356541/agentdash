//! git 快照观察者(W1-004):对仓库目录做只读快照,采集分支、HEAD 短 SHA、
//! 脏计数、ahead/behind 与近 5 条提交。
//!
//! 约定:目录无 `.git`、git 不可用、命令非零退出或单命令超时,一律降级为
//! 空值(`GitFacts::absent()` / 字段空),绝不 panic、绝不阻塞渲染。

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// 单条 git 命令的超时上限;超时按"不可用"处理,不拖住仪表盘。
const GIT_TIMEOUT: Duration = Duration::from_secs(3);
/// 子进程退出状态的轮询间隔。
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// 一次快照的全部观测结果;`present == false` 时其余字段均为空值。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)] // 名称由 W1-004 接口契约固定
pub struct GitFacts {
    /// 目标目录是否为可用 git 仓库。
    pub present: bool,
    /// 当前分支名;分离 HEAD 或不可用时为 `None`。
    pub branch: Option<String>,
    /// HEAD 短 SHA(`rev-parse --short`);空仓或不可用时为 `None`。
    pub head_short: Option<String>,
    /// `status --porcelain` 的条目数(含未跟踪文件)。
    pub dirty: u32,
    /// 领先上游(`@{u}`)的提交数;无上游时为 0。
    pub ahead: u32,
    /// 落后上游(`@{u}`)的提交数;无上游时为 0。
    pub behind: u32,
    /// `log --oneline -5` 的行,新提交在前。
    pub recent: Vec<String>,
    /// 仓库根目录路径(`rev-parse --show-toplevel`;渲染层项目名的来源,
    /// W2-3b);裸仓等不可用时为 `None`。
    pub root: Option<String>,
}

impl GitFacts {
    /// 全空快照:目标不是可用 git 仓库时的返回值。
    #[must_use]
    pub fn absent() -> Self {
        Self::default()
    }
}

/// 对 `repo` 目录做一次只读 git 快照。任何失败都降级为空值,不 panic。
#[must_use]
pub fn snapshot(repo: &Path) -> GitFacts {
    // `.git` 可能是目录(普通仓)或文件(worktree/submodule),`exists` 两者都覆盖;
    // 再用 `rev-parse --git-dir` 确认 git 可用且目录确为有效仓库。
    if !repo.join(".git").exists() || run_git(repo, &["rev-parse", "--git-dir"]).is_none() {
        return GitFacts::absent();
    }
    let branch = run_git(repo, &["branch", "--show-current"]).and_then(|out| non_empty(&out));
    let head_short =
        run_git(repo, &["rev-parse", "--short", "HEAD"]).and_then(|out| non_empty(&out));
    let dirty = run_git(repo, &["status", "--porcelain"]).map_or(0, |out| count_lines(&out));
    let ahead = rev_count(repo, "@{u}..HEAD");
    let behind = rev_count(repo, "HEAD..@{u}");
    let recent = run_git(repo, &["log", "--oneline", "-5"])
        .map_or_else(Vec::new, |out| out.lines().map(str::to_owned).collect());
    let root = run_git(repo, &["rev-parse", "--show-toplevel"]).and_then(|out| non_empty(&out));
    GitFacts {
        present: true,
        branch,
        head_short,
        dirty,
        ahead,
        behind,
        recent,
        root,
    }
}

/// 去掉首尾空白;空白输出归一为 `None`(如分离 HEAD 时 `branch --show-current`
/// 不产生输出)。
fn non_empty(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// 非空行数(porcelain 每条目一行);仅病态超大盘点才会饱和到 `u32::MAX`。
fn count_lines(output: &str) -> u32 {
    let lines = output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    u32::try_from(lines).unwrap_or(u32::MAX)
}

/// `rev-list --count <range>`;无上游、超时或任何失败 => 0,不报错。
fn rev_count(repo: &Path, range: &str) -> u32 {
    run_git(repo, &["rev-list", "--count", range])
        .and_then(|out| out.trim().parse::<u32>().ok())
        .unwrap_or(0)
}

/// 在 `repo` 目录运行 `git <args>`:退出码为 0 时返回 UTF-8 解码后的 stdout,
/// 否则(spawn 失败即无 git、超时、非零退出)返回 `None`。
fn run_git(repo: &Path, args: &[&str]) -> Option<String> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(repo)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // 独立线程读 stdout:git 输出超过管道缓冲(如海量脏文件)时进程也能退出,
    // 主线程不会在 try_wait 轮询里假死到超时。
    let mut stdout = child.stdout.take();
    let reader = thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(handle) = stdout.as_mut() {
            let _ = handle.read_to_end(&mut buf);
        }
        buf
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let bytes = reader.join().unwrap_or_default();
                if !status.success() {
                    return None;
                }
                return Some(String::from_utf8_lossy(&bytes).into_owned());
            }
            Ok(None) => {
                if started.elapsed() >= GIT_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                thread::sleep(POLL_INTERVAL);
            }
            Err(_) => return None,
        }
    }
}
