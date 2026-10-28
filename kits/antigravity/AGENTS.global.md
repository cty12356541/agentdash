<!-- agentdash:begin v1 -->
## agentdash 任务仪表盘与多轨编排约定(本机全局 · Antigravity 宿主,勿改本标记段)

当工作区存在 `.agentdash/ledger.json`(或执行大型多任务并行编排)时生效:

1. **权威数据源(禁止凭对话记忆重构进度)**:
   `.agentdash/ledger.json`(任务契约 v1.1) → `.agentdash/events.jsonl`(`agentdash hook --host agy` 自动追加事件流) → Git 快照。任务边界先跑 `agentdash oneline` 或 `agentdash render panel` 读取真值。
2. **钩子独占文件(零手工伪造)**:
   严禁直接手写、截断或伪造 `.agentdash/events.jsonl`、`.agentdash/pending_gate.json`、`.agentdash/.lock`;事件与验证门折叠(含内置门与 `.agentdash/config.json` 自定义门)一律由本机全局钩子 `agentdash hook --host agy` 自动登记与 `Stop` 折叠。若某仓有专属台账工具(如 `tools/ledger.py`),台账变更须经其规定入口执行。
3. **屏障与状态机纪律**:
   严格遵守 `barriers`(`after` → `unlocks`),前置未全 `done` 前严禁抢跑解锁下游任务;进入 `active` 记录 RFC 3339 `started_at`,收口 `done` 记录 `done_at` 并确保真实门禁在 `done_at` 之后折叠通过(`last_gate_passed >= done_at`,消除 `done ?` 存疑标记)。
4. **反重力多轨并行编排**:
   复杂工程按互不重叠的文件归属拆分为 3–5 条 `lanes` + `barriers`,每条并行任务建独立 Git Worktree 并通过 `invoke_subagent` 并行派发实现者;在派发、过审、修复环、跨屏障合流及收口时附 `agentdash render panel` / `render graph` 快照,收尾跑 `agentdash render digest --strict` 与 `agentdash stats --host agy`。
<!-- agentdash:end -->
