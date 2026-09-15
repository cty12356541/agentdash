//! CLI 文案语言(W9-005):中英双语字符串表,`AGENTDASH_LANG` 切换,缺省中文。
//!
//! 取值容忍区域后缀(`zh-CN`/`en_US` 等,前缀判定):`en*` → 英文;
//! `zh*`/未设置/未知值一律回落中文(仓库母语;绝不因语言配置失败阻塞命令,
//! 与 hook 降级铁律同源)。文案覆盖 `--help` 用法全文与 main 层参数错误;
//! 渲染词表(panel/oneline/graph)本就中文,不经此表。

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
  oneline [PATH]                  打印单行状态摘要
  watch [--once] [SECONDS] [PATH] 常驻实时刷新(q/Ctrl-C 退出)
  hook <EVENT>                    从 stdin 消费宿主工具钩子载荷

参数:
  [PATH]        项目或台账目录 [默认: .]
  [SECONDS]     watch 刷新间隔,钳制在 1..3600 [默认: 5]
  watch --once  只渲染一帧即退(stdin 非 TTY 时自动生效)

选项:
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
  oneline [PATH]                  Print a one-line status summary
  watch [--once] [SECONDS] [PATH] Watch a plan and refresh live (q/Ctrl-C quits)
  hook <EVENT>                    Consume a host-tool hook payload from stdin

Arguments:
  [PATH]        Path to the project or plan directory [default: .]
  [SECONDS]     Watch refresh interval, clamped to 1..3600 [default: 5]
  watch --once  Render a single frame and exit (implied when stdin is not a TTY)

Options:
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
            Lang::Zh => "error: render 需要视图:`render panel|graph [PATH]`",
            Lang::En => "error: render needs a view: `render panel|graph [PATH]`",
        }
    }

    /// 未知 render 视图。
    pub(crate) fn unknown_view(self, view: &str) -> String {
        match self {
            Lang::Zh => format!("error: 未知的 render 视图 `{view}`(应为 panel|graph)"),
            Lang::En => format!("error: unknown render view `{view}` (expected panel|graph)"),
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
}
