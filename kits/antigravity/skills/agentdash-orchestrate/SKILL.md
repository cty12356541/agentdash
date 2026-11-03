---
name: agentdash-orchestrate
description: >-
  Decompose complex engineering requirements into SDD/TDD multi-lane DAG waves and orchestrate parallel subagent execution in Antigravity using .agentdash/ledger.json v1.1, isolated Git worktrees, and verification gates.
  Use this skill when the user asks to break down tasks (任务拆分/波次规划), orchestrate a wave, dispatch parallel subagent lanes, maintain .agentdash/ledger.json barriers and started_at/done_at timestamps, or enforce verification gates across subagents.
---

# AgentDash 任务拆分与多车道波次编排规程 (Antigravity)

在 Antigravity 中将复杂需求拆解为正交决策空间,结合 `.agentdash/ledger.json` v1.1、`git worktree` 物理隔离与 `invoke_subagent` 并行子智能体执行多车道 DAG 编排。主控会话唯一持有主工作区与 `.agentdash/` 台账写权限,子智能体仅在各自隔离 worktree 分支内实现并自验。

---

## 1. 任务拆分判据与四段式波次拓扑 (Decomposition Rules)

### 1.1 何时启用多车道编排 (防过度拆分)
- **单车道直跑(不建 worktree)**:改动 ≤2 个强耦合文件、规格尚在探索 spike 阶段、或全链路串行依赖的小修小补。
- **多车道并行波次(3–5 条 `lanes`)**:涉及 ≥3 个独立模块/文件族,且共享边界契约已明确。

### 1.2 同批可并行的三判据 (决策空间正交)
两个任务仅当**同时满足**以下三条时,才允许排入同一并行批次(无先后 `barriers` 边):
1. **一文件一主(零写冲突)**:同一并行批次内,任何源码/测试文件最多只有一个写者。巨石文件(如核心路由表、总入口、主状态机)是天然串行点——必须先在窄前置波拆文件或抽桩,严禁多子智能体同改一处。
2. **前置契约已冻结(SDD 先行)**:跨车道调用的函数签名、数据结构、API 载荷或 Schema 已在前置屏障任务中落盘定桩;严禁在接口未定时用并行掩盖分叉。
3. **失败互不掩埋**:各车道拥有独立可跑的单元测试/验证门目标(`tests` 字段),A 车道的红灯不会阻塞 B 车道的本地红绿闭环。
*(注:纯只读任务——代码审查、取证、基准测量——零写冲突,可在单屏障后 5–8 路最大扇出。)*

### 1.3 标准四段式 DAG 拓扑 (`W0 → W1 → W2 → W3`)
拆分复杂波次时,按以下四层骨架映射为 `lanes` + `barriers`(大型波次可同步声明 `milestones`):
- **阶段 0 · 规格与定桩层(窄/串行,解锁 `B1`)**:冻结契约文档、公共类型/接口 stub、共享配置(含 `.agentdash/config.json` 自定义验证门)。不确定性最高,保持单主窄波。
- **阶段 1 · 并行实现层(宽扇出 3–5 车道,`B1` → `B2`)**:各车道按独立文件域领走任务,严格执行 **TDD 先写失败测试(红)再写实现(绿)**。
- **阶段 2 · 集成合流层(窄/单主,`B2` → `B3`)**:跨车道合流、删桩接真实现、联调跨模块接线与差分测试(防测试缝注入掩盖生产断链)。
- **阶段 3 · 终验与收口层(`B3` 后)**:全量回归门禁、端到端验证、`digest --strict` 零告警校验与文档同步。

---

## 2. 起草波次契约 (`.agentdash/ledger.json` & `config.json`)

在主工作区建立或更新 `.agentdash/ledger.json`(`$schema: "agentdash.tasklog.v1"`,使用 `review`/`fix-round` 富态时声明 `"profile": "sdd"` 或 `"rich"`):

```json
{
  "$schema": "agentdash.tasklog.v1",
  "profile": "sdd",
  "wave": "W15",
  "title": "W15: Feature Wave Title",
  "lanes": [
    { "name": "A-contract-stub", "tasks": ["W15-01", "W15-05"] },
    { "name": "B-engine-impl",   "tasks": ["W15-02"] },
    { "name": "C-ui-adapter",    "tasks": ["W15-03"] },
    { "name": "D-e2e-audit",     "tasks": ["W15-04"] }
  ],
  "tasks": {
    "W15-01": {
      "label": "冻结公共接口签名与契约测试桩",
      "state": "done",
      "spec": "docs/specs/w15-contract.md",
      "tests": "tests/test_contract_stub.py",
      "gate": "pytest",
      "round": 1,
      "max_rounds": 3,
      "started_at": "2026-11-03T06:00:00+08:00",
      "done_at": "2026-11-03T06:20:00+08:00",
      "note": "a1b2c3d · stub + schema frozen"
    },
    "W15-02": {
      "label": "实现后端引擎增量缓存与预算降级",
      "state": "active",
      "spec": "docs/specs/w15-contract.md#engine",
      "tests": "tests/test_engine_cache.py",
      "gate": "pytest",
      "round": 1,
      "max_rounds": 3,
      "started_at": "2026-11-03T06:22:00+08:00"
    },
    "W15-03": {
      "label": "实现前端工作台渐进披露视图组件",
      "state": "active",
      "spec": "docs/specs/w15-contract.md#ui",
      "tests": "tests/test_ui_view.py",
      "gate": "pytest",
      "round": 1,
      "max_rounds": 3,
      "started_at": "2026-11-03T06:22:00+08:00"
    },
    "W15-04": {
      "label": "全链路集成合流与 E2E 差分验证",
      "state": "pending",
      "spec": "docs/specs/w15-contract.md#e2e",
      "tests": "tests/test_e2e_w15.py",
      "gate": "pytest"
    },
    "W15-05": {
      "label": "波次收口审计与发布包同步",
      "state": "pending",
      "gate": "pytest"
    }
  },
  "barriers": [
    { "id": "B1", "after": ["W15-01"], "unlocks": ["W15-02", "W15-03"] },
    { "id": "B2", "after": ["W15-02", "W15-03"], "unlocks": ["W15-04"] },
    { "id": "B3", "after": ["W15-04"], "unlocks": ["W15-05"] }
  ],
  "milestones": [
    { "id": "M1", "title": "契约与并行实现", "tasks": ["W15-01", "W15-02", "W15-03"] },
    { "id": "M2", "title": "集成与收口", "tasks": ["W15-04", "W15-05"] }
  ]
}
```

- **依赖单点声明**:前置约束只写 `barriers`(`after` 全部 `done` 后才解锁 `unlocks`),不在 `tasks` 内重复维护依赖字段;同一任务在 `milestones[].tasks` 中最多出现一次(重复引用会触发降级警告并导致 `digest --strict` 退 1)。
- **任务自描述扩展字段**:推荐为每个任务声明 `"spec"`(规格锚点)、`"tests"`(验收测试文件)、`"gate"`(目标验证门名)与 `"round"`/`"max_rounds"`(返修轮次上限,内核按未知字段容忍保留)。
- **自定义验证门**:若项目门禁超出内置六门(`cargo test`/`clippy`/`fmt`、`go test`、`npm test`、`gh pr checks`),先在 `.agentdash/config.json` 配置 `{"gates":[{"name":"pytest","words":["pytest"]}]}`。
- **拓扑预检**:起草后立即运行 `agentdash render panel` 与 `agentdash render graph` 核验车道、屏障与里程碑无误且无 `warnings`。

---

## 3. 建立隔离 Worktree 与 `invoke_subagent` 实名派发铁律

### 3.1 建立每车道独立 Git Worktree
仅挑选所有前置 `barriers` 已满足且文件域互不重叠的任务同批并行,在主仓外(如父目录或临时目录)建立独立 worktree:

```bash
git worktree add <worktree_root>/wt-<task-id> -b feat/<task-id>-r1
```

### 3.2 盖章 `started_at` 并遵循钩子实名配对规则调用 `invoke_subagent`
1. **盖章派发时刻**:在主工作区 `.agentdash/ledger.json` 将本批任务置为 `"state": "active"` 并写入当前 RFC 3339 `"started_at"`,执行 `agentdash render panel` 留存派发快照。
2. **并行调用 `invoke_subagent`(严格遵守 `--host agy` 钩子归因四铁律)**:
   - **`TypeName`: `"self"`**,`Model`: `"inherit"`(除非用户另有指定)。
   - **`Workspace`: 必须保持默认 `"inherit"`(严禁传 `"branch"`)**:`agentdash hook --host agy` 依赖宿主传入的 `workspacePaths` 定位主仓 `.agentdash/events.jsonl`;保持 `"inherit"` 并让子智能体在显式分配的 worktree 路径下操作,子会话 `Stop` 事件才能准确写回主仓闭合在跑状态。
   - **`Role` 必须带唯一任务 ID 前缀**(如 `"[W15-02] Engine Cache Implementer"`):`events.rs` 以 `who`(`Role` 前 80 字符)作为活跃子智能体主键,同波重名会互相覆盖槽位。
   - **`Prompt` 首行前 80 字符必须唯一且以 `[<task-id>]` 开头**:`hook.rs` 在子智能体 `Stop`(`fullyIdle: true`)时读取子会话 `transcript.jsonl` 首步(`step_index == 0`)中 `<SYSTEM_MESSAGE>` 的 `content` 前 80 字符与 `dispatched.task` 精确比对以反查实名 `who`。切勿在多个子智能体 `Prompt` 开头粘贴相同的长段公共背景。
   - **`Prompt` 正文必须锁死四件事**:
     1) **唯一工作目录**:只准在 `<worktree_root>/wt-<task-id>` 绝对路径内读写文件与执行命令,严禁触碰主工作区或 `.agentdash/` 台账文件,严禁 `git push`;
     2) **文件白名单与规格锚点**:列出仅允许新建/修改的文件路径清单(对应一文件一主边界)及 `spec` 验收条款;
     3) **TDD 与自验门禁**:先写失败测试验证红灯,再实现至绿灯,跑通目标测试文件(`tests`);
     4) **提交与回报格式**:在 `feat/<task-id>-r1` 分支提交 commit,调用 `send_message` 向父会话回报 `commit hash`、修改文件清单与测试通过计数。
3. **响应式唤醒(禁止轮询)**:发出 `invoke_subagent` 后直接结束当前轮次或处理主控工作,切勿用循环或 sleep 轮询。
4. **异常子智能体终止回收**:若某个子智能体跑偏或无需继续,主控调用 `manage_subagents(Action: "kill", ConversationIds: [...])` 或 `"kill_all"`——`--host agy` 的 `PostToolUse` 钩子会自动补齐被杀子智能体的实名 `completed` 事件并清理其遗留 `pending_gate.json` 槽位。

---

## 4. 复核、返修环、防 `done ?` 门禁收口与统计

1. **只读复核与返修环 (`review` / `fix-round`)**:
   - 收到子智能体回报后,将任务置为 `"state": "review"`,在 worktree 检查 `git diff` / `git show` 是否越界、是否真有接线差分测试。
   - 若需返修:置 `"state": "fix-round"` 并更新 `"round"` 与 `"note": "fix round 1/3: <原因>"`,通过 `send_message` 发回**原**子智能体 `conversationId` 继续修复(复用上下文,不重开新实例)。
2. **主仓合流与 Worktree 清理**:
   - 审查通过后,在主工作区 `git merge --no-ff feat/<task-id>-r1` 或 `git cherry-pick`,随后执行 `git worktree remove <worktree_root>/wt-<task-id>` 与 `git branch -d feat/<task-id>-r1`。
3. **防 `done ?`(自报无物证)的收口顺序铁律**:
   - 内核渲染器会核对 `done_at` 与 `events.jsonl` 中最近一次 `gate=passed` 的 `ts`:一旦 `done_at > last_gate_passed_ts`,面板就会打出 `done ?` 存疑标记!
   - **标准收口顺序**:
     1) 先在主工作区 `.agentdash/ledger.json` 将合流任务置为 `"state": "done"`,写入当前 RFC 3339 `"done_at"` 与 `"note": "<commit> · <test summary>"`;
     2) **紧接着在主工作区运行真实项目验证门命令**(使 `--host agy` 钩子采集并在 `Stop` 折叠出 `last_gate_passed_ts >= done_at` 的最新物证)。
   - **防跨仓门禁污染**:在非 Rust 业务仓会话中,切勿在顶层命令裸跑其他仓的验证门词(如 `cargo test`/`clippy`/`fmt`),以免工作区钩子误记跨仓验证门事件。
4. **严格摘要与周转统计闭环**:
   - 波次收尾时依次执行:
     ```bash
     agentdash render digest --strict
     agentdash render panel
     agentdash render graph
     agentdash stats --host agy
     ```
   - 确认 `digest --strict` 退出码为 `0`(无失败门、无 `blocked` 任务、无契约/事件警告),核对 `started_at → done_at` 周转表后向用户汇报并附上最终面板。
