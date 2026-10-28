---
name: agentdash-setup
description: >-
  Install, configure, and verify the agentdash binary, Antigravity lifecycle hooks (--host agy), and .agentdash/config.json custom verification gates.
  Use this skill when agentdash --version fails, when the dashboard shows no [agy] events, or when the user asks to install, set up, configure custom gates, or troubleshoot AgentDash in Antigravity.
---

# AgentDash 安装与宿主配置引导 (Antigravity)

Antigravity 集成包通过生命周期钩子(`PreToolUse` / `PostToolUse` / 扁平 `Stop`)直调 `PATH` 上的 `agentdash hook --host agy <event>`,并按 Antigravity 协议在 `stdout` 返回单行 JSON(`{"decision":"allow"}` / `{}` / `{"decision":"stop"}`)。二进制缺失或未重载配置会导致事件不落盘。按以下步骤完成安装与端到端自检。

## 步骤

### 1. 探测二进制

执行 `agentdash --version`。输出 `agentdash 0.x.y` 且退出码为 `0` 则跳至第 3 步。

### 2. 安装 `agentdash` 二进制(按平台)

- **Windows**:
  - 源码编译(Rust ≥ 1.88):`cargo install --path <agentdash 仓库根>`
  - 或从 [Releases](https://github.com/cty12356541/agentdash/releases) 下载 `agentdash-v<x>-x86_64-pc-windows-msvc.zip`,解压 `agentdash.exe` 到 `%LOCALAPPDATA%\Programs\agentdash` 并加入用户 `PATH`:
    ```powershell
    $bin = "$env:LOCALAPPDATA\Programs\agentdash"
    New-Item -ItemType Directory -Force $bin | Out-Null
    Expand-Archive <zip路径> -DestinationPath $bin -Force
    ```
- **macOS / Linux**:
  - `cargo install --path <agentdash 仓库根>`(Rust ≥ 1.88),或解压对应平台 Release 压缩包放入 `/usr/local/bin` / `~/.cargo/bin`。
- 完成后重跑 `agentdash --version` 确认退出码为 `0`。

### 3. 挂载 Antigravity 钩子与技能(二选一)

- **工作区级安装(推荐随项目共享或单仓接入)**:
  - Git Bash / macOS / Linux:`bash <agentdash仓>/kits/antigravity/install.sh <目标工作区>`
  - Windows PowerShell:`powershell -ExecutionPolicy Bypass -File <agentdash仓>\kits\antigravity\install.ps1 -Target <目标工作区>`
  - 产物:幂等合入 `<工作区>/AGENTS.md` 标记段、在 `<工作区>/.agents/hooks.json` 注册 `"agentdash"` 顶层命名空间(保留其他插件命名空间)、复制三套技能至 `<工作区>/.agents/skills/`、并在 `.gitignore` 确保 `.agentdash/`。
  - **生效时机**:首次写入 `.agents/hooks.json` 与 `.agents/skills/` 后,需**新建会话或重载窗口**使当前工作区抓取新钩子与技能。
- **全局插件安装(本机所有工作区通用)**:
  - 将 `kits/antigravity/` 整目录复制到 `~/.gemini/config/plugins/agentdash/`(内含 `plugin.json`、根级 `hooks.json`、`assets/logo.svg` 与 `skills/`)。
  - **生效时机**:新创建的全局插件目录仅在宿主启动时扫描发现,复制后需**重启 Antigravity**。

### 4. 协议与事件落盘自检

1. 验证 `--host agy` 标准输出回执:
   - `printf '{}' | agentdash hook --host agy pretooluse` → `{"decision":"allow"}`
   - `printf '{}' | agentdash hook --host agy posttooluse` → `{}`
   - `printf '{}' | agentdash hook --host agy stop` → `{"decision":"stop"}`
2. 在重载后的会话中运行一次 `run_command`(如 `agentdash oneline`),再执行 `agentdash stats --host agy` 确认 `.agentdash/events.jsonl` 已记录 `host="agy"` 事件。

### 5. 自定义验证门与台账初始化(可选)

- **自定义验证门(`.agentdash/config.json`)**:除内置六门(`cargo test`/`clippy`/`fmt`、`go test`、`npm test`、`gh pr checks`)外,可按需配置最多 32 条词序列门(无需重启,hook 现读现用,先于内置表匹配):
  ```json
  {
    "gates": [
      { "name": "pytest", "words": ["pytest"] },
      { "name": "ui-smoke", "words": ["npm", "run", "smoke"] }
    ]
  }
  ```
  约束:`name` 限 `[a-z0-9-_]+`(≤ 40 字符),`words` 为 1..=8 个连续词元。
- **任务台账(`.agentdash/ledger.json` v1.1)**:多车道波次编排请调用 `agentdash-orchestrate` 技能起草含 `lanes`、`barriers` 与 `started_at`/`done_at` 的台账,并用 `agentdash render panel` 与 `agentdash render digest --strict` 核验。

## 诚实约束

严禁手改 `~/.gemini/config/config.json`;PATH 未就绪、钩子尚未在新会话生效或门禁失败时如实告知原因与重载要求,不得虚报已通。
