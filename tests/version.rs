//! `--version` / `-V` 自检集成测试(W2 Task 1b)。
//!
//! README 与 kits/claude-code 安装指引均以 `agentdash --version  # 自检` 收尾:
//! 子进程回放二进制,断言退出码 0 且 stdout 按惯例报出 `agentdash <version>`
//! (版本号取自 Cargo.toml `package.version`)。

use std::process::{Command, Output, Stdio};

/// 被测二进制(cargo 注入的绝对路径,bin-only crate 走子进程回放)。
const EXE: &str = env!("CARGO_BIN_EXE_agentdash");
/// 期望版本号(与 Cargo.toml `package.version` 同源)。
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 子进程回放:无 stdin 输入,捕获 stdout/stderr。
fn run(args: &[&str]) -> Output {
    Command::new(EXE)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn agentdash")
}

/// 首行恰为 `agentdash <version>` 且进程退 0(自检语义)。
fn assert_self_check(out: &Output, flag: &str) {
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let first_line = stdout.lines().next().unwrap_or_default();
    assert!(
        out.status.success(),
        "`{flag}` 应退出 0,实得 {:?},stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !first_line.is_empty(),
        "`{flag}` stdout 不应为空,stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        first_line,
        format!("agentdash {VERSION}"),
        "`{flag}` stdout 首行应为 `agentdash {VERSION}`"
    );
}

#[test]
fn long_flag_prints_version_and_exits_zero() {
    assert_self_check(&run(&["--version"]), "--version");
}

#[test]
fn short_flag_prints_version_and_exits_zero() {
    assert_self_check(&run(&["-V"]), "-V");
}
