# RFC 003 — ADE 归因补丁（草案）

状态：草案（feat/ade-attribution，待审合 main）
日期：2026-09-21

## 动机

xja3.0 级并行子智能体把 gate 开在子会话上，但 `SubagentStop` 不折叠 pending 槽，tool/gate 行又不带 `session_id`。ADE 控制塔无法把事件归因到任务/会话。本 RFC 给 hook 写侧补四件最小改动，读侧（panel/graph）保持向后兼容。

## 字段

事件行新增可选 `session_id`（字符串）。载荷顶层 `session_id` 非空则写入；缺席不落字段——与既有 claude-code 行逐字节同形。

`events.rs` `RawEvent` 同步加该可选字段（serde 缺省忽略，旧语料零迁移）。

## 补丁与兼容矩阵

| 补丁 | 行为 | 无 session_id 的老路径 |
|---|---|---|
| (a) session_id 落 tool/gate/agent 行 | `stamp` 第三参写入 | 字段省略 |
| (b) SubagentStop 折叠本会话 pending | 复用 `on_stop` 会话池折叠 | default 池互折（与今日 Stop 一致） |
| (c) opencode metadata 退出码 | `tool_response.metadata.exit`（vendor `bash.ts` StructuredOutput） | 既有 status/exit_code/exit 链不变 |
| (d) kit `session.idle` → `session.status` | 仅 `status.type===idle` 发 stop | 其他宿主 kit 不动 |

## Sunset

v1 无 `session_id` 行永久可读。sunset 条件：补丁二进制部署到全部活跃项目（含 xja3.0 PATH 上的全局 `agentdash`）之后，ADE 才把“行上必有 session_id”当成硬断言。合入 main 前本 feat 仅经 `cargo install --root <ade>/.tools` 私有消费。
