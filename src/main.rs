//! agentdash: agent progress dashboard CLI(W1-007 起 render/oneline/watch 全量接线)。

mod contract;
// W1-007 接线后 events 仅消费 gates/warnings;agents/tools 与 AgentEntry
// 待 W2 activity 车道消费,窄域放行(替代原 crate 级 allow)。
#[allow(dead_code)]
mod events;
mod hook;
mod lang;
mod model;
mod render;
mod sources;
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

/// `render` 参数解析(W6-002,纯函数):`--format ansi|svg`(或 `--format=X`,
/// 缺省 ansi)+ 至多一个 [PATH]。`Err` 为已成型错误消息,调用方打印后退 2。
fn parse_render_args(
    rest: &[String],
    lang: lang::Lang,
) -> Result<(RenderFormat, Option<PathBuf>), String> {
    let mut format = RenderFormat::Ansi;
    let mut path = None;
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
        } else if arg.starts_with('-') {
            return Err(lang.unexpected_flag(arg));
        } else if path.is_none() {
            path = Some(PathBuf::from(arg));
        } else {
            return Err(lang.unexpected_extra("[PATH]"));
        }
    }
    Ok((format, path))
}

/// `render panel|graph [PATH]`:打印对应渲染。宽度非 tty 用默认、tty 读终端
/// 原始列:低于 40 列退化为 oneline 单行(AD-ERR-004),否则钳 40..120 出
/// 框化视图;模型走多源合并(损坏降级为警告行,不失败)。`--format svg`
/// 仅 graph(W6-002):矢量 DAG 文档,其余同旧路径。
fn cmd_render(rest: &[String], lang: lang::Lang) -> ExitCode {
    let Some(view) = rest.first().map(String::as_str) else {
        eprintln!("{}\n\n{}", lang.render_needs_view(), lang.usage());
        return ExitCode::from(2);
    };
    let default_width = match view {
        "panel" => render::DEFAULT_PANEL_WIDTH,
        "graph" => render::graph::DEFAULT_GRAPH_WIDTH,
        other => {
            eprintln!("{}\n\n{}", lang.unknown_view(other), lang.usage());
            return ExitCode::from(2);
        }
    };
    let (format, path) = match parse_render_args(&rest[1..], lang) {
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
    let path = path.unwrap_or_else(|| PathBuf::from("."));
    let dash = model::merge(&path);
    match format {
        RenderFormat::Svg => println!("{}", render::graph::render_graph_svg(&dash)),
        RenderFormat::Ansi => match tui::output_form(tui::stdout_cols(), default_width) {
            tui::OutputForm::OneLine => println!("{}", render::render_oneline(&dash)),
            tui::OutputForm::Framed(width) => {
                let rendered = if view == "panel" {
                    render::render_panel(&dash, width)
                } else {
                    render::graph::render_graph(&dash, width)
                };
                println!("{rendered}");
            }
        },
    }
    ExitCode::SUCCESS
}

/// `oneline [PATH]`:无 ANSI 单行 statusline。
fn cmd_oneline(rest: &[String], lang: lang::Lang) -> ExitCode {
    let Some(path) = positional_path(rest, lang) else {
        return ExitCode::from(2);
    };
    println!("{}", render::render_oneline(&model::merge(&path)));
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

/// 取 `[PATH]` 位置参数:缺省 `.`;旗标与多余参数报错(返回 `None` 时
/// 调用方已打印用法,应以退出码 2 终止)。
fn positional_path(rest: &[String], lang: lang::Lang) -> Option<PathBuf> {
    match rest {
        [] => Some(PathBuf::from(".")),
        [only] if !only.starts_with('-') => Some(PathBuf::from(only)),
        [flag] => {
            eprintln!("error: {}\n\n{}", lang.unexpected_flag(flag), lang.usage());
            None
        }
        _ => {
            eprintln!(
                "error: {}\n\n{}",
                lang.unexpected_extra("[PATH]"),
                lang.usage()
            );
            None
        }
    }
}
