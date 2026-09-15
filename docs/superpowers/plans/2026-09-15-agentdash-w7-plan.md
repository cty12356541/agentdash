# agentdash W7 计划(多宿主 kit 与跨宿主一致性)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development。批内可并行车道,批间屏障;执行/审查子代理一律 flash 档。
> **Spec 基线:** 2026-09-15 W7 规格增量(API 依据 2026-09 调研:codex lifecycle hooks / opencode plugins)。
> **开工前:** dogfood 台账换 W7 候选车道;基线 199 测试三件套全绿。

## 批一(内核:一致性地基)

### T1 (W7-001):`--host` 旗标 + `subagentstart` 事件 + agent 表 host
**Files:** `src/hook.rs`(--host 解析、subagentstart → agent dispatched、事件落盘带 host)、`src/events.rs`(RawEvent.host;AgentEntry.host 首见)、`src/model.rs`(AgentView.host)、`src/render/panel.rs`(在跑行 `[host]`)、`tests/hook.rs` + `tests/events.rs` + `tests/merge.rs` + `tests/render_panel.rs`。
**验收:** spec C2/D1/D2;未传旗标事件无 host 字段(向后兼容);既有断言不破;三件套净。

## 批二(共享 harness + 三套 kit)

### T2 (W7-002):共享 AGENTDASH.md 单源
**Files:** `kits/shared/AGENTDASH.md`(跃迁约定/诚实约束/契约指针,标记段定界)。
**验收:** 内容与 claude-code skill 的诚实约束口径一致;可被三套安装器幂等合入。

### T3 (W7-003):codex-kit
**Files:** `kits/codex/hooks.json`、`kits/codex/install.sh`(D3 保守策略 + AGENTS.md 合入)、`kits/codex/README.md`。
**验收:** fresh/已含自家/无自家三分支行为正确;trust+/hooks 审查提示在文档。

### T4 (W7-004):opencode-kit
**Files:** `kits/opencode/plugins/agentdash.js`(D4 降级铁律)、`kits/opencode/install.sh`、`kits/opencode/README.md`。
**验收:** 插件过 `node --check`;tool.execute.after / session.idle 映射如实;子代理缺失如实标注。

### T5 (W7-005):claude-code kit 归属补齐
**Files:** `kits/claude-code/hooks/hooks.json` + `install.sh`(`--host claude`;CLAUDE.md 合入 AGENTDASH.md 标记段)、相关清单测试。
**验收:** 四钩子命令带 host;清单测试同步;幂等不破。

## 收口

### T6 (W7-006):双宿主交错 dogfood + 版本收尾
同一 scratch 项目 codex/opencode 载荷交错喂入 → 单 events.jsonl + 面板 `[codex]`/`[opencode]` 并现;0.6.0 三处版本 + marketplace description;README 宿主矩阵;报告入册;三件套;终审;推送(核远端);tag `v0.6.0`;资产抽验。
