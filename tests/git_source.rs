//! W1-004:git 快照观察者集成测试。
//!
//! agentdash 当前是纯二进制 crate(无 lib 目标),集成测试用 `#[path]`
//! 直接挂载 `src/sources/git.rs`,以进程内方式验证公开接口。

// W4-004:git.rs 经 super::run_capture 消费公共执行器,挂载点收口 sources/mod.rs
#[path = "../src/sources/mod.rs"]
#[allow(dead_code)]
mod sources;

use sources::git;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// 在 `repo` 目录里跑一条 git 命令(fixture 准备专用),失败即测试报错。
fn run_git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .expect("git should be on PATH for tests");
    assert!(
        status.success(),
        "git {args:?} failed in {}",
        repo.display()
    );
}

fn next_dir(name: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "agentdash-w1-004-{name}-{}-{serial}",
        std::process::id()
    ))
}

fn cleanup(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

/// 建一个受控 fixture 仓库:git init + 本地身份 + 固定分支名 main + 2 次提交。
fn fixture_repo(name: &str) -> PathBuf {
    let repo = next_dir(name);
    fs::create_dir_all(&repo).expect("create fixture dir");
    run_git(&repo, &["init"]);
    run_git(&repo, &["config", "user.name", "agentdash-test"]);
    run_git(
        &repo,
        &["config", "user.email", "agentdash-test@example.com"],
    );
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    run_git(&repo, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    fs::write(repo.join("a.txt"), "first\n").expect("write a.txt");
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "one"]);
    fs::write(repo.join("b.txt"), "second\n").expect("write b.txt");
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "two"]);
    repo
}

#[test]
fn snapshot_reports_branch_head_dirty_and_recent() {
    let repo = fixture_repo("basic");
    let facts = git::snapshot(&repo);
    assert!(facts.present);
    assert_eq!(facts.branch.as_deref(), Some("main"));
    let head = facts.head_short.as_deref().expect("head_short is set");
    assert!(
        (7..=40).contains(&head.len()),
        "unexpected short sha {head}"
    );
    assert_eq!(facts.dirty, 0);
    assert_eq!(facts.ahead, 0, "no upstream => ahead 0 without error");
    assert_eq!(facts.behind, 0, "no upstream => behind 0 without error");
    assert_eq!(facts.recent.len(), 2);
    assert!(facts.recent[0].ends_with("two"));
    assert!(facts.recent[1].ends_with("one"));
    assert!(facts.recent[0].starts_with(head));
    // W2-3b F3:仓根目录随快照记录(渲染层项目名的来源),末段即目录名
    let root = facts.root.as_deref().expect("toplevel recorded");
    assert_eq!(
        Path::new(root)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .as_deref(),
        repo.file_name()
            .map(|name| name.to_string_lossy())
            .as_deref(),
        "root 取 git rev-parse --show-toplevel"
    );

    // 未提交改动:1 个已跟踪文件修改 + 1 个未跟踪文件 => dirty == 2
    fs::write(repo.join("a.txt"), "changed\n").expect("modify a.txt");
    fs::write(repo.join("c.txt"), "untracked\n").expect("write c.txt");
    let dirty_facts = git::snapshot(&repo);
    assert_eq!(dirty_facts.dirty, 2);
    cleanup(&repo);
}

#[test]
fn absent_for_non_git_directory() {
    let dir = next_dir("plain");
    fs::create_dir_all(&dir).expect("create plain dir");
    let facts = git::snapshot(&dir);
    assert_eq!(facts, git::GitFacts::absent());
    assert!(!facts.present);
    assert_eq!(facts.branch, None);
    assert_eq!(facts.head_short, None);
    assert_eq!(facts.recent, Vec::<String>::new());
    assert_eq!(facts.root, None, "非 git 仓无仓根可记");
    cleanup(&dir);
}

#[test]
fn ahead_and_behind_track_upstream() {
    let upstream = fixture_repo("upstream");
    let parent = upstream.parent().expect("fixture dir parent").to_path_buf();
    let clone_dir = next_dir("clone");
    let clone_status = Command::new("git")
        .args([
            "clone",
            upstream.to_str().expect("utf-8 temp path"),
            clone_dir.to_str().expect("utf-8 temp path"),
        ])
        .current_dir(&parent)
        .status()
        .expect("git should be on PATH for tests");
    assert!(clone_status.success(), "git clone failed");
    // clone 不继承源仓本地身份,给克隆仓单独补一份。
    run_git(&clone_dir, &["config", "user.name", "agentdash-test"]);
    run_git(
        &clone_dir,
        &["config", "user.email", "agentdash-test@example.com"],
    );
    run_git(&clone_dir, &["config", "commit.gpgsign", "false"]);

    let facts = git::snapshot(&clone_dir);
    assert_eq!(facts.ahead, 0);
    assert_eq!(facts.behind, 0);

    run_git(&upstream, &["commit", "--allow-empty", "-m", "three"]);
    run_git(&clone_dir, &["fetch"]);
    let behind_facts = git::snapshot(&clone_dir);
    assert_eq!(behind_facts.behind, 1);
    assert_eq!(behind_facts.ahead, 0);

    run_git(&clone_dir, &["commit", "--allow-empty", "-m", "local"]);
    let diverged = git::snapshot(&clone_dir);
    assert_eq!(diverged.ahead, 1);
    assert_eq!(diverged.behind, 1);

    cleanup(&clone_dir);
    cleanup(&upstream);
}
