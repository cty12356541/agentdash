//! agentdash: agent progress dashboard CLI(W1-007 起 render/oneline/watch 全量接线)。

mod contract;
// W1-007 接线后 events 仅消费 gates/warnings;agents/tools 与 AgentEntry
// 待 W2 activity 车道消费,窄域放行(替代原 crate 级 allow)。
#[allow(dead_code)]
mod events;
mod hook;
mod model;
mod render;
mod sources;
mod tui;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
agentdash - agent progress dashboard

Usage: agentdash <COMMAND> [ARGS]

Commands:
  render panel|graph [PATH]  Render the dashboard panel or the task DAG
  oneline [PATH]             Print a one-line status summary
  watch [--once] [PATH]      Watch a plan and refresh live (q/Ctrl-C quits)
  hook <EVENT>               Consume a host-tool hook payload from stdin

Arguments:
  [PATH]        Path to the project or plan directory [default: .]
  watch --once  Render a single frame and exit (implied when stdin is not a TTY)

Options:
  -h, --help  Print help";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some("hook") => {
            // 集成包钩子:agentdash hook <event>,payload 从 stdin 读(W1-008b)
            hook::run(args.get(1).map(String::as_str))
        }
        Some("render") => cmd_render(&args[1..]),
        Some("oneline") => cmd_oneline(&args[1..]),
        Some("watch") => cmd_watch(&args[1..]),
        Some(cmd) => {
            eprintln!("error: unrecognized command `{cmd}`\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// `render panel|graph [PATH]`:打印对应渲染。宽度非 tty 用默认、tty 读终端
/// 列钳 40..120;模型走多源合并(损坏降级为警告行,不失败)。
fn cmd_render(rest: &[String]) -> ExitCode {
    let Some(view) = rest.first().map(String::as_str) else {
        eprintln!("error: render needs a view: `render panel|graph [PATH]`\n\n{USAGE}");
        return ExitCode::from(2);
    };
    let default_width = match view {
        "panel" => render::DEFAULT_PANEL_WIDTH,
        "graph" => render::DEFAULT_GRAPH_WIDTH,
        other => {
            eprintln!("error: unknown render view `{other}` (expected panel|graph)\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let Some(path) = positional_path(&rest[1..]) else {
        return ExitCode::from(2);
    };
    let dash = model::merge(&path);
    let width = tui::stdout_width(default_width);
    let rendered = match view {
        "panel" => render::render_panel(&dash, width),
        _ => render::render_graph(&dash, width),
    };
    println!("{rendered}");
    ExitCode::SUCCESS
}

/// `oneline [PATH]`:无 ANSI 单行 statusline。
fn cmd_oneline(rest: &[String]) -> ExitCode {
    let Some(path) = positional_path(rest) else {
        return ExitCode::from(2);
    };
    println!("{}", render::render_oneline(&model::merge(&path)));
    ExitCode::SUCCESS
}

/// `watch [--once] [PATH]`:ratatui watch;`--once` 渲染一帧即退,非 tty
/// stdin 同样自动退化为单帧。
fn cmd_watch(rest: &[String]) -> ExitCode {
    let mut once = false;
    let mut path: Option<PathBuf> = None;
    for arg in rest {
        match arg.as_str() {
            "--once" => once = true,
            other if other.starts_with('-') => {
                eprintln!("error: unknown flag `{other}` (watch takes [--once] [PATH])\n\n{USAGE}");
                return ExitCode::from(2);
            }
            other if path.is_none() => path = Some(PathBuf::from(other)),
            _ => {
                eprintln!("error: unexpected extra arguments after [PATH]\n\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let repo = path.unwrap_or_else(|| PathBuf::from("."));
    let outcome = if once {
        tui::watch_once(&repo)
    } else {
        tui::watch(&repo, tui::MODEL_INTERVAL)
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: watch failed: {err}");
            ExitCode::FAILURE
        }
    }
}

/// 取 `[PATH]` 位置参数:缺省 `.`;旗标与多余参数报错(返回 `None` 时
/// 调用方已打印用法,应以退出码 2 终止)。
fn positional_path(rest: &[String]) -> Option<PathBuf> {
    match rest {
        [] => Some(PathBuf::from(".")),
        [only] if !only.starts_with('-') => Some(PathBuf::from(only)),
        [flag] => {
            eprintln!("error: unexpected flag `{flag}`\n\n{USAGE}");
            None
        }
        _ => {
            eprintln!("error: unexpected extra arguments after [PATH]\n\n{USAGE}");
            None
        }
    }
}
