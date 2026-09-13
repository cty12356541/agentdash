---
name: agentdash
description: Render the agentdash task dashboard (panel/oneline) on demand, and attach a panel at task state transitions in an orchestrated wave. Use when the user asks for 进度/DAG/仪表盘/gate 状态/验证门, or at any task state transition. Works for any repo; deep semantics when .agentdash/ contract exists.
---

# /agentdash — 任务仪表盘点播

数据源是权威契约文件与会话事件,不要从对话记忆重构状态:
`.agentdash/events.jsonl`(hook 追加事件,含 gate running/passed/failed)
+ `.agentdash/ledger.json`(任务台账,存在时)+ git 快照。零契约时降级为 git+进程伪任务。

## 执行

```bash
agentdash render panel   # 会话内点播(附图)
agentdash oneline        # 单行摘要(statusline 同源)
agentdash watch          # 常驻 TUI(侧栏分屏跑)
```

## 跃迁约定(编排波次中)

派发/过审/修复环/完成任务时:回复末尾附 `render panel` 输出。
验证门(cargo test/clippy/fmt、go test、npm test、gh pr checks)由 hook 自动记录:
工具回调时 gate=running,turn 结束折叠为 passed/failed(exit + 一行摘要)——
无需手工登记;引用结论须来自 events.jsonl 而非对话记忆。

## 诚实约束

仪表盘是契约的投影:任一源损坏/缺失 → 警告行不白屏;
图与数据冲突时以台账/事件为准并修渲染器,不得改数据迁就图。
