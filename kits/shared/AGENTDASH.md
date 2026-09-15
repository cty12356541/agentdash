<!-- agentdash:begin v1 -->
## agentdash 任务仪表盘约定(跨宿主统一,勿改本标记段)

数据源是权威契约,不要从对话记忆重构状态:
`.agentdash/events.jsonl`(hook 追加事件,含 gate running/passed/failed)+
`.agentdash/ledger.json`(任务台账,存在时)+ git 快照。

**执行**:`agentdash render panel`(面板)/ `render graph`(DAG)/ `oneline`
(单行)/ `watch`(常驻 TUI)。

**验证门**:cargo test/clippy/fmt、go test、npm test、gh pr checks 由 hook
自动登记(工具回调 running,turn 结束折叠 passed/failed),无需手工登记。

**跃迁约定(编排波次中)**:派发/过审/修复环/完成任务时,回复末尾附
`agentdash render panel` 输出;引用结论须来自 events.jsonl / ledger.json,
不得来自对话记忆。

**台账纪律**:状态变更即写回 `ledger.json`(state/done_at),台账更新是任务
完成定义的一部分;图与数据冲突时以台账/事件为准并修渲染器,不得改数据迁就图。
<!-- agentdash:end -->
