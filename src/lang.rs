//! CLI 文案语言(W9-005):中英双语字符串表,`AGENTDASH_LANG` 切换,缺省中文。
//!
//! 取值容忍区域后缀(`zh-CN`/`en_US` 等,前缀判定):`en*` → 英文;
//! `zh*`/未设置/未知值一律回落中文(仓库母语;绝不因语言配置失败阻塞命令,
//! 与 hook 降级铁律同源)。文案覆盖 `--help` 用法全文与 main 层参数错误;
//! 渲染词表(panel/oneline/graph)本就中文,不经此表。

use std::path::Path;

/// CLI 文案语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lang {
    /// 中文(缺省)。
    Zh,
    /// English(显式 `AGENTDASH_LANG=en`)。
    En,
}

/// 语言码解析:入口收口为 `Option<&str>` 便于直测,环境读取只在 [`current`]。
pub(crate) fn resolve(code: Option<&str>) -> Lang {
    match code {
        Some(c) if c.trim().to_ascii_lowercase().starts_with("en") => Lang::En,
        _ => Lang::Zh,
    }
}

/// 当前语言:读 `AGENTDASH_LANG`,缺省中文。
pub(crate) fn current() -> Lang {
    resolve(std::env::var("AGENTDASH_LANG").ok().as_deref())
}

impl Lang {
    /// `--help`/无参数用法全文(两语言版,尾部互附切换提示)。
    pub(crate) fn usage(self) -> &'static str {
        match self {
            Lang::Zh => {
                "\
agentdash - agent 进度仪表盘(agent progress dashboard)

用法: agentdash <命令> [参数]

命令:
  render panel|graph [--format ansi|svg] [PATH]
                                  渲染面板或任务 DAG(--format svg:矢量 DAG,仅 graph)
  render digest [--strict] [PATH] 离场摘要(纯文本;--strict 遇失败门、阻塞或告警退 1)
  oneline [PATH]                  打印单行状态摘要
  stats [PATH] [--host <name>]    观测统计:宿主/gate/周转三表(纯文本,恒退 0)
  archive [--list] [--wave <id>] [--strict] [--force] [PATH]
                                  封存当前波次到 .agentdash/waves/<id>/ 或列出归档
  watch [--once] [SECONDS] [PATH] 常驻实时刷新(q/Ctrl-C 退出)
  hook <EVENT>                    从 stdin 消费宿主工具钩子载荷

参数:
  [PATH]        项目或台账目录 [默认: .]
  [SECONDS]     watch 刷新间隔,钳制在 1..3600 [默认: 5]
  watch --once  只渲染一帧即退(stdin 非 TTY 时自动生效)

选项:
  --wave <id>    指定归档波次 ID(render/oneline/stats/archive 系)
  --no-infer     关闭无 who completed 配对启发(render/oneline/watch 系;缺省开启推断)
  -V, --version  打印版本号
  -h, --help     打印本帮助

Language/语言: 中文 · AGENTDASH_LANG=en 切换英文(switch to English)"
            }
            Lang::En => {
                "\
agentdash - agent progress dashboard

Usage: agentdash <COMMAND> [ARGS]

Commands:
  render panel|graph [--format ansi|svg] [PATH]
                                  Render the dashboard panel or the task DAG
                                  (--format svg: vector DAG, graph only)
  render digest [--strict] [PATH] Print a plain-text leave digest (--strict
                                  exits 1 on failed gate, blocked task, or warning)
  oneline [PATH]                  Print a one-line status summary
  stats [PATH] [--host <name>]    Observation tables: hosts / gates / turnover
                                  (plain text, always exits 0)
  archive [--list] [--wave <id>] [--strict] [--force] [PATH]
                                  Archive active wave to .agentdash/waves/<id>/ or list waves
  watch [--once] [SECONDS] [PATH] Watch a plan and refresh live (q/Ctrl-C quits)
  hook <EVENT>                    Consume a host-tool hook payload from stdin

Arguments:
  [PATH]        Path to the project or plan directory [default: .]
  [SECONDS]     Watch refresh interval, clamped to 1..3600 [default: 5]
  watch --once  Render a single frame and exit (implied when stdin is not a TTY)

Options:
  --wave <id>    Select archived wave ID (render/oneline/stats/archive family)
  --no-infer     Disable no-who completed pairing inference (render/oneline/watch
                 family; inference is on by default)
  -V, --version  Print version
  -h, --help     Print help

Language/语言: English · AGENTDASH_LANG=zh 切换中文(switch to Chinese)"
            }
        }
    }

    /// 未知子命令(退 2)。
    pub(crate) fn unrecognized_command(self, cmd: &str) -> String {
        match self {
            Lang::Zh => format!("error: 无法识别的命令 `{cmd}`"),
            Lang::En => format!("error: unrecognized command `{cmd}`"),
        }
    }

    /// render 缺视图。
    pub(crate) fn render_needs_view(self) -> &'static str {
        match self {
            Lang::Zh => "error: render 需要视图:`render panel|graph|digest [PATH]`",
            Lang::En => "error: render needs a view: `render panel|graph|digest [PATH]`",
        }
    }

    /// 未知 render 视图。
    pub(crate) fn unknown_view(self, view: &str) -> String {
        match self {
            Lang::Zh => format!("error: 未知的 render 视图 `{view}`(应为 panel|graph|digest)"),
            Lang::En => {
                format!("error: unknown render view `{view}` (expected panel|graph|digest)")
            }
        }
    }

    /// `--format` 取值非法;`got` 为调用方预成型的实际值串(缺参显示占位)。
    pub(crate) fn format_invalid(self, got: &str) -> String {
        match self {
            Lang::Zh => format!("--format 需要 ansi|svg,得到 {got}"),
            Lang::En => format!("--format needs ansi|svg, got {got}"),
        }
    }

    /// 意外旗标。
    pub(crate) fn unexpected_flag(self, flag: &str) -> String {
        match self {
            Lang::Zh => format!("意外的旗标 `{flag}`"),
            Lang::En => format!("unexpected flag `{flag}`"),
        }
    }

    /// 位置参数多余(main 层 PATH / tui 层 SECONDS+PATH 两处复用)。
    pub(crate) fn unexpected_extra(self, after: &str) -> String {
        match self {
            Lang::Zh => format!("{after} 之后出现多余参数"),
            Lang::En => format!("unexpected extra arguments after {after}"),
        }
    }

    /// svg 格式只对 graph 合法。
    pub(crate) fn svg_only_graph(self) -> &'static str {
        match self {
            Lang::Zh => "error: --format svg 仅支持 `render graph`",
            Lang::En => "error: --format svg is only supported by `render graph`",
        }
    }

    /// watch 运行失败前缀(`{err}` 由调用方拼)。
    pub(crate) fn watch_failed(self) -> &'static str {
        match self {
            Lang::Zh => "watch 失败:",
            Lang::En => "watch failed:",
        }
    }

    /// `--format` 缺参占位(两语言各自的实际值显示)。
    pub(crate) fn missing_value(self) -> &'static str {
        match self {
            Lang::Zh => "(缺参)",
            Lang::En => "(missing)",
        }
    }

    /// `--host` 缺参/空值(W11-004 stats;`got` 为调用方预成型的实际值串)。
    pub(crate) fn host_needs_name(self, got: &str) -> String {
        match self {
            Lang::Zh => format!("--host 需要宿主名,得到 {got}"),
            Lang::En => format!("--host needs a host name, got {got}"),
        }
    }

    /// `--wave` 取值非法或缺失(W15-05)。
    pub(crate) fn wave_invalid(self, got: &str) -> String {
        match self {
            Lang::Zh => {
                format!("--wave 需要合法波次 ID([A-Za-z0-9][A-Za-z0-9._-]{{0,63}}),得到 {got}")
            }
            Lang::En => format!(
                "--wave needs a valid wave ID ([A-Za-z0-9][A-Za-z0-9._-]{{0,63}}), got {got}"
            ),
        }
    }

    /// 互斥旗标报错(W15-05)。
    pub(crate) fn incompatible_flags(self, a: &str, b: &str) -> String {
        match self {
            Lang::Zh => format!("旗标 `{a}` 与 `{b}` 互斥"),
            Lang::En => format!("flags `{a}` and `{b}` cannot be used together"),
        }
    }

    /// 历史波次未找到(退 1)。
    pub(crate) fn wave_not_found(self, wave: &str, path: &Path) -> String {
        match self {
            Lang::Zh => format!("error: 未找到归档波次 `{wave}`(期望 {})", path.display()),
            Lang::En => format!(
                "error: archived wave `{wave}` not found (expected {})",
                path.display()
            ),
        }
    }

    /// 归档时台账缺失(退 1)。
    pub(crate) fn archive_missing_ledger(self, path: &Path) -> String {
        match self {
            Lang::Zh => format!("error: 归档失败:未找到台账 {}", path.display()),
            Lang::En => format!(
                "error: archive failed: ledger not found at {}",
                path.display()
            ),
        }
    }

    /// 归档时台账损坏(退 1)。
    pub(crate) fn archive_corrupt_ledger(self, why: &str) -> String {
        match self {
            Lang::Zh => format!("error: 归档失败:台账损坏 ({why})"),
            Lang::En => format!("error: archive failed: corrupt ledger ({why})"),
        }
    }

    /// 归档时缺少波次 ID(退 1)。
    pub(crate) fn archive_missing_wave_id(self) -> &'static str {
        match self {
            Lang::Zh => "error: 归档需要波次 ID:请在 ledger.json 声明 \"wave\" 或传入 --wave <id>",
            Lang::En => {
                "error: archive requires a wave ID: set \"wave\" in ledger.json or pass --wave <id>"
            }
        }
    }

    /// 归档波次已存在(退 1)。
    pub(crate) fn archive_exists(self, wave: &str) -> String {
        match self {
            Lang::Zh => format!("error: 归档波次 `{wave}` 已存在(传入 --force 可覆盖)"),
            Lang::En => {
                format!("error: archived wave `{wave}` already exists (pass --force to overwrite)")
            }
        }
    }

    /// `--strict` 归档门禁未通过(退 1)。
    pub(crate) fn archive_strict_failed(self, reasons: &[String]) -> String {
        match self {
            Lang::Zh => format!(
                "error: --strict 归档门禁未通过:\n  - {}",
                reasons.join("\n  - ")
            ),
            Lang::En => format!(
                "error: --strict archive check failed:\n  - {}",
                reasons.join("\n  - ")
            ),
        }
    }

    /// 归档锁超时(退 1)。
    pub(crate) fn archive_lock_timeout(self, path: &Path) -> String {
        match self {
            Lang::Zh => format!("error: 归档无法获取文件锁 {}", path.display()),
            Lang::En => format!(
                "error: archive timed out waiting for lock {}",
                path.display()
            ),
        }
    }

    /// 归档 IO 失败(退 1)。
    pub(crate) fn archive_io_error(self, why: &str) -> String {
        match self {
            Lang::Zh => format!("error: 归档读写失败: {why}"),
            Lang::En => format!("error: archive I/O failed: {why}"),
        }
    }

    /// 归档成功一行回执(stdout)。
    pub(crate) fn archive_receipt(
        self,
        wave: &str,
        done: usize,
        total: usize,
        events: usize,
    ) -> String {
        match self {
            Lang::Zh => format!(
                "已归档 {wave} -> .agentdash/waves/{wave} ({done}/{total} done, {events} 条事件)"
            ),
            Lang::En => format!(
                "archived {wave} -> .agentdash/waves/{wave} ({done}/{total} done, {events} events)"
            ),
        }
    }

    /// 归档列表空态(stdout)。
    pub(crate) fn archive_empty_list(self) -> &'static str {
        match self {
            Lang::Zh => "archive: 暂无归档波次",
            Lang::En => "archive: no archived waves",
        }
    }
}
