//! agentdash: agent progress dashboard CLI (W1-001 command skeleton).

// W1 过渡:render 与 model 尚未接线进命令(W1-007+ 车道),bin 目标按死代码
// 放行;接线车道消费后应删除本 allow。
#![allow(dead_code)]

mod contract;
mod events;
mod hook;
mod model;
mod render;
mod sources;

use std::process::ExitCode;

const USAGE: &str = "\
agentdash - agent progress dashboard

Usage: agentdash <COMMAND> [PATH]

Commands:
  render  Render the dashboard panel for a plan
  oneline Print a one-line status summary for a plan
  watch   Watch a plan directory and refresh the dashboard live
  hook    Consume a host-tool hook payload from stdin (integration kits)

Arguments:
  [PATH]  Path to the project or plan directory [default: .]

Options:
  -h, --help  Print help

Unimplemented commands exit with 0 and print a notice in W1-001.";

const OK_CMDS: [&str; 3] = ["render", "oneline", "watch"];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some("hook") => {
            // 集成包钩子:agentdash hook <event>,payload 从 stdin 读(W1-008b 车道实现)
            hook::run(args.get(1).map(String::as_str))
        }
        Some(cmd) if OK_CMDS.contains(&cmd) => {
            if args.len() > 2 {
                eprintln!("error: unexpected extra arguments after [PATH]\n\n{USAGE}");
                return ExitCode::from(2);
            }
            println!("{cmd}: not implemented in W1-001");
            ExitCode::SUCCESS
        }
        Some(cmd) => {
            eprintln!("error: unrecognized command `{cmd}`\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}
