//! 集成包钩子入口:stdin JSON → .agentdash/events.jsonl 追加(W1-008b 车道实现,占位桩)。

use std::process::ExitCode;

pub fn run(_event: Option<&str>) -> ExitCode {
    eprintln!("hook: not implemented in W1-008b");
    ExitCode::SUCCESS
}
