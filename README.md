# agentdash

agent 进度仪表盘:从任务台账、会话事件流与 git 快照三个数据源投影出任务 DAG、
验证门状态与里程碑进度——纯只读、降级不失败、零 Python/Node 运行时前置。

Rust 单 crate 实现(ratatui + crossterm / serde_json,stable ≥**1.88**——源码使用
let-chains,见 `rust-toolchain.toml`),通用任务契约 v1 见 `schema/agentdash.tasklog.v1.json`。

## 安装

```bash
# 源码安装(进入本仓库根目录)
cargo install --path .
agentdash --version   # 自检
```

### CLI 语言(中英双语,0.7.0 起)

帮助与参数错误文案默认**中文**;`AGENTDASH_LANG=en` 切换英文(容忍
`en-US`/`en_US` 区域后缀;`zh*`/未知值回落中文)。两语言 `--help` 尾部
互附对方切换提示。

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

### 宿主矩阵(多工具混用,0.6.0 起)

| 宿主 | kit | 机制 | 在跑 agent 面板 |
|---|---|---|---|
| Claude Code | `kits/claude-code`(插件市场) | hooks.json 五事件(含 `PostToolUseFailure`) | ✓ |
| Codex CLI | `kits/codex` | `.codex/hooks.json`(repo 级;需 trust + `/hooks` 一次性审查) | ✓ |
| opencode | `kits/opencode` | `.opencode/plugins/agentdash.js`(Bun 插件) | 待宿主子代理事件 |
| ZCode(公司内部) | `kits/zcode` | `.zcode/config.json` → hooks(`enabled:true`,安装器置位;会话启动加载)。⚠ hook 生命周期串行阻塞(需上一 hook 回复),兼容暂时搁置(2026-09-16 维护者实测) | 待宿主子代理事件 |
| Cursor | `kits/cursor` | `.cursor/hooks.json`(`version:1`;项目级需信任工作区) | ✓(`subagentStart/Stop` 原生) |
| DeepSeek Harness | `kits/deepseek` | DSH 桥 `dsh-hooks-claude-code` 挂载,`configPath` 指向 `.deepseek/agentdash-hooks.json`(进程级,启动时读取) | ✓(桥原生 `SubagentStart/Stop`) |

现状(2026-09-16):**5 宿主活跃,zcode 1 家暂时搁置**(行内 ⚠ caveat);搁置是
兼容推进暂停,不动契约——六 kit 同写 `<repo>/.agentdash/` 的语义不变。

**一致性保证**:六套 kit 写同一 `<repo>/.agentdash/`(同文件、同锁、同事件词表),
事件带 `host` 字段归因,面板在跑行显示 `▶ <who> [host]`;harness 约定单源
`kits/shared/AGENTDASH.md`,各安装器以标记段幂等合入宿主指令文件
(Claude Code → `CLAUDE.md`,codex / opencode / zcode / cursor / deepseek →
`AGENTS.md`)——换工具不换契约。Cursor 载荷词表(`afterShellExecution` 顶层
`command`+`output`)由 hook 二进制归一成 bash 视图。**gate 终态证据按宿主
载荷能力如实分级(W11-002 真机实测入册)**:deepseek(DSH 桥字符串回执带
`[exit code: N]` 尾契约)绿门禁折叠真实退出码;claude-code 的 PostToolUse
`tool_response` 仅 `stdout/stderr/interrupted/isImage/noOutputExpected`
五键、**无退出码字段**(W11-002 捕获原文,见 reports),绿门禁只能如实折叠
`failed (exit unknown)`,不臆造 0;失败侧 `PostToolUseFailure` 事件(W9-003;
zcode/cursor/claude-code 注册)与在途 gate 配对后升级真证据——claude 真码在
顶层 `error` 串头 `Exit code N`(解析非零码入账,缺码落 1、中断 130),不再挂尾注。

**一键巡检**:`bash scripts/verify-kits.sh` 在临时目录对六 kit 各跑一遍安装
体验证(全新安装产物 → 已装钩子命令端到端事件落盘与 host 归属 → 二次安装
幂等 → 既有内容保留:用户键/他人钩子/损坏 JSON 备份重建/粘行防护);
`--keep` 保留现场供人工检查。jq 缺失时 JSON 类断言自动降级 SKIP,与安装器
的无-jq 降级语义一致。改动 `src/` 后先 `cargo install --path .` 重装再巡检
(巡检的是 PATH 上已装二进制)。

### claude-code 集成包(可选)

把 Claude Code 会话事件接进 agentdash:

```bash
# 类 Unix(在仓库根目录)
./kits/claude-code/install.sh [目标项目目录]
# Windows PowerShell
.\kits\claude-code\install.ps1 [-Target <目标项目目录>]
```

安装器幂等注册五个 hook(PostToolUse / PostToolUseFailure / PreToolUse / Stop / SubagentStop;
PreToolUse 以 matcher `Task|Agent` 只拦 agent 派发),命令直调二进制
`agentdash hook <event> || true`——**无路径 baked、无任何脚本运行时依赖**;
hook 自身任何失败静默退出 0,绝不阻塞会话。插件市场分发直接用
`kits/claude-code/hooks/hooks.json`。详见 `kits/claude-code/README.md`。

本仓另附开发用侧栏启动器 `dash-side.bat <项目目录>`(Windows Terminal 左右
分屏:左 cmd、右 `agentdash watch` 常驻 TUI)。

## 命令

| 命令 | 作用 |
|---|---|
| `agentdash render panel [--no-infer] [PATH]` | 终端面板:页眉统计 → 健康(验证门/警告)→ 轨迹(里程碑进度条)→ 车道任务 |
| `agentdash render graph [--format ansi|svg] [--no-infer] [PATH]` | 任务 DAG 字符图(同车道链 + 屏障边,拓扑分层布局);`--format svg` 出矢量文档 |
| `agentdash render digest [--strict] [--no-infer] [PATH]` | 离场摘要:纯文本无 ANSI(失败门/在跑 agent/blocked/done 任务/⚠);`--strict` 存在失败门或 blocked 时退 1(cron 夜间监控);源降级 ⚠ 警告不计入——`--strict` 不校验源降级状态,缺台账/事件等降级仍退 0 |
| `agentdash oneline [--no-infer] [PATH]` | 无 ANSI 单行 statusline:`[dash] <project> ✓d▶a·r ⚑s ·nag` |
| `agentdash watch [--once] [PATH]` | 常驻 TUI(5s 刷新档,git 快照 30s 节流;`q`/Ctrl-C 退出);`--once` 或非 tty stdin 渲染一帧即退 |
| `agentdash stats [--host <name>] [PATH]` | 观测统计:宿主使用率/gate 通过率/任务周转三张纯文本表(零 ANSI,恒退 0;`--host` 过滤宿主表) |
| `agentdash hook <EVENT>` | 消费宿主 hook 载荷(stdin),折叠后追加 `.agentdash/events.jsonl` |

`[PATH]` 缺省 `.`;渲染宽度非 tty 用默认、tty 按终端列钳 40..120,
窄于 40 列时框化视图必破图,`render`/`watch` 自动退化为 oneline 单行。

**无 who completed 配对启发(W11-003)**:宿主载荷缺 `who` 的 completed
事件(部分宿主子代理停止通知天然不带身份),回放层缺省配给**最老在跑**的
dispatched 行(FIFO);配对行在 panel/摘要/精要中显式标注
`▶⇢✓ <who> … (inferred)`(完成绿),计数走完成侧——不占 `N agents` /
`·nag` 在跑口径;配对成功的逐行 `missing who` 警告降频为回放收尾一条
`⚠ N 个无 who completed 已推断配对` 汇总(无在跑可配的仍逐行丢弃警告;
配对是启发推断,不是实测配对)。`--no-infer`(panel/graph/digest/oneline
支持)关闭启发回到严格丢弃 + 逐行警告,行为与启发落地前逐字节一致;
`watch` 常驻视图暂不支持该旗标(恒缺省开启,见 W11-003 报告)。

## 数据契约 v1(两文件 + config.json)

**契约公开版 v1.0 全文见 [`docs/contract.md`](docs/contract.md)**(全词表/字段语义/
折叠与会话池/推断配对/证据分级,第三方宿主照写即可);本节为速览。

约定目录 `<repo>/.agentdash/`;合并可信序 **契约 > 事件 > git 快照**,
任何单源缺失/损坏只降级为警告行,不失败、不白屏。

| 文件 | 写入方 | 内容 |
|---|---|---|
| `ledger.json` | agent 或人 | 任务台账:`wave` / `title`(必填)/ `profile` / `lanes` / `tasks`(label+state;可选 `done_at` 完成自报时刻,写回 `d` 键自动盖章)/ `barriers`(after→unlocks)/ `milestones`(可选声明式分组:被引用任务按组归并、同任务首见为准,余者入「未分组」尾组;缺省由 wave+全任务合成单里程碑)/ `note`(可选根级波次注记:自由文本,内核按未知字段忽略语义容忍,供人读/流程留痕)。状态机最小核 `pending→active→done`,终态 `blocked`;`review`/`fix-round` 富态仅在声明 `profile` 后有效,否则自动降级并记警告。物证核对:done 任务自带 `done_at` 且事件窗在场时,自报晚于最近通过门(或窗内无通过门)打 `?`(自报无物证);严格校验面:`schema/agentdash.tasklog.v1.json` |
| `events.jsonl` | `agentdash hook` | 会话事件流:`gate`(running→passed/failed,退出码不可知按 failed 折叠,验证门自动登记)、`tool`、`agent`;多进程并发追加经文件锁保证零丢失 |
| `config.json` | 人或 agent(可选) | 用户自定义 gate 词表(W10-003,见下) |

**用户自定义 gate**(W10-003;取代 v1 设计散文中的 `config.toml` 提法):
`.agentdash/config.json` 声明验证门词表,验证门从内置六条(cargo test/clippy/fmt、
go test、npm test、gh pr checks)开放——

```json
{"gates": [{"name": "pytest", "words": ["pytest"]}, {"name": "make-check", "words": ["make", "check"]}]}
```

自定义词序列与内置门走**同一匹配机制**(连续词序列 + 词边界,不引入正则),
**用户表先于内置表匹配**(first-match-wins,与内置同名即覆盖)。校验:`name`
限 `[a-z0-9-_]+` 且截 40 字符(超长截断、字符集越界才裁);`words` 1..=8 个
非空词;至多 32 条。任一形状错/越界(含文件损坏、超 64KB)→ **整表静默回退
内置**(降级铁律:零警告、零事件,hook 照常退 0);文件缺失 = 仅内置;
`"gates": []` = 合法空表。配置按每次 hook 调用现读、改完即生效(无缓存无热
重载概念);六宿主 kit 共用同一二进制,自动受益。

三源全无时输出空态引导文案;仅 git 仓无契约时降级为最近提交伪任务单链
(恒 `pending`),oneline 照常可用——降级同时出一条 `⚠ missing ledger.json`
警告行(其余源在场而台账缺失,同理告警)。

## 路线

- **一期(W1–W3,0.2.0)**:Rust 内核(契约/事件/git 源/合并/渲染/TUI)+ claude-code-kit + gate 事件 + oneline + 三平台 CI(ubuntu / windows / macos);W2 交互跃迁(任务详情/多波次/过滤/台账写回/gh 远程源);W3 收口:多里程碑分组、面板屏障行、PreToolUse dispatched 入口、事件窗速度线。
- **一期清偿(W4,0.3.0)**:评估清偿——gate 折叠保守化(exit 不可知记 failed,不虚报)、详情面板事件尾上板、渲染截断/子进程执行器双收敛、手搓解析器 proptest 性质面、Release 自动化(tag 触发四 triple 资产)。
- **信任锚与可达性(W5–W6,0.4.0/0.5.0)**:done_at 契约加法(写回 `d` 键自动盖章)+ 物证 `?` 交叉核对(done 自报晚于最近通过门即亮,自报无物证非指控);`±HHMM` 基本格式容忍;`render graph --format svg` 矢量输出;安装器可执行位修复与 ps1 静态审查。
- **0.8 产品化(W10,0.8.0,已完成)**:多仓聚合(`render panel` 多位置 + 自研 glob 展开,多仓出精要视图——每仓页眉统计/在跑/失败门/blocked/⚠,单仓输出逐字节不变)、离场摘要(`render digest [--strict]` 纯文本无 ANSI,`--strict` 有失败门或 blocked 退 1,cron 夜间监控)、用户自定义 gate(`.agentdash/config.json` 声明词表,验证门契约从内置六条开放,损坏整表静默回退,六宿主同一二进制自动受益)。
- **0.9 证据闭环(W11,0.9.0,已完成)**:pending 池会话归属(gate 槽位记录归属会话,Stop 只折叠本会话——嵌套双会话真机实证互不污染)、claude 宿主退出码实证入册 + 红侧 error 串头解析(绿侧证据分级句校准,不臆造 0)、无 who completed 配对启发(缺省配最老在跑并标 `(inferred)`,`--no-infer` 关掉回严格)、`agentdash stats` 观测面(宿主使用率/gate 通过率/任务周转三表,`--host` 过滤,纯文本恒退 0)、契约 v1.0 公开版([`docs/contract.md`](docs/contract.md))。
- **0.10 观测可读性与 TUI 交互(W12,0.10.0,已完成)**:面板 unknown 第三态(`exit=null` 折叠——如 zcode 绿侧——渲染暗色 `? … (unknown)`,与有码真失败 ✗ 视觉可辨,panel/digest 同源,stats 三列既有)、watch TUI 点击交互(面板视图左键点击任务行聚焦——行映射与渲染同源零漂移;滚轮滚动主区/详情右栏按列路由;换视图滚移归零;图视图点击守卫修复漏算页眉行)。
- **二期**:codex-kit / opencode-kit(宿主扩展)、远程源扩展(PR / CI 缓存)、观察者兼底完善。
- **三期**:分发矩阵铺满(Release 二进制 / 包管理器 / 插件市场)。

## 与 claude-dash 的关系

claude-dash 是已**冻结的参考实现**:agentdash 的渲染与单行算法移植自它,
以 claude-dash 76 断言套件的实测语义为准(本仓移植黄金断言 21 条);
它不再演进,新能力一律在 agentdash 落地。两者数据文件互不通用。
