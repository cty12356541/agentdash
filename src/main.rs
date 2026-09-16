//! agentdash: agent progress dashboard CLI(W1-007 起 render/oneline/watch 全量接线)。

mod contract;
// W1-007 接线后 events 仅消费 gates/warnings;agents/tools 与 AgentEntry
// 待 W2 activity 车道消费,窄域放行(替代原 crate 级 allow)。
#[allow(dead_code)]
mod events;
// W10-001:自研 glob(分量级 `*`/`?`),panel 多仓聚合的参数展开面
mod glob;
mod hook;
mod lang;
mod model;
mod render;
mod sources;
// W11-004:agentstats 观测面(三张纯文本表),私有模块单入口平铺
mod stats;
mod tui;

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let lang = lang::current();
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help") => {
            println!("{}", lang.usage());
            ExitCode::SUCCESS
        }
        // 自检入口:安装指引(README ×2 / kits README ×1 / bin README ×1)
        // 以 `agentdash --version` 收尾
        Some("-V" | "--version") => {
            println!("agentdash {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("hook") => {
            // 集成包钩子:agentdash hook [--host <name>] <event>,payload 从
            // stdin 读(W1-008b;--host 多宿主归属,W7-001)
            hook::run(&args[1..])
        }
        Some("render") => cmd_render(&args[1..], lang),
        Some("oneline") => cmd_oneline(&args[1..], lang),
        Some("watch") => cmd_watch(&args[1..], lang),
        // 观测面(W11-004):宿主/gate/周转三表,自带参数解析(见 cmd_stats)
        Some("stats") => cmd_stats(&args[1..], lang),
        Some(cmd) => {
            eprintln!("{}\n\n{}", lang.unrecognized_command(cmd), lang.usage());
            ExitCode::from(2)
        }
    }
}

/// `render` 输出格式(W6-002):ansi(缺省)/ svg(仅 graph)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderFormat {
    /// 终端字符输出(承旧路径)。
    Ansi,
    /// 矢量 DAG 文档(仅 `render graph`)。
    Svg,
}

/// `render` 参数解析(W6-002;W10-001 起多 PATH;W11-003 起 `--no-infer`,
/// 纯函数):`--format ansi|svg`(或 `--format=X`,缺省 ansi)+ `--no-infer`
/// (关闭无 who completed 配对启发,缺省开启)+ 任意个 [PATH];panel 侧
/// 逐参 glob 展开,graph 侧仍限单个,由 [`cmd_render`] 收口。`Err` 为已成型
/// 错误消息,调用方打印后退 2。
fn parse_render_args(
    rest: &[String],
    lang: lang::Lang,
) -> Result<(RenderFormat, Vec<PathBuf>, bool), String> {
    let mut format = RenderFormat::Ansi;
    let mut paths = Vec::new();
    let mut infer = true;
    let mut iter = rest.iter();
    while let Some(arg) = iter.next() {
        if arg == "--format" {
            format = match iter.next().map(String::as_str) {
                Some("ansi") => RenderFormat::Ansi,
                Some("svg") => RenderFormat::Svg,
                other => {
                    return Err(lang.format_invalid(other.unwrap_or(lang.missing_value())));
                }
            };
        } else if let Some(value) = arg.strip_prefix("--format=") {
            format = match value {
                "ansi" => RenderFormat::Ansi,
                "svg" => RenderFormat::Svg,
                other => return Err(lang.format_invalid(other)),
            };
        } else if arg == "--no-infer" {
            infer = false;
        } else if arg.starts_with('-') {
            return Err(lang.unexpected_flag(arg));
        } else {
            paths.push(PathBuf::from(arg));
        }
    }
    Ok((format, paths, infer))
}

/// `render panel|graph|digest [PATH]...`:打印对应渲染(digest 自带解析臂,
/// 见 [`cmd_render_digest`])。宽度非 tty 用默认、tty 读
/// 终端原始列:低于 40 列退化为 oneline 单行(AD-ERR-004),否则钳 40..120
/// 出框化视图;模型走多源合并(损坏降级为警告行,不失败)。`--format svg`
/// 仅 graph(W6-002):矢量 DAG 文档,其余同旧路径。panel 自 W10-001 起
/// 收多 PATH(逐参自研 glob 展开,多仓精要视图,见 [`cmd_render_panel`]);
/// graph 仍至多一个 PATH,多余即用法错退 2。
fn cmd_render(rest: &[String], lang: lang::Lang) -> ExitCode {
    let Some(view) = rest.first().map(String::as_str) else {
        eprintln!("{}\n\n{}", lang.render_needs_view(), lang.usage());
        return ExitCode::from(2);
    };
    // 离场摘要(W10-002):自带解析臂——`--strict` 旗标 + 至多一位置;不收
    // `--format`(svg 是图视图专属,digest 无格式面),不走下方宽度面板路径
    if view == "digest" {
        return cmd_render_digest(&rest[1..], lang);
    }
    let default_width = match view {
        "panel" => render::DEFAULT_PANEL_WIDTH,
        "graph" => render::graph::DEFAULT_GRAPH_WIDTH,
        other => {
            eprintln!("{}\n\n{}", lang.unknown_view(other), lang.usage());
            return ExitCode::from(2);
        }
    };
    let (format, paths, infer) = match parse_render_args(&rest[1..], lang) {
        Ok(parsed) => parsed,
        Err(msg) => {
            eprintln!("error: {msg}\n\n{}", lang.usage());
            return ExitCode::from(2);
        }
    };
    if format == RenderFormat::Svg && view != "graph" {
        eprintln!("{}\n\n{}", lang.svg_only_graph(), lang.usage());
        return ExitCode::from(2);
    }
    // 多仓聚合仅 panel(D1):graph(与 oneline/watch)仍单 PATH,措辞承旧
    if view != "panel" && paths.len() > 1 {
        eprintln!(
            "error: {}\n\n{}",
            lang.unexpected_extra("[PATH]"),
            lang.usage()
        );
        return ExitCode::from(2);
    }
    if view == "panel" {
        let mut paths = paths;
        if paths.is_empty() {
            paths.push(PathBuf::from(".")); // 缺省 cwd(承现行)
        }
        return cmd_render_panel(&paths, default_width, infer);
    }
    let path = paths
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from("."));
    let dash = model::merge_opts(&path, infer);
    match format {
        RenderFormat::Svg => println!("{}", render::graph::render_graph_svg(&dash)),
        RenderFormat::Ansi => match tui::output_form(tui::stdout_cols(), default_width) {
            tui::OutputForm::OneLine => println!("{}", render::render_oneline(&dash)),
            tui::OutputForm::Framed(width) => {
                println!("{}", render::graph::render_graph(&dash, width));
            }
        },
    }
    ExitCode::SUCCESS
}

/// panel 多 PATH 入口(W10-001):逐参自研 glob 展开(含 `*`/`?` 的分量,
/// 字面前缀 `read_dir` 逐段匹配)。恰 1 仓(含展开后)= 现行全面板路径
/// (无 nomatch 时输出逐字节不变,W10 黄金);N>1 逐仓精要块(空行分隔);
/// 无匹配 pattern 一律尾随 ⚠ 行(W10 终审 Important-1 起 N==1 同样补行,
/// 不再被单仓路径吞掉);0 仓(全模式无匹配)逐 pattern 一行 ⚠——恒退 0,
/// 不崩溃。窄终端退化同现行:低于 40 列逐仓 oneline 单行。`infer` 透传
/// 合并(W11-003 `--no-infer`)。
fn cmd_render_panel(paths: &[PathBuf], default_width: usize, infer: bool) -> ExitCode {
    let (repos, nomatch) = glob::expand_args(paths);
    match repos.len() {
        0 => {
            let lines: Vec<String> = nomatch
                .iter()
                .map(|pattern| nomatch_line(pattern))
                .collect();
            println!("{}", lines.join("\n"));
        }
        1 => {
            let dash = model::merge_opts(&repos[0], infer);
            // W10 终审 Important-1:恰 1 仓(含展开后)也随行补无匹配 ⚠——
            // 拼接法与 N>1 臂同构,nomatch 为空时 join 单元素逐字节不变
            // (单仓字节钉 W10 黄金不破)
            match tui::output_form(tui::stdout_cols(), default_width) {
                tui::OutputForm::OneLine => {
                    let mut lines = vec![render::render_oneline(&dash)];
                    lines.extend(nomatch.iter().map(|pattern| nomatch_line(pattern)));
                    println!("{}", lines.join("\n"));
                }
                tui::OutputForm::Framed(width) => {
                    let mut blocks = vec![render::render_panel(&dash, width)];
                    blocks.extend(nomatch.iter().map(|pattern| nomatch_line(pattern)));
                    println!("{}", blocks.join("\n\n"));
                }
            }
        }
        _ => match tui::output_form(tui::stdout_cols(), default_width) {
            tui::OutputForm::OneLine => {
                let mut lines: Vec<String> = repos
                    .iter()
                    .map(|path| render::render_oneline(&model::merge_opts(path, infer)))
                    .collect();
                lines.extend(nomatch.iter().map(|pattern| nomatch_line(pattern)));
                println!("{}", lines.join("\n"));
            }
            tui::OutputForm::Framed(width) => {
                let mut blocks: Vec<String> = repos
                    .iter()
                    .map(|path| render::render_brief(&model::merge_opts(path, infer), width))
                    .collect();
                blocks.extend(nomatch.iter().map(|pattern| nomatch_line(pattern)));
                println!("{}", blocks.join("\n\n"));
            }
        },
    }
    ExitCode::SUCCESS
}

/// 无匹配 pattern 的 ⚠ 行(与 panel 警告行同款着色前缀;D1:模式无匹配产
/// 一行 ⚠ 而非崩溃)。
fn nomatch_line(pattern: &str) -> String {
    format!("{}⚠ no match: {pattern}{}", render::C_WARN, render::C_END)
}

/// `render digest [--strict] [--no-infer] [PATH]`(W10-002):离场摘要,
/// 纯文本无 ANSI。单 PATH(oneline 同款缺省 `.`;多仓 digest 即用法错退 2,
/// D2);`--strict` 是本仓首个内容性退出码——存在失败门或 blocked 任务退 1,
/// 否则 0。渲染与判定分层:`render::render_digest` 只出文本,退出码在 cmd
/// 层凭 [`render::digest_needs_attention`] 收口;`--no-infer` 透传合并
/// (W11-003)。
fn cmd_render_digest(rest: &[String], lang: lang::Lang) -> ExitCode {
    let mut strict = false;
    let mut infer = true;
    let mut paths: Vec<PathBuf> = Vec::new();
    for arg in rest {
        if arg == "--strict" {
            strict = true;
        } else if arg == "--no-infer" {
            infer = false;
        } else if arg.starts_with('-') {
            eprintln!("error: {}\n\n{}", lang.unexpected_flag(arg), lang.usage());
            return ExitCode::from(2);
        } else {
            paths.push(PathBuf::from(arg));
        }
    }
    if paths.len() > 1 {
        eprintln!(
            "error: {}\n\n{}",
            lang.unexpected_extra("[PATH]"),
            lang.usage()
        );
        return ExitCode::from(2);
    }
    let path = paths.pop().unwrap_or_else(|| PathBuf::from("."));
    let dash = model::merge_opts(&path, infer);
    println!("{}", render::render_digest(&dash));
    if strict && render::digest_needs_attention(&dash) {
        return ExitCode::FAILURE; // 1:失败门/blocked 在场(cron 监控用)
    }
    ExitCode::SUCCESS
}

/// `stats [--host <name>] [PATH]`(W11-004):观测面——宿主使用率 / gate
/// 通过率 / 任务周转三张纯文本表(渲染归 [`stats::render`],本层只收参)。
/// 自带旗标解析:`--host <name>`(或 `--host=<name>`,空白值 = 用法错)+
/// 至多一位置 PATH(缺省 `.`);未知旗标与多余 PATH 用法错退 2。stats 是
/// 纯投影且恒退 0(D2):渲染/退出码不在此层判定,内容永不影响退出。
fn cmd_stats(rest: &[String], lang: lang::Lang) -> ExitCode {
    let mut host: Option<String> = None;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut iter = rest.iter();
    while let Some(arg) = iter.next() {
        if arg == "--host" {
            match iter.next().map(String::as_str).map(str::trim) {
                Some(name) if !name.is_empty() => host = Some(name.to_owned()),
                other => {
                    let shown = other.unwrap_or_else(|| lang.missing_value());
                    eprintln!("error: {}\n\n{}", lang.host_needs_name(shown), lang.usage());
                    return ExitCode::from(2);
                }
            }
        } else if let Some(value) = arg.strip_prefix("--host=") {
            match value.trim() {
                "" => {
                    eprintln!(
                        "error: {}\n\n{}",
                        lang.host_needs_name(lang.missing_value()),
                        lang.usage()
                    );
                    return ExitCode::from(2);
                }
                name => host = Some(name.to_owned()),
            }
        } else if arg.starts_with('-') {
            eprintln!("error: {}\n\n{}", lang.unexpected_flag(arg), lang.usage());
            return ExitCode::from(2);
        } else {
            paths.push(PathBuf::from(arg));
        }
    }
    if paths.len() > 1 {
        eprintln!(
            "error: {}\n\n{}",
            lang.unexpected_extra("[PATH]"),
            lang.usage()
        );
        return ExitCode::from(2);
    }
    let path = paths.pop().unwrap_or_else(|| PathBuf::from("."));
    stats::run(&path, host.as_deref())
}

/// `oneline [--no-infer] [PATH]`:无 ANSI 单行 statusline;旗标与多余参数
/// 报错退 2(W11-003 起 `--no-infer` 透传合并)。
fn cmd_oneline(rest: &[String], lang: lang::Lang) -> ExitCode {
    let mut infer = true;
    let mut paths: Vec<PathBuf> = Vec::new();
    for arg in rest {
        if arg == "--no-infer" {
            infer = false;
        } else if arg.starts_with('-') {
            eprintln!("error: {}\n\n{}", lang.unexpected_flag(arg), lang.usage());
            return ExitCode::from(2);
        } else {
            paths.push(PathBuf::from(arg));
        }
    }
    if paths.len() > 1 {
        eprintln!(
            "error: {}\n\n{}",
            lang.unexpected_extra("[PATH]"),
            lang.usage()
        );
        return ExitCode::from(2);
    }
    let path = paths.pop().unwrap_or_else(|| PathBuf::from("."));
    println!(
        "{}",
        render::render_oneline(&model::merge_opts(&path, infer))
    );
    ExitCode::SUCCESS
}

/// `watch [--once] [SECONDS] [PATH]`:ratatui watch;位置参数解析收口在
/// [`tui::parse_watch_args`](W2-008 恢复 interval 位置档);`--once` 渲染
/// 一帧即退,非 tty stdin 同样自动退化为单帧。
fn cmd_watch(rest: &[String], lang: lang::Lang) -> ExitCode {
    match tui::parse_watch_args(rest) {
        Ok((once, interval, repo)) => {
            let outcome = if once {
                tui::watch_once(&repo)
            } else {
                tui::watch(&repo, interval)
            };
            match outcome {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("error: {} {err}", lang.watch_failed());
                    ExitCode::FAILURE
                }
            }
        }
        Err(msg) => {
            let text = match msg {
                tui::WatchArgError::UnknownFlag(flag) => lang.unexpected_flag(&flag),
                tui::WatchArgError::ExtraArgs => lang.unexpected_extra("[SECONDS] [PATH]"),
            };
            eprintln!("error: {text}\n\n{}", lang.usage());
            ExitCode::from(2)
        }
    }
}
