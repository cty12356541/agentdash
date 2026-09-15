//! `--version` / `-V` 自检集成测试(W2 Task 1b)。
//!
//! README 与 kits/claude-code 安装指引均以 `agentdash --version  # 自检` 收尾:
//! 子进程回放二进制,断言退出码 0 且 stdout 按惯例报出 `agentdash <version>`
//! (版本号取自 Cargo.toml `package.version`)。
//!
//! W9-005 起兼测 `--help` 双语(`AGENTDASH_LANG` 切换,缺省中文):语言经
//! 子进程环境注入,互相隔离,无进程级 env 竞态。

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

/// 子进程回放(注入 `AGENTDASH_LANG`):语言只作用于子进程,无 env 竞态。
fn run_with_lang(args: &[&str], lang: &str) -> Output {
    Command::new(EXE)
        .args(args)
        .env("AGENTDASH_LANG", lang)
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

// ------------------------------------------------------------ --help 双语(W9-005)

/// 帮助两语言共同的关键词面(CLI 令牌不随语言变)。
fn assert_usage_tokens(text: &str, label: &str) {
    for token in [
        "render panel|graph",
        "oneline [PATH]",
        "watch [--once] [SECONDS] [PATH]",
        "hook <EVENT>",
        "-V, --version",
        "AGENTDASH_LANG",
    ] {
        assert!(text.contains(token), "{label} 缺关键词 `{token}`:\n{text}");
    }
}

#[test]
fn help_defaults_to_chinese() {
    // 缺省(无 AGENTDASH_LANG)= 中文:--help / -h / 无参数裸调同文同码
    let arg_sets: [&[&str]; 3] = [&["--help"], &["-h"], &[]];
    for args in arg_sets {
        let out = run(args);
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(out.status.success(), "帮助应退 0");
        assert!(text.contains("用法:"), "缺省帮助应为中文(含「用法:」)");
        assert!(text.contains("命令:"), "缺省帮助应为中文(含「命令:」)");
        assert!(text.contains("打印版本号"), "选项段应为中文");
        assert!(text.contains("AGENTDASH_LANG=en"), "尾部应附英文切换提示");
        assert_usage_tokens(&text, "中文帮助");
    }
}

#[test]
fn help_switches_to_english_via_lang_env() {
    for value in ["en", "en-US", "en_US"] {
        let out = run_with_lang(&["--help"], value);
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(out.status.success(), "帮助应退 0");
        assert!(text.contains("Usage:"), "`{value}` 应切英文(含 Usage:)");
        assert!(text.contains("Print version"), "`{value}` 选项段应为英文");
        assert!(text.contains("AGENTDASH_LANG=zh"), "尾部应附中文切换提示");
        assert_usage_tokens(&text, "英文帮助");
    }
}

#[test]
fn lang_env_variants_stay_chinese() {
    // zh/zh-CN/zh_CN/未知值/空值:一律回落中文(降级不报错)
    for value in ["zh", "zh-CN", "zh_CN", "fr", ""] {
        let out = run_with_lang(&["--help"], value);
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(out.status.success());
        assert!(text.contains("用法:"), "`{value}` 应回落中文");
    }
}

#[test]
fn unrecognized_command_errors_in_current_lang() {
    // 未知命令:缺省中文报错 + 用法随语言;退出码 2 不变
    let out = run(&["bogus"]);
    assert_eq!(out.status.code(), Some(2), "未知命令退 2");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        stderr.contains("无法识别的命令 `bogus`"),
        "缺省报错应为中文:{stderr}"
    );
    assert!(stderr.contains("用法:"), "用法应随缺省中文");

    let en = run_with_lang(&["bogus"], "en");
    assert_eq!(en.status.code(), Some(2));
    let stderr_en = String::from_utf8_lossy(&en.stderr).into_owned();
    assert!(
        stderr_en.contains("unrecognized command `bogus`"),
        "en 报错应为英文:{stderr_en}"
    );
    assert!(stderr_en.contains("Usage:"), "用法应随 en 切英文");
}
