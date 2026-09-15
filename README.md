# agentdash

agent 进度仪表盘:从任务台账、会话事件流与 git 快照三个数据源投影出任务 DAG、
验证门状态与里程碑进度——纯只读、降级不失败、零 Python/Node 运行时前置。

Rust 单 crate 实现(ratatui + crossterm / serde_json,stable ≥1.85,见
`rust-toolchain.toml`),通用任务契约 v1 见 `schema/agentdash.tasklog.v1.json`。

## 安装

```bash
# 源码安装(进入本仓库根目录)
cargo install --path .
agentdash --version   # 自检
```

### 插件市场(Claude Code)

```bash
claude plugin marketplace add cty12356541/agentdash
claude plugin install agentdash@agentdash-marketplace
agentdash --version   # 自检:hook 直调二进制,必须已在 PATH
```

市场安装只装 hooks + skill(插件体 `kits/claude-code/`),**不内嵌二进制**——
hook 运行时直调 PATH 上的 `agentdash`;缺失时先补二进制(`cargo install --path .`
或从 GitHub Releases 下载放入 PATH,Windows release 资产后续手动挂)。预构建
分发策略见 `bin/README.md`,集成包细节见 `kits/claude-code/README.md`。

### claude-code 集成包(可选)

把 Claude Code 会话事件接进 agentdash:

```bash
# 类 Unix(在仓库根目录)
./kits/claude-code/install.sh [目标项目目录]
# Windows PowerShell
.\kits\claude-code\install.ps1 [-Target <目标项目目录>]
```

安装器幂等注册四个 hook(PostToolUse / PreToolUse / Stop / SubagentStop;
PreToolUse 以 matcher `Task|Agent` 只拦 agent 派发),命令直调二进制
`agentdash hook <event> || true`——**无路径 baked、无任何脚本运行时依赖**;
hook 自身任何失败静默退出 0,绝不阻塞会话。插件市场分发直接用
`kits/claude-code/hooks/hooks.json`。详见 `kits/claude-code/README.md`。

本仓另附开发用侧栏启动器 `dash-side.bat <项目目录>`(Windows Terminal 左右
分屏:左 cmd、右 `agentdash watch` 常驻 TUI)。

## 命令

| 命令 | 作用 |
|---|---|
| `agentdash render panel [PATH]` | 终端面板:页眉统计 → 健康(验证门/警告)→ 轨迹(里程碑进度条)→ 车道任务 |
| `agentdash render graph [PATH]` | 任务 DAG 字符图(同车道链 + 屏障边,拓扑分层布局) |
| `agentdash oneline [PATH]` | 无 ANSI 单行 statusline:`[dash] <project> ✓d▶a·r ⚑s ·nag` |
| `agentdash watch [--once] [PATH]` | 常驻 TUI(5s 刷新档,git 快照 30s 节流;`q`/Ctrl-C 退出);`--once` 或非 tty stdin 渲染一帧即退 |
| `agentdash hook <EVENT>` | 消费宿主 hook 载荷(stdin),折叠后追加 `.agentdash/events.jsonl` |

`[PATH]` 缺省 `.`;渲染宽度非 tty 用默认、tty 按终端列钳 40..120,
窄于 40 列时框化视图必破图,`render`/`watch` 自动退化为 oneline 单行。

## 数据契约 v1(两文件)

约定目录 `<repo>/.agentdash/`;合并可信序 **契约 > 事件 > git 快照**,
任何单源缺失/损坏只降级为警告行,不失败、不白屏。

| 文件 | 写入方 | 内容 |
|---|---|---|
| `ledger.json` | agent 或人 | 任务台账:`wave` / `title`(必填)/ `profile` / `lanes` / `tasks`(label+state)/ `barriers`(after→unlocks)/ `milestones`(可选声明式分组:被引用任务按组归并、同任务首见为准,余者入「未分组」尾组;缺省由 wave+全任务合成单里程碑)/ `note`(可选根级波次注记:自由文本,内核按未知字段忽略语义容忍,供人读/流程留痕)。状态机最小核 `pending→active→done`,终态 `blocked`;`review`/`fix-round` 富态仅在声明 `profile` 后有效,否则自动降级并记警告。严格校验面:`schema/agentdash.tasklog.v1.json` |
| `events.jsonl` | `agentdash hook` | 会话事件流:`gate`(running→passed/failed,退出码不可知按 failed 折叠,验证门自动登记)、`tool`、`agent`;多进程并发追加经文件锁保证零丢失 |

三源全无时输出空态引导文案;仅 git 仓无契约时降级为最近提交伪任务单链
(恒 `pending`),oneline 照常可用——降级同时出一条 `⚠ missing ledger.json`
警告行(其余源在场而台账缺失,同理告警)。

## 路线

- **一期(W1–W3,0.2.0)**:Rust 内核(契约/事件/git 源/合并/渲染/TUI)+ claude-code-kit + gate 事件 + oneline + 三平台 CI(ubuntu / windows / macos);W2 交互跃迁(任务详情/多波次/过滤/台账写回/gh 远程源);W3 收口:多里程碑分组、面板屏障行、PreToolUse dispatched 入口、事件窗速度线。
- **一期清偿(W4,0.3.0)**:评估清偿——gate 折叠保守化(exit 不可知记 failed,不虚报)、详情面板事件尾上板、渲染截断/子进程执行器双收敛、手搓解析器 proptest 性质面、Release 自动化(tag 触发四 triple 资产)。
- **二期**:codex-kit / opencode-kit(宿主扩展)、远程源扩展(PR / CI 缓存)、观察者兼底完善。
- **三期**:分发矩阵铺满(Release 二进制 / 包管理器 / 插件市场)。

## 与 claude-dash 的关系

claude-dash 是已**冻结的参考实现**:agentdash 的渲染与单行算法移植自它,
以 claude-dash 76 断言套件的实测语义为准(本仓移植黄金断言 21 条);
它不再演进,新能力一律在 agentdash 落地。两者数据文件互不通用。
