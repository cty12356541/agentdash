---
name: agentdash-orchestrate
description: >-
  Plan and orchestrate multi-lane DAG waves in Antigravity using .agentdash/ledger.json v1.1, isolated Git worktrees, and parallel invoke_subagent workers.
  Use this skill when the user asks to orchestrate a wave, dispatch parallel subagent lanes, maintain .agentdash/ledger.json barriers and started_at/done_at timestamps, or enforce verification gates across subagents.
---

# AgentDash 多车道波次编排规程 (Antigravity)

在 Antigravity 中结合 `.agentdash/ledger.json` v1.1、`git worktree` 物理隔离与 `invoke_subagent` 并行子智能体执行多车道 DAG 编排。主控会话唯一持有主工作区台账写权限,子智能体仅在各自隔离 worktree 分支内修改代码与跑门禁。

## 1. 起草波次契约 (`.agentdash/ledger.json` & `config.json`)

在主工作区建立或更新 `.agentdash/ledger.json`(`$schema: "agentdash.tasklog.v1"`,使用 `review`/`fix-round` 富态时声明 `"profile": "rich"`):

```json
{
  "$schema": "agentdash.tasklog.v1",
  "profile": "rich",
  "wave": "W14",
  "title": "Antigravity Host Integration",
  "lanes": [
    { "name": "A-core", "tasks": ["W14-01", "W14-03"] },
    { "name": "B-kit", "tasks": ["W14-05"] }
  ],
  "tasks": {
    "W14-01": {
      "label": "Implement --host agy hook protocol",
      "state": "done",
      "started_at": "2026-10-28T03:00:00Z",
      "done_at": "2026-10-28T03:35:00Z"
    },
    "W14-03": {
      "label": "Add v1.1 started_at turnaround metrics",
      "state": "active",
      "started_at": "2026-10-28T03:36:00Z"
    },
    "W14-05": {
      "label": "Create kits/antigravity 7th host kit",
      "state": "pending"
    }
  },
  "barriers": [
    { "id": "B1", "after": ["W14-01"], "unlocks": ["W14-03", "W14-05"] }
  ]
}
```

- **依赖单点声明**:前置约束只写 `barriers`(`after` 全部 `done` 后才解锁 `unlocks`),不在任务字段重复维护依赖。
- **自定义验证门**:若项目门禁超出内置六门(`cargo test`/`clippy`/`fmt`、`go test`、`npm test`、`gh pr checks`),先在 `.agentdash/config.json` 配置 `{"gates":[{"name":"...","words":["..."]}]}`。
- 起草后运行 `agentdash render panel` 与 `agentdash render graph` 核验拓扑无误。

## 2. 按可并行车道建立隔离 Git Worktree

仅挑选所有前置 `barriers` 已满足且文件域互不重叠的任务同批并行,为每条车道创建独立 worktree(可放在临时/scratch 目录):

```bash
git worktree add <scratch_dir>/wt-<task-id> -b feat/<task-id>-r1
```

## 3. 更新台账并发起 `invoke_subagent` 并行派发

1. **盖章派发时刻**:在主工作区 `.agentdash/ledger.json` 将本批任务置为 `"state": "active"` 并写入当前 UTC RFC 3339 `"started_at"`,执行 `agentdash render panel` 留存派发快照。
2. **并行调用 `invoke_subagent`**:
   - `TypeName`: `"self"`(默认继承当前模型 `Model: "inherit"`)
   - `Role`: 清晰标注车道与职责(如 `"Lane B Kit Builder"`——`--host agy` 的 `PreToolUse` 钩子自动提取 `Role`/`TypeName` 作为面板 `▶ <who> [agy]` 的在跑标识)
   - `Prompt` 必须明确规定:
     1) **唯一工作目录**:只准在分配的 worktree 绝对路径内读写与执行命令,严禁触碰主工作区或 `.agentdash/` 台账文件,严禁 `git push`;
     2) **文件边界与交付物**:列出允许新建/修改的路径清单与验收标准;
     3) **自验门禁与提交**:先红后绿跑通测试与验证门,在 `feat/<task-id>-r1` 分支提交 commit,并通过 `send_message` 回报 commit hash 与测试计数。
3. **响应式唤醒(禁止轮询)**:发出 `invoke_subagent` 后直接结束当前轮次或处理主控工作,切勿用循环或 sleep 轮询;子智能体调用 `send_message` 及结束触发 `Stop`(`fullyIdle: true`,自动配对 `agent completed`)时系统会自动唤醒主控会话。

## 4. 复核、合流、验证门收口与统计

1. **复核与返修环**:收到子智能体回报后,将任务置为 `"review"`,在 worktree 检查 `git diff` / `git show` 与文件范围:
   - 若需返修:置 `"state": "fix-round"` 并更新 `"note": "fix round 1/N"`,通过 `send_message` 发回**原**子智能体 `conversationId` 继续修复(不重开新实例)。
2. **主仓合流与门禁终验**:
   - 将通过审查的分支合入/cherry-pick 回主工作区,清理临时 worktree(`git worktree remove <path>`)。
   - 在主工作区真实运行项目验证门(使 `--host agy` `PostToolUse` + `Stop` 将最新门禁证据写入主工作区 `.agentdash/events.jsonl`)。
3. **完成盖章与严格摘要校验**:
   - 将任务置为 `"state": "done"`,写入真实 UTC RFC 3339 `"done_at"`。
   - 执行 `agentdash render digest --strict`(须退 `0`)、`agentdash render panel` 与 `agentdash stats --host agy`,核对所有门禁绿灯及 `started_at → done_at` 周转表后向用户汇报并附上最终面板。
