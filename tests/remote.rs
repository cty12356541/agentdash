//! 远程源观察者的集成测试(W2-007 / T7)。
//!
//! agentdash 是纯二进制 crate,集成测试用 `#[path]` 直接挂载被测模块;
//! 探测命令经 [`remote::fetch_with`] 注入,真 gh 不进单测。缓存语义
//! (命中 / 过期 / 损坏跳过 / gh 缺失)逐一路径覆盖。

#[path = "../src/sources/remote.rs"]
// 单测只走 fetch_with 注入面:fetch/run_gh 及其超时常量在本挂载下无调用方
#[allow(dead_code)]
mod remote;

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use remote::RemoteFacts;
use serde_json::json;

/// 注入用命令执行器:记录每次 gh 子命令参数,按预置脚本回放 stdout。
struct FakeGh {
    /// 每次调用的 args 追加记录(命中缓存时必须为空)。
    calls: RefCell<Vec<Vec<String>>>,
    /// `gh pr view` 的回放输出。
    view: Option<String>,
    /// `gh pr checks` 的回放输出。
    checks: Option<String>,
}

impl FakeGh {
    fn new(view: Option<String>, checks: Option<String>) -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            view,
            checks,
        }
    }

    fn run(&self, args: &[&str]) -> Option<String> {
        self.calls
            .borrow_mut()
            .push(args.iter().map(ToString::to_string).collect());
        let joined = args.join(" ");
        if joined.starts_with("pr view") {
            return self.view.clone();
        }
        if joined.starts_with("pr checks") {
            return self.checks.clone();
        }
        None
    }

    fn call_count(&self) -> usize {
        self.calls.borrow().len()
    }
}

/// 命中缓存的 fixture 事实(与 [`cached_json`] 互为镜像)。
fn cached_facts() -> RemoteFacts {
    RemoteFacts {
        pr_number: 7,
        pr_title: "stale cached title".to_owned(),
        checks: vec![("old-check".to_owned(), "SUCCESS".to_owned())],
    }
}

fn cached_json() -> String {
    json!({
        "pr_number": 7,
        "pr_title": "stale cached title",
        "checks": [["old-check", "SUCCESS"]]
    })
    .to_string()
}

/// 实时探测的 fixture 回放(与 [`fresh_facts`] 互为镜像)。
fn fresh_view_json() -> String {
    json!({"number": 42, "title": "W2 batch three"}).to_string()
}

fn fresh_checks_json() -> String {
    json!({"checks": [
        {"name": "cargo-test", "state": "SUCCESS"},
        {"name": "cargo-clippy", "state": "FAILURE"}
    ]})
    .to_string()
}

fn fresh_facts() -> RemoteFacts {
    RemoteFacts {
        pr_number: 42,
        pr_title: "W2 batch three".to_owned(),
        checks: vec![
            ("cargo-test".to_owned(), "SUCCESS".to_owned()),
            ("cargo-clippy".to_owned(), "FAILURE".to_owned()),
        ],
    }
}

// ---------------------------------------------------------------- fixtures

struct TempRepo(PathBuf);

impl TempRepo {
    /// 带 `.git` 标记的伪仓库(fetch 只探测 git 仓;无需真 git 元数据)。
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("agentdash-remote-{tag}-{}-{n}", std::process::id()));
        fs::create_dir_all(dir.join(".git")).expect("create fixture repo");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn cache_path(&self) -> PathBuf {
        self.0.join(".agentdash").join("cache").join("gh.json")
    }

    /// 预置缓存文件并把 mtime 回拨 `age_secs` 秒(0 = 新鲜)。
    fn seed_cache(&self, content: &str, age_secs: u64) {
        let path = self.cache_path();
        fs::create_dir_all(path.parent().unwrap()).expect("mkdir cache");
        fs::write(&path, content).expect("write cache");
        let f = fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open cache for backdate");
        f.set_times(
            fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(age_secs)),
        )
        .expect("backdate cache mtime");
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.0));
    }
}

// ---------------------------------------------------------------- 缓存路径

#[test]
fn fresh_cache_hit_skips_probe() {
    let repo = TempRepo::new("hit");
    repo.seed_cache(&cached_json(), 0); // 刚写入:TTL 内
    let gh = FakeGh::new(Some(fresh_view_json()), Some(fresh_checks_json()));
    let facts = remote::fetch_with(repo.path(), |args| gh.run(args)).expect("cache hit");
    assert_eq!(facts, cached_facts(), "命中缓存应原样返回旧事实");
    assert_eq!(gh.call_count(), 0, "TTL 内命中缓存不得发起任何 gh 子进程");
}

#[test]
fn expired_cache_reprobes_and_rewrites() {
    let repo = TempRepo::new("expire");
    repo.seed_cache(&cached_json(), 121); // 超过 120s TTL
    let gh = FakeGh::new(Some(fresh_view_json()), Some(fresh_checks_json()));
    let facts = remote::fetch_with(repo.path(), |args| gh.run(args)).expect("live probe");
    assert_eq!(facts, fresh_facts());
    assert_eq!(
        gh.call_count(),
        2,
        "过期缓存后应恰探测 view + checks 两命令"
    );
    let rewritten: RemoteFacts =
        serde_json::from_str(&fs::read_to_string(repo.cache_path()).expect("cache readable"))
            .expect("cache valid json");
    assert_eq!(rewritten, fresh_facts(), "过期缓存应以新事实覆写");
}

#[test]
fn corrupt_cache_is_skipped_and_replaced() {
    let repo = TempRepo::new("corrupt");
    repo.seed_cache("{{{not json", 0); // mtime 新鲜但内容损坏
    let gh = FakeGh::new(Some(fresh_view_json()), Some(fresh_checks_json()));
    let facts = remote::fetch_with(repo.path(), |args| gh.run(args)).expect("live probe");
    assert_eq!(facts, fresh_facts(), "损坏缓存按未命中处理,照走实时探测");
    assert_eq!(gh.call_count(), 2);
    let rewritten: RemoteFacts =
        serde_json::from_str(&fs::read_to_string(repo.cache_path()).expect("cache readable"))
            .expect("cache valid json");
    assert_eq!(rewritten, fresh_facts(), "损坏缓存应被成功探测覆写");
}

#[test]
fn no_cache_no_gh_creates_no_cache() {
    let repo = TempRepo::new("nogh");
    let gh = FakeGh::new(None, None); // gh 缺失 / 命令失败:执行器回 None
    assert_eq!(
        remote::fetch_with(repo.path(), |args| gh.run(args)),
        None,
        "gh 不可用降级 None"
    );
    assert_eq!(gh.call_count(), 1, "view 失败即止,checks 不再发起");
    assert!(!repo.cache_path().exists(), "探测失败不得写缓存");
}

// ---------------------------------------------------------------- 降级路径

#[test]
fn non_git_directory_never_probes() {
    let dir = std::env::temp_dir().join(format!("agentdash-remote-plain-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("create plain dir");
    let gh = FakeGh::new(Some(fresh_view_json()), Some(fresh_checks_json()));
    assert_eq!(
        remote::fetch_with(&dir, |args| gh.run(args)),
        None,
        "非 git 目录直接不探测"
    );
    assert_eq!(gh.call_count(), 0, "非 git 目录不得发起 gh 子进程");
    drop(fs::remove_dir_all(&dir));
}

#[test]
fn pr_checks_failure_keeps_pr_with_empty_checks() {
    let repo = TempRepo::new("nochecks");
    let gh = FakeGh::new(Some(fresh_view_json()), None); // PR 在,checks 尚无
    let facts = remote::fetch_with(repo.path(), |args| gh.run(args)).expect("pr facts");
    assert_eq!(facts.pr_number, 42);
    assert_eq!(facts.pr_title, "W2 batch three");
    assert!(
        facts.checks.is_empty(),
        "checks 失败只损失 checks,不算探测失败"
    );
}

#[test]
fn malformed_view_output_is_none() {
    let repo = TempRepo::new("malformed");
    // 合法 JSON 但缺 number 字段:按损坏输出处理
    let gh = FakeGh::new(Some(r#"{"title": "no number"}"#.to_owned()), None);
    assert_eq!(remote::fetch_with(repo.path(), |args| gh.run(args)), None);
}

#[test]
fn checks_json_tolerates_top_level_array() {
    let repo = TempRepo::new("array");
    let gh = FakeGh::new(
        Some(fresh_view_json()),
        Some(r#"[{"name":"ci","state":"PENDING"}]"#.to_owned()),
    );
    let facts = remote::fetch_with(repo.path(), |args| gh.run(args)).expect("pr facts");
    assert_eq!(
        facts.checks,
        vec![("ci".to_owned(), "PENDING".to_owned())],
        "顶层裸数组形态兼容"
    );
}
