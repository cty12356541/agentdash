---
name: agentdash
description: >-
  Render the AgentDash task dashboard, DAG graph, strict wave digest, and host/gate/turnaround stats on demand or at task state transitions.
  Use this skill when the user asks for 进度/DAG/仪表盘/gate 状态/验证门/周转统计, or whenever dispatching, reviewing, fixing, or completing tasks in an orchestrated Antigravity wave.
---

# AgentDash — 任务仪表盘与度量点播 (Antigravity)

数据源是落盘权威契约与会话事件,禁止从对话记忆臆造进度或门禁结论:
- `.agentdash/events.jsonl` — Antigravity 生命周期钩子(`--host agy`)追加的工具、验证门(`running`/`passed`/`failed`)与子智能体派发/归并事件(带 `[agy]` 与 `conversationId` 归因)
- `.agentdash/ledger.json` — 任务台账 v1.1(`lanes` / `tasks` / `barriers` / `milestones`,含 `started_at` 与 `done_at` RFC 3339 时间戳)
- `.agentdash/config.json` — 项目自定义验证门词表(可选,先于内置六门匹配)
- Git 快照 — 零契约仓自动降级为 Git 提交链 + 在跑活动视图

## 常用渲染与统计命令

通过 `run_command` 在工作区根目录执行:

```bash
agentdash render panel                 # 全面板视图(支持多路径/glob 聚合精要面板)
agentdash render graph                 # 终端字符 DAG 图(--format svg 可导出矢量图)
agentdash render digest --strict       # 纯文本离场摘要;存在失败 gate 或 blocked 任务时退出码为 1
agentdash oneline                      # 无 ANSI 单行状态摘要
agentdash stats --host agy             # 观测三表:[agy] 宿主使用率 / gate 通过率 / started_at→done_at 周转耗时
agentdash watch --once                 # 单帧 TUI 快照(常驻交互式 TUI 请在独立终端跑 agentdash watch)
```

## Antigravity 跃迁与验证门约定

1. **波次状态跃迁附图**:每次在 `.agentdash/ledger.json` 推进任务状态(`pending → active → review/fix-round → done/blocked`,同步写入 `started_at` / `done_at`)后,在回复末尾附上 `agentdash render panel` 的真实输出。
2. **验证门自动采集(`--host agy`)**:
   - `PostToolUse` 捕获 `run_command`:命中 `.agentdash/config.json` 自定义门或内置六门(`cargo test`/`clippy`/`fmt`、`go test`、`npm test`、`gh pr checks`)时自动记 `gate=running`,并从顶层 `error`、结构化回执或 `transcriptPath`(`transcript.jsonl` 有界尾部 + `stepIdx`)提取退出码。
   - 会话 `Stop` 自动按 `conversationId` 会话池折叠在途门为 `passed`(显式 `exit 0`)或 `failed`(非零或 `(exit unknown)` 延迟补查仍缺失),绝不虚报通过。
   - `PreToolUse`(`invoke_subagent`)自动登记 `agent dispatched`(`Role`/`TypeName` + `Prompt`),子智能体结束并触发 `Stop`(`fullyIdle: true`)时自动配对 `completed`。
3. **收口门禁校验**:波次收尾或合并前执行 `agentdash render digest --strict`,确认退出码为 `0` 且无悬空失败门或 `blocked` 任务。

## 诚实约束

仪表盘是契约文件的只读投影:任一数据源缺失或损坏时保留警告行原样呈现;图形展示与台账/事件冲突时以 `.agentdash/ledger.json` 和 `.agentdash/events.jsonl` 为准,不得篡改数据迎合展示。
