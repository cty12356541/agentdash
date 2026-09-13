# agentdash v1 设计——CLI 编码代理的通用 TUI 任务仪表盘

- 日期:2026-09-13
- 状态:方向经维护者批准(混合架构 / 首发三工具 / Rust 单二进制 / 分期新建)
- 血统:claude-dash(Python 参考实现)的泛化重写;算法资产(分层布局/边路由/宽字符/汇流接合)已在该仓库实测验证
- 前置分析:`claude-dash/docs/history/2026-09-13-dashboard-info-analysis.md`(信息需求反推)

## 1. 定位与目标

**一句话**:任何 CLI 编码代理(Claude Code / Codex CLI / opencode,后续更多)旁边的常驻 TUI 任务仪表盘——单二进制、零运行时依赖、开箱降级可用、装集成包后深语义。

- 推广是硬目标:分发即 `cargo install`/`brew`/`scoop`/各家插件市场,用户机器上**不允许**出现"装 Python/对版本"类前置
- 信息需求以分析文档为准:验证门运行态与 PR/CI 状态是最高频缺口(P1)
- 成功判据(一期):Claude Code 用户一条命令装上、侧栏常驻、能看到任务 DAG + 验证门状态

## 2. 非目标(一期明确不做)

远程/团队共享、Web 面、自身调度执行(接 llmos W28 议题另议)、Windows 之外的 tmux 深联动、i18n 框架(内置中英与现有 dash 一致)

## 3. 总体架构

```
┌─ agentdash(Rust 单二进制 crate:agentdash)─────────┐
│  contract  契约解析与校验(ledger.json + events.jsonl)│
│  sources   观察者源:git(快照)/ 进程探测 / 文件监视    │
│  remote    远程源:gh api(缓存 120s,闪断降级)        │
│  model     多源合并(源可信序:契约 > 事件 > git > 远程)│
│  render    ratatui 投影:panel/graph/oneline/statusline│
│  tui       事件循环:分级刷新 + 键鼠(移植 shim 语义)   │
├─ 集成包(独立分发音单元,只写契约)───────────────────┤
│  claude-code-kit(hooks + skill + 安装脚本)          │
│  codex-kit / opencode-kit(二期)                     │
└──────────────────────────────────────────────────────┘
```

- 单向数据流(承 claude-dash):集成包/观察者只产出数据,投影无副作用;图与数据冲突修渲染器不修数据
- 刷新分级:契约/事件层 5s、git 层 30s、远程层 120s+磁盘缓存(闪断时显示缓存+staleness 标记)
- 降级矩阵:无契约 → git+进程伪任务;无 git → 空态引导;无远程 → 隐藏区块;终端窄 → 宽度自适应(算法已验)

## 4. 通用任务契约 v1(核心资产)

目录约定:`<repo>/.agentdash/`(gitignore 整目录;`config.toml` 例外可提交)。两文件:

### 4.1 `ledger.json`(任务台账;agent 或人写)

```json
{
  "$schema": "agentdash.tasklog.v1",
  "wave": "W23",
  "title": "波次标题",
  "profile": "sdd",
  "lanes":  [{"name": "A-impl", "tasks": [1, 2]}],
  "tasks": {
    "1": {"label": "动词短语", "state": "active", "note": "fix round 2/5"}
  },
  "barriers": [{"id": "B1", "after": [1], "unlocks": [2]}]
}
```

- **状态机(通用最小核)**:`pending → active → done`,终态另含 `blocked`;`review` / `fix-round` 为**可选富态**(profile 声明后有效,渲染加分但非校验必填)
- `profile`:语义超集标识。`"sdd"` 启用修复环计数/停放项/Rulings 透出(解析 note 与台账附加文件);其他工具可定义自己的 profile,内核只保证最小核
- 校验:损坏 JSON/未知字段 → 该源降级为警告行,不白屏(承 §9 精神);schema 用 JSON Schema 文件发布,集成包可自行校验

### 4.2 `events.jsonl`(追加事件流;hook 写,一行一 JSON)

```json
{"ts": "2026-09-13T21:00:00+08:00", "kind": "gate", "gate": "cargo-test", "state": "running"}
{"ts": "...", "kind": "gate", "gate": "cargo-test", "state": "passed", "detail": "298 passed / 0 failed"}
{"ts": "...", "kind": "agent", "event": "dispatched", "task": "1", "who": "implementer-1"}
{"ts": "...", "kind": "agent", "event": "completed", "task": "1", "who": "reviewer-1"}
{"ts": "...", "kind": "tool", "tool": "bash", "phase": "end", "exit": 0, "summary": "gh pr checks"}
```

- `gate` 事件是 P1 主角:验证门(测试/clippy/构建)的 running/passed/failed + 关键数字——由集成包从工具回调里提取
- `agent` 事件:子代理身份+时长(高频缺口 #3)
- 事件只追加;重放语义承 state.jsonl(首见身份、标签配对)

## 5. Rust 内核要点

- crate 单一(先不分库,规模阈值后再拆);ratatui + crossterm;serde + serde_json;git 调用走 std::process(显式 UTF-8,承 Python 版教训)
- 渲染移植清单(已验算法,直接翻译):最长路径分层(含环保险)、框化节点 + ┬/▼/┴ 接合路由、东亚宽字符显示宽、窄窗格自适应(钳 40..120)、里程碑标题截断
- 新增:ratatui 差分重绘取代整屏清除(消灭闪烁);状态栏输出模式(`agentdash oneline`,供各工具 statusline 配置)
- 键鼠:f/c/⏎/g/q + SGR 点击聚焦;⏎ 送对话通道一期仅在 Claude Code kit(tmux send-keys;Windows 一期降级为复制提示)

## 6. Claude Code 集成包(一期)

- hooks:PostToolUse/Stop/SubagentStop → 提取 gate/agent/tool 事件追加 events.jsonl(**新增 gate 提取**:匹配 bash 命令前缀 cargo test/clippy/fmt/go test/npm test + 退出码与输出摘要)
- skill `/agentdash`:点播渲染 + 状态跃迁附图约定(承 /dash)
- 分发:插件市场(`claude plugin marketplace add`)+ 独立脚本安装;二进制进插件 `bin/`

## 7. 错误处理与降级(规范)

- [AD-ERR-001] 任一源损坏/缺失 → 该源警告行 + 其余源照常,绝不白屏
- [AD-ERR-002] 远程源超时/断网 → 缓存 + staleness 时间戳
- [AD-ERR-003] 契约 schema 不识别 → 按"仅事件层"运行并提示升级
- [AD-ERR-004] 终端 <40 列 → oneline 形态而非破图

## 8. 测试策略

- 契约:schema 校验用例集(合法/损坏/富态/最小核)
- 渲染:黄金输出快照(移植 Python 版 76 测试的断言语义,含宽字符/窄窗格/路由)
- 源:git/事件重放的单测;远程源用录制的 JSON 夹具
- 集成:三平台 CI(承 llmos 流水线经验);claude-code-kit 的 hook 用 fixture 会话回放

## 9. 分期

- **一期**:内核(契约/事件/git 源/渲染/TUI)+ claude-code-kit + gate 事件 + oneline;验收 = 本机侧栏常驻跑通 Claude Code 场景 + 三平台 CI 绿
- **二期**:codex-kit、opencode-kit、远程源(gh/PR/CI)、观察者兼底完善
- **三期**:分发矩阵铺满、Gemini 观察者、多仓聚合视图

## 10. 与 claude-dash 的关系

Python 版冻结为参考实现(算法语义的 executable spec),只修阻塞级问题不再加功能;其 PR #1 合并后打 `v0.2.x-frozen` 标签。claude-dash 的 W25 演示台账作为契约 v1 的首个真实样本。
