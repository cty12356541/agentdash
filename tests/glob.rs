//! W10-001 自研 glob 集成测试:分量级 `*` / `?` 展开(D1,零依赖)。
//!
//! agentdash 是纯二进制 crate,集成测试按 `#[path]` 在 crate 根挂载模块树,
//! 与 `tests/render_panel.rs` 同约定。glob 模块只依赖 std,单独挂载即可直测:
//! 临时目录树(含 CJK 目录名)上验证字面前缀 `read_dir` 逐段展开、通配不跨
//! 分量、无 `**` 递归、尾随分隔符只收目录、无匹配返回空表。

#![allow(dead_code)]

#[path = "../src/glob.rs"]
mod glob;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

fn next_dir(name: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "agentdash-w10-glob-{name}-{}-{serial}",
        std::process::id()
    ))
}

fn cleanup(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

/// 受控目录树:root 下建 `dirs`(名字可含 `/` 造嵌套)与 `files`。
fn tree(dirs: &[&str], files: &[&str]) -> PathBuf {
    let root = next_dir("tree");
    fs::create_dir_all(&root).expect("create root");
    for dir in dirs {
        fs::create_dir_all(root.join(dir)).expect("create dir");
    }
    for file in files {
        fs::write(root.join(file), "x\n").expect("write file");
    }
    root
}

/// 展开结果的末段名列表(与基址路径解耦,断言只看命中了什么)。
fn names(hits: &[PathBuf]) -> Vec<String> {
    hits.iter()
        .map(|hit| {
            hit.file_name()
                .expect("hit has file name")
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

#[test]
fn star_expands_direct_components_including_cjk() {
    let root = tree(&["alpha", "甲仓", "乙仓"], &["notes.txt"]);
    let hits = glob::expand(&root.join("*").to_string_lossy());
    assert_eq!(
        names(&hits),
        // 展开结果排序确定(此处 Path 字节序:ASCII 在前,CJK 按码点)
        vec!["alpha", "notes.txt", "乙仓", "甲仓"],
        "直下分量全展开(目录与文件),CJK 目录名逐字匹配: {hits:?}"
    );
    cleanup(&root);
}

#[test]
fn trailing_separator_matches_directories_only() {
    let root = tree(&["alpha", "甲仓"], &["notes.txt"]);
    let pattern = format!("{}/*/", root.display()); // `…/*/`
    let hits = glob::expand(&pattern);
    assert_eq!(
        names(&hits),
        vec!["alpha", "甲仓"],
        "尾随分隔符:只收目录,文件不混入: {hits:?}"
    );
    cleanup(&root);
}

#[test]
fn question_mark_matches_exactly_one_char() {
    let root = tree(&["a1", "a2", "ab", "abc"], &[]);
    let hits = glob::expand(&root.join("a?").to_string_lossy());
    assert_eq!(
        names(&hits),
        vec!["a1", "a2", "ab"],
        "`?` 恰配一个字符(ab 入选),`abc` 不入选: {hits:?}"
    );
    cleanup(&root);
}

#[test]
fn wildcard_stays_within_one_component() {
    let root = tree(&["a/b", "ab"], &[]);
    let hits = glob::expand(&root.join("*").to_string_lossy());
    assert_eq!(
        names(&hits),
        vec!["a", "ab"],
        "`*` 不跨分量:a/b 的深层不入选: {hits:?}"
    );
    cleanup(&root);
}

#[test]
fn double_star_is_not_recursive() {
    let root = tree(&["x/y/z"], &[]);
    let hits = glob::expand(&root.join("**").to_string_lossy());
    assert_eq!(
        names(&hits),
        vec!["x"],
        "无 `**`:双层星号按单分量通配处理,不递归下钻: {hits:?}"
    );
    cleanup(&root);
}

#[test]
fn no_match_yields_empty() {
    let root = tree(&["alpha"], &[]);
    let hits = glob::expand(&root.join("zz*").to_string_lossy());
    assert!(hits.is_empty(), "无匹配返回空表(由调用方产 ⚠ 行): {hits:?}");
    cleanup(&root);
}

#[test]
fn literal_without_wildcard_passes_through() {
    let raw = "Z:\\no-such-dir\\literal-path";
    assert_eq!(
        glob::expand(raw),
        vec![PathBuf::from(raw)],
        "不含通配符的参数不做展开、不触文件系统"
    );
}

#[test]
fn expand_args_keeps_order_and_collects_nomatch() {
    let root = tree(&["r1", "r2"], &[]);
    let args = [root.join("r1"), root.join("nope*"), root.join("r*")];
    let (repos, nomatch) = glob::expand_args(&args);
    assert_eq!(
        repos,
        vec![root.join("r1"), root.join("r1"), root.join("r2")],
        "字面参数原样透传,模式命中按展开序追加,参数序保持"
    );
    assert_eq!(
        nomatch,
        vec![root.join("nope*").to_string_lossy().into_owned()],
        "无匹配模式原串回收,供调用方逐 pattern 产 ⚠ 行"
    );
    cleanup(&root);
}
