# agentdash W5 计划(信任锚落地:done_at 物证交叉核对)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development。批内可并行车道,批间屏障;执行/审查子代理一律 flash 档。
> **Spec 基线:** 2026-09-15 W5 规格增量(来源:信任模型讨论 + W4 复评清单)。
> **开工前:** dogfood 台账换 W5 候选车道;基线 189 测试三件套全绿。

## 批一(快速清偿——低风险热身)

### T1 (W5-002):MSRV 文档修正
**Files:** `README.md`(≥1.85 → ≥1.88,注明 let-chains 缘由)、`kits/claude-code/skills/agentdash-setup/SKILL.md`(同改)。
**验收:** 两处口径一致;三件套净(纯文档)。

### T2 (W5-003):clock_slice 收敛
**Files:** `src/render/mod.rs`(公共 `clock_slice`)、`src/render/panel.rs`、`src/tui.rs`(私有副本删除)。
**验收:** 零行为变更;既有断言不破;三件套净;单 commit。

## 批二(主 feature——信任锚)

### T3 (W5-001):done_at 契约 + 写回盖章 + `?` 物证标记
**Files:** `src/contract.rs`(TaskSpec 可选 done_at)、`schema/agentdash.tasklog.v1.json`(加法)、`src/events.rs`(last_gate_passed 对账锚)、`src/model.rs`(TaskView.done_at 穿透;Dashboard.events_present / last_gate_passed)、`src/writeback.rs`(MarkDone 盖章 / done→blocked 摘戳;注释改口)、`src/render/mod.rs`(unattested_done 判定)、`src/render/panel.rs`(任务行 ` ?`)、`src/tui.rs`(详情物证字段)、`tests/writeback.rs`(挂载树补齐)及 contract/merge/events/render_panel/tui 各组正负断言。
**验收:** spec D1–D4 全落地;旧台账黄金对照不变;`?` 四态矩阵有断言;三件套净。

## 收口

### T4 (W5-004):版本 + 收尾
0.4.0 三处版本;README 契约段补 done_at/`?` 语义一句;dogfood 台账换 W5 真值(回填两例物证状态并注明"正常来源应为写回盖章");帧 spot-check(真实事件窗下 `?` 可见);三件套;全分支终审;推送(先核远端);tag `v0.4.0`;Release 资产 workflow 出件并抽验。
