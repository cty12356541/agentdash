# agentdash · antigravity-kit(W14;Google Antigravity 宿主)

把 Google Antigravity(`agy` CLI / Antigravity 2.0 / IDE)会话与并行子智能体事件接进
agentdash 通用任务契约,与其他六宿主写**同一份** `<repo>/.agentdash/`(多宿主同面板
归因)。

## 钩子与协议映射

Antigravity 的 `hooks.json` 采用顶层命名空间(`"agentdash"`),向子命令 `stdin` 喂
camelCase JSON(`conversationId` / `workspacePaths` / `transcriptPath` / `stepIdx` /
`toolCall`),并要求 `stdout` 返回单行 JSON 回执:

| Antigravity 事件 | 结构 / Matcher | agentdash 命令 | stdout 回执 & 语义 |
|---|---|---|---|
| `PreToolUse` | 分组 `matcher: "invoke_subagent"` | `agentdash hook --host agy pretooluse` | `{"decision":"allow"}`;记 `agent` `dispatched`(`who` 取 `Role`/`TypeName`,`task` 取 `Prompt`) |
| `PostToolUse` | 分组 `matcher: "*"` | `agentdash hook --host agy posttooluse` | `{}`;`run_command` 命中验证门记 `gate=running` + 暂存槽,其余记 `tool` 行 |
| `Stop` | **扁平数组**(不支持 `matcher`) | `agentdash hook --host agy stop` | `{"decision":"stop"}`;按会话池折叠在途 gate,并归并子智能体完成事件 |

所有命令带 `--host agy`,混用时面板显示 `[agy]`、会话归因取 `conversationId`。
命令末尾**不加 `|| true`**:Windows 下宿主经 `cmd /c` 调起命令并解析 `stdout` JSON,
由 `agentdash hook --host agy` 原生保证任何异常/空输入皆退 `0` 且输出合法单行 JSON。

## 退出码提取与子智能体配对

- **`transcript.jsonl` 有界退出码提取**:Antigravity 的 `PostToolUse` 载荷在失败时携带顶层
  `error`,成功路径则按 `transcriptPath` + `stepIdx` 从会话 `transcript.jsonl` 尾部(单次上限
  64KB)提取真实退出码与末行摘要。若 `PostToolUse` 触发时刻轨迹尚未刷盘,暂存槽自动保留路径与步号,
  待 `Stop` 折叠前二次补查;仍无退出码证据时如实折叠为 `failed (exit unknown)`,绝不虚报 `0`。
- **`invoke_subagent` / `Stop` 子智能体配对**:Antigravity 无独立 `SubagentStop` 事件。父会话调
  `invoke_subagent` 时由 `PreToolUse` 落 `dispatched`;子智能体自身执行环结束并以独立
  `conversationId` + `fullyIdle: true` 触发 `Stop` 时,`agentdash` 在锁内扫描活跃派发池并自动补记
  `completed`,闭合面板在跑行。

## 两种安装形态与生效时机

### 1. 本机级全局注册(缺省推荐:`~/.gemini/AGENTS.md` + `~/.gemini/config/plugins/agentdash/`)

不传目录参数(或显式传 `--global` / `-Global`)默认执行**本机级注册**,作为本机全局 `AGENTS.md` 的一部分对所有工作区统一生效,无需逐项目安装:

```bash
# Git Bash / macOS / Linux(缺省即 --global)
./kits/antigravity/install.sh

# Windows PowerShell(缺省即 -Global)
powershell -ExecutionPolicy Bypass -File .\kits\antigravity\install.ps1
```

行为:`AGENTS.global.md` 标记段(`<!-- agentdash:begin v1 -->` .. `<!-- agentdash:end -->`)幂等合入本机全局
`~/.gemini/AGENTS.md`(Antigravity `user_global` `globalScope`,保留既有其他本机规则)→ 同步全局插件包
`~/.gemini/config/plugins/agentdash/`(`plugin.json`、根级 `hooks.json`、`assets/logo.svg` 与三套技能
`agentdash` / `agentdash-setup` / `agentdash-orchestrate`,且不重复放置 `rules/AGENTS.md` 以防双重注入)。

- **重载语义**:首次新建 `~/.gemini/config/plugins/agentdash/` 后需**重启一次 Antigravity** 使全局钩子在所有项目中挂载生效;
  `~/.gemini/AGENTS.md` 全局规则实时纳入 `user_global`。

### 2. 可选工作区级安装(`<repo>/.agents/`)

仅在团队需要将配置随单个项目仓库入库时显式传入目标目录:

```bash
# Git Bash / macOS / Linux
./kits/antigravity/install.sh <目标项目目录>

# Windows PowerShell
powershell -ExecutionPolicy Bypass -File .\kits\antigravity\install.ps1 -Target <目标项目目录>
```

行为:`AGENTDASH.md` 幂等合入 `<repo>/AGENTS.md` → `.agents/hooks.json` 幂等写入 `"agentdash"`
顶层命名空间(**保留同文件其他插件/用户命名空间**,损坏时备份 `.bak-agentdash` 后重建)→
复制三套技能至 `.agents/skills/` → `.gitignore` 幂等追加 `.agentdash/`。

- **一键巡检七宿主 kit(含本机级与项目级双形态)**:`bash scripts/verify-kits.sh`(仓库根)。
