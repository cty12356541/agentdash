//! 观察者源(W1:git 快照;W2-007:远程 PR 探测;进程/文件监视二期)。
//!
//! 公共子进程执行器 [`run_capture`](W4-004):"spawn + 独立线程读 stdout +
//! `try_wait` 轮询超时"是 git/gh 两观察者的同形双胞胎,收口在此——独立读线程
//! 保证输出超过管道缓冲时子进程也能退出(主线程不在轮询里假死到超时)。

pub mod git;
pub mod remote;

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// 子进程退出状态的轮询间隔(git/gh 两观察者同档)。
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// 在 `dir` 目录运行 `<program> <args>`:退出码为 0 时返回 UTF-8 解码后的
/// stdout,否则(spawn 失败即程序缺失、超时、非零退出)返回 [`None`]。
pub(crate) fn run_capture(
    dir: &Path,
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .current_dir(dir)
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
                if started.elapsed() >= timeout {
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
