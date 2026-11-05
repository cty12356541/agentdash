//! agentdash: agent progress dashboard CLI(W1-007 起 render/oneline/watch 全量接线)。

mod archive;
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
        // 波次归档与列表面(W15-05)
        Some("archive") => cmd_archive(&args[1..], lang),
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

/// `render` 参数解析(W6-002;W10-001 起多 PATH;W11-003 起 `--no-infer`;
/// W15-05 起 `--wave <id>`):`--format ansi|svg`(或 `--format=X`,缺省 ansi)+
/// `--no-infer` + `--wave <id>` + 任意个 [PATH]。`Err` 为已成型错误消息,调用方打印后退 2。
fn parse_render_args(
    rest: &[String],
    lang: lang::Lang,
) -> Result<(RenderFormat, Vec<PathBuf>, bool, Option<String>), String> {
    let mut format = RenderFormat::Ansi;
    let mut paths = Vec::new();
    let mut infer = true;
    let mut wave: Option<String> = None;
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
        } else if arg == "--wave" {
            let raw = iter
                .next()
                .map(String::as_str)
                .ok_or_else(|| lang.wave_invalid(lang.missing_value()))?;
            let valid = archive::validate_wave_id(raw).map_err(|_| lang.wave_invalid(raw))?;
            wave = Some(valid);
        } else if let Some(raw) = arg.strip_prefix("--wave=") {
            let valid = archive::validate_wave_id(raw).map_err(|_| {
                lang.wave_invalid(if raw.is_empty() {
                    lang.missing_value()
                } else {
                    raw
                })
            })?;
            wave = Some(valid);
        } else if arg == "--no-infer" {
            infer = false;
        } else if arg.starts_with('-') {
            return Err(lang.unexpected_flag(arg));
        } else {
            paths.push(PathBuf::from(arg));
        }
    }
    Ok((format, paths, infer, wave))
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
    let (format, paths, infer, wave) = match parse_render_args(&rest[1..], lang) {
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
        return cmd_render_panel(&paths, default_width, infer, wave.as_deref(), lang);
    }
    let path = paths
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from("."));
    let dash = match load_dashboard(&path, wave.as_deref(), infer, lang) {
        Ok(d) => d,
        Err(code) => return code,
    };
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

/// 根据可选 `--wave <id>` 加载活跃或历史归档 [`model::Dashboard`]。
fn load_dashboard(
    repo: &std::path::Path,
    wave: Option<&str>,
    infer: bool,
    lang: lang::Lang,
) -> Result<model::Dashboard, ExitCode> {
    if let Some(wave_id) = wave {
        match archive::resolve_wave_dir(repo, wave_id) {
            Ok(wave_dir) => Ok(model::merge_wave_opts(repo, &wave_dir, infer)),
            Err(archive::WaveError::WaveNotFound { wave, expected }) => {
                eprintln!("{}", lang.wave_not_found(&wave, &expected));
                Err(ExitCode::FAILURE)
            }
            Err(_) => Err(ExitCode::FAILURE),
        }
    } else {
        Ok(model::merge_opts(repo, infer))
    }
}

/// panel 多 PATH 入口(W10-001;W15-05 支持 `--wave <id>` 与 `waves/*` 过滤排序):逐参自研 glob 展开。
fn cmd_render_panel(
    paths: &[PathBuf],
    default_width: usize,
    infer: bool,
    wave: Option<&str>,
    lang: lang::Lang,
) -> ExitCode {
    let (raw_repos, nomatch) = glob::expand_args(paths);
    let mut repos: Vec<PathBuf> = raw_repos
        .into_iter()
        .filter(|p| !is_wave_auxiliary_entry(p))
        .collect();
    if repos.len() > 1 && repos.iter().all(|p| model::is_direct_wave_dir(p)) {
        repos.sort_by(|a, b| {
            let na = a.file_name().and_then(|s| s.to_str()).unwrap_or_default();
            let nb = b.file_name().and_then(|s| s.to_str()).unwrap_or_default();
            archive::natural_wave_cmp(na, nb)
        });
    }
    let multi_mode = repos.len() > 1;
    let mut dashboards = Vec::with_capacity(repos.len());
    for repo in &repos {
        match load_dashboard(repo, wave, infer, lang) {
            Ok(mut d) => {
                if multi_mode
                    && model::is_direct_wave_dir(repo)
                    && !d.milestones.iter().any(|m| !m.is_complete())
                    && let Some(ms) = d.milestones.first()
                {
                    let wave_label = ms
                        .wave
                        .as_deref()
                        .or_else(|| repo.file_name().and_then(|n| n.to_str()))
                        .unwrap_or("-");
                    d.project = format!("{} · {wave_label} {}", d.project, ms.title);
                }
                dashboards.push(d);
            }
            Err(code) => return code,
        }
    }
    match dashboards.len() {
        0 => {
            let lines: Vec<String> = nomatch
                .iter()
                .map(|pattern| nomatch_line(pattern))
                .collect();
            println!("{}", lines.join("\n"));
        }
        1 => {
            let dash = &dashboards[0];
            match tui::output_form(tui::stdout_cols(), default_width) {
                tui::OutputForm::OneLine => {
                    let mut lines = vec![render::render_oneline(dash)];
                    lines.extend(nomatch.iter().map(|pattern| nomatch_line(pattern)));
                    println!("{}", lines.join("\n"));
                }
                tui::OutputForm::Framed(width) => {
                    let mut blocks = vec![render::render_panel(dash, width)];
                    blocks.extend(nomatch.iter().map(|pattern| nomatch_line(pattern)));
                    println!("{}", blocks.join("\n\n"));
                }
            }
        }
        _ => match tui::output_form(tui::stdout_cols(), default_width) {
            tui::OutputForm::OneLine => {
                let mut lines: Vec<String> =
                    dashboards.iter().map(render::render_oneline).collect();
                lines.extend(nomatch.iter().map(|pattern| nomatch_line(pattern)));
                println!("{}", lines.join("\n"));
            }
            tui::OutputForm::Framed(width) => {
                let mut blocks: Vec<String> = dashboards
                    .iter()
                    .map(|dash| render::render_brief(dash, width))
                    .collect();
                blocks.extend(nomatch.iter().map(|pattern| nomatch_line(pattern)));
                println!("{}", blocks.join("\n\n"));
            }
        },
    }
    ExitCode::SUCCESS
}

/// 判定路径是否为 `.agentdash/waves/` 下的索引文件或临时目录(`index.json` / `.tmp-*` / `.old-*`),
/// 在 `render panel .agentdash/waves/*` 通配展开时予以过滤。
fn is_wave_auxiliary_entry(path: &std::path::Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    if parent.file_name().and_then(|n| n.to_str()) != Some("waves") {
        return false;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    name == "index.json" || name.starts_with('.') || !path.is_dir()
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
    let mut wave: Option<String> = None;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut iter = rest.iter();
    while let Some(arg) = iter.next() {
        if arg == "--strict" {
            strict = true;
        } else if arg == "--no-infer" {
            infer = false;
        } else if arg == "--wave" {
            let Some(raw) = iter.next().map(String::as_str) else {
                eprintln!(
                    "error: {}\n\n{}",
                    lang.wave_invalid(lang.missing_value()),
                    lang.usage()
                );
                return ExitCode::from(2);
            };
            if let Ok(valid) = archive::validate_wave_id(raw) {
                wave = Some(valid);
            } else {
                eprintln!("error: {}\n\n{}", lang.wave_invalid(raw), lang.usage());
                return ExitCode::from(2);
            }
        } else if let Some(raw) = arg.strip_prefix("--wave=") {
            if let Ok(valid) = archive::validate_wave_id(raw) {
                wave = Some(valid);
            } else {
                let shown = if raw.is_empty() {
                    lang.missing_value()
                } else {
                    raw
                };
                eprintln!("error: {}\n\n{}", lang.wave_invalid(shown), lang.usage());
                return ExitCode::from(2);
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
    let dash = match load_dashboard(&path, wave.as_deref(), infer, lang) {
        Ok(d) => d,
        Err(code) => return code,
    };
    println!("{}", render::render_digest(&dash));
    if strict && render::digest_needs_attention(&dash) {
        return ExitCode::FAILURE; // 1:失败门/blocked 在场(cron 监控用)
    }
    ExitCode::SUCCESS
}

/// `stats [--host <name>] [--wave <id>] [PATH]`(W11-004;W15-05 增 `--wave <id>`):
/// 观测面——宿主使用率 / gate 通过率 / 任务周转三张纯文本表。
fn cmd_stats(rest: &[String], lang: lang::Lang) -> ExitCode {
    let mut host: Option<String> = None;
    let mut wave: Option<String> = None;
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
        } else if arg == "--wave" {
            let Some(raw) = iter.next().map(String::as_str) else {
                eprintln!(
                    "error: {}\n\n{}",
                    lang.wave_invalid(lang.missing_value()),
                    lang.usage()
                );
                return ExitCode::from(2);
            };
            if let Ok(valid) = archive::validate_wave_id(raw) {
                wave = Some(valid);
            } else {
                eprintln!("error: {}\n\n{}", lang.wave_invalid(raw), lang.usage());
                return ExitCode::from(2);
            }
        } else if let Some(raw) = arg.strip_prefix("--wave=") {
            if let Ok(valid) = archive::validate_wave_id(raw) {
                wave = Some(valid);
            } else {
                let shown = if raw.is_empty() {
                    lang.missing_value()
                } else {
                    raw
                };
                eprintln!("error: {}\n\n{}", lang.wave_invalid(shown), lang.usage());
                return ExitCode::from(2);
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
    if let Some(wave_id) = wave.as_deref() {
        match archive::resolve_wave_dir(&path, wave_id) {
            Ok(wave_dir) => stats::run_dir(&wave_dir, host.as_deref()),
            Err(archive::WaveError::WaveNotFound { wave, expected }) => {
                eprintln!("{}", lang.wave_not_found(&wave, &expected));
                ExitCode::FAILURE
            }
            Err(_) => ExitCode::FAILURE,
        }
    } else {
        stats::run(&path, host.as_deref())
    }
}

/// `oneline [--no-infer] [--wave <id>] [PATH]`:无 ANSI 单行 statusline;
/// 旗标与多余参数报错退 2(W11-003 起 `--no-infer`,W15-05 起 `--wave <id>`)。
fn cmd_oneline(rest: &[String], lang: lang::Lang) -> ExitCode {
    let mut infer = true;
    let mut wave: Option<String> = None;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut iter = rest.iter();
    while let Some(arg) = iter.next() {
        if arg == "--no-infer" {
            infer = false;
        } else if arg == "--wave" {
            let Some(raw) = iter.next().map(String::as_str) else {
                eprintln!(
                    "error: {}\n\n{}",
                    lang.wave_invalid(lang.missing_value()),
                    lang.usage()
                );
                return ExitCode::from(2);
            };
            if let Ok(valid) = archive::validate_wave_id(raw) {
                wave = Some(valid);
            } else {
                eprintln!("error: {}\n\n{}", lang.wave_invalid(raw), lang.usage());
                return ExitCode::from(2);
            }
        } else if let Some(raw) = arg.strip_prefix("--wave=") {
            if let Ok(valid) = archive::validate_wave_id(raw) {
                wave = Some(valid);
            } else {
                let shown = if raw.is_empty() {
                    lang.missing_value()
                } else {
                    raw
                };
                eprintln!("error: {}\n\n{}", lang.wave_invalid(shown), lang.usage());
                return ExitCode::from(2);
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
    let dash = match load_dashboard(&path, wave.as_deref(), infer, lang) {
        Ok(d) => d,
        Err(code) => return code,
    };
    println!("{}", render::render_oneline(&dash));
    ExitCode::SUCCESS
}

/// `archive` 子命令模式(列表或封存,避免多布尔值结构)。
enum ArchiveCmdMode {
    List { json_out: bool },
    Seal(archive::ArchiveOptions),
}

/// 解析 `agentdash archive` 参数。
#[allow(clippy::too_many_lines)]
fn parse_archive_args(
    rest: &[String],
    lang: lang::Lang,
) -> Result<(ArchiveCmdMode, PathBuf), String> {
    let mut list_flag: Option<String> = None;
    let mut json_flag = false;
    let mut wave: Option<String> = None;
    let mut strict = archive::StrictPolicy::Permissive;
    let mut write_policy = archive::WritePolicy::CreateNew;
    let mut retention: Option<(archive::EventRetention, &'static str)> = None;
    let mut infer = true;
    let mut seal_flag_used: Option<String> = None;
    let mut paths: Vec<PathBuf> = Vec::new();

    let mut iter = rest.iter();
    while let Some(arg) = iter.next() {
        if arg == "--list" || arg == "-l" {
            list_flag = Some(arg.clone());
        } else if arg == "--json" {
            json_flag = true;
        } else if arg == "--strict" {
            strict = archive::StrictPolicy::Enforce;
            seal_flag_used = Some(arg.clone());
        } else if arg == "--force" {
            write_policy = archive::WritePolicy::ForceOverwrite;
            seal_flag_used = Some(arg.clone());
        } else if arg == "--prune-events" {
            if let Some((_, prev)) = retention
                && prev != "--prune-events"
            {
                return Err(lang.incompatible_flags(prev, "--prune-events"));
            }
            retention = Some((archive::EventRetention::Prune, "--prune-events"));
            seal_flag_used = Some(arg.clone());
        } else if arg == "--keep-events" {
            if let Some((_, prev)) = retention
                && prev != "--keep-events"
            {
                return Err(lang.incompatible_flags(prev, "--keep-events"));
            }
            retention = Some((archive::EventRetention::Keep, "--keep-events"));
            seal_flag_used = Some(arg.clone());
        } else if arg == "--no-infer" {
            infer = false;
            seal_flag_used = Some(arg.clone());
        } else if arg == "--wave" {
            let Some(raw) = iter.next() else {
                return Err(lang.wave_invalid(lang.missing_value()));
            };
            let valid = archive::validate_wave_id(raw).map_err(|_| lang.wave_invalid(raw))?;
            wave = Some(valid);
            seal_flag_used = Some("--wave".to_owned());
        } else if let Some(raw) = arg.strip_prefix("--wave=") {
            let valid = archive::validate_wave_id(raw).map_err(|_| {
                lang.wave_invalid(if raw.is_empty() {
                    lang.missing_value()
                } else {
                    raw
                })
            })?;
            wave = Some(valid);
            seal_flag_used = Some("--wave".to_owned());
        } else if arg.starts_with('-') {
            return Err(lang.unexpected_flag(arg));
        } else {
            paths.push(PathBuf::from(arg));
        }
    }

    if list_flag.is_none()
        && paths.first().and_then(|p| p.to_str()) == Some("list")
        && (paths.len() > 1
            || json_flag
            || (seal_flag_used.is_none()
                && !std::path::Path::new("list").join(".agentdash").exists()))
    {
        paths.remove(0);
        list_flag = Some("list".to_owned());
    }

    if let Some(ref lf) = list_flag
        && let Some(ref sf) = seal_flag_used
    {
        return Err(lang.incompatible_flags(lf, sf));
    }
    if json_flag && list_flag.is_none() {
        return Err(lang.unexpected_flag("--json"));
    }
    if paths.len() > 1 {
        return Err(lang.unexpected_extra("[PATH]"));
    }
    let path = paths.pop().unwrap_or_else(|| PathBuf::from("."));

    if list_flag.is_some() {
        Ok((
            ArchiveCmdMode::List {
                json_out: json_flag,
            },
            path,
        ))
    } else {
        Ok((
            ArchiveCmdMode::Seal(archive::ArchiveOptions {
                wave_override: wave,
                strict,
                write_policy,
                retention: retention.map_or(archive::EventRetention::Keep, |(r, _)| r),
                infer,
            }),
            path,
        ))
    }
}

/// `agentdash archive` 子命令入口(W15-05)。
fn cmd_archive(rest: &[String], lang: lang::Lang) -> ExitCode {
    let (mode, path) = match parse_archive_args(rest, lang) {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("error: {msg}\n\n{}", lang.usage());
            return ExitCode::from(2);
        }
    };
    match mode {
        ArchiveCmdMode::List { json_out } => {
            let index = archive::list_waves(&path);
            if json_out {
                if let Ok(pretty) = serde_json::to_string_pretty(&index) {
                    println!("{pretty}");
                }
            } else {
                println!(
                    "{}",
                    archive::render_wave_list(&index, lang.archive_empty_list())
                );
            }
            ExitCode::SUCCESS
        }
        ArchiveCmdMode::Seal(opts) => match archive::archive_wave(&path, &opts) {
            Ok(receipt) => {
                println!(
                    "{}",
                    lang.archive_receipt(
                        &receipt.wave,
                        receipt.tasks_done,
                        receipt.tasks_total,
                        receipt.events_count
                    )
                );
                ExitCode::SUCCESS
            }
            Err(archive::WaveError::MissingLedger(p)) => {
                eprintln!("{}", lang.archive_missing_ledger(&p));
                ExitCode::FAILURE
            }
            Err(archive::WaveError::CorruptLedger(why)) => {
                eprintln!("{}", lang.archive_corrupt_ledger(&why));
                ExitCode::FAILURE
            }
            Err(archive::WaveError::MissingWaveId) => {
                eprintln!("{}", lang.archive_missing_wave_id());
                ExitCode::FAILURE
            }
            Err(archive::WaveError::InvalidWaveId(bad)) => {
                eprintln!("error: {}", lang.wave_invalid(&bad));
                ExitCode::FAILURE
            }
            Err(archive::WaveError::AlreadyExists(wave)) => {
                eprintln!("{}", lang.archive_exists(&wave));
                ExitCode::FAILURE
            }
            Err(archive::WaveError::StrictFailed(reasons)) => {
                eprintln!("{}", lang.archive_strict_failed(&reasons));
                ExitCode::FAILURE
            }
            Err(archive::WaveError::LockTimeout(p)) => {
                eprintln!("{}", lang.archive_lock_timeout(&p));
                ExitCode::FAILURE
            }
            Err(archive::WaveError::Io(why)) => {
                eprintln!("{}", lang.archive_io_error(&why));
                ExitCode::FAILURE
            }
            Err(archive::WaveError::WaveNotFound { wave, expected }) => {
                eprintln!("{}", lang.wave_not_found(&wave, &expected));
                ExitCode::FAILURE
            }
        },
    }
}

/// `watch [--once] [--no-infer] [SECONDS] [PATH]`:ratatui watch;位置参数解析收口在
/// [`tui::parse_watch_args`](W2-008 恢复 interval 位置档;W14-06 起透传 `--no-infer`);
/// `--once` 渲染一帧即退,非 tty stdin 同样自动退化为单帧。
fn cmd_watch(rest: &[String], lang: lang::Lang) -> ExitCode {
    match tui::parse_watch_args(rest) {
        Ok((once, interval, repo, infer)) => {
            let outcome = if once {
                tui::watch_once(&repo, infer)
            } else {
                tui::watch(&repo, interval, infer)
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
