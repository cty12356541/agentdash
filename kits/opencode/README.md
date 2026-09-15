# agentdash · opencode-kit

把 opencode 会话事件接进 agentdash 通用任务契约,与 claude-code/codex 两 kit
写**同一份** `<repo>/.agentdash/`(台账一致性 C1),指令约定同源
`AGENTDASH.md`(harness 一致性 C3)。

## 机制(2026-09 调研)

opencode 插件:`.opencode/plugins/*.js|ts` 启动自动加载,插件函数收到
`{ directory, ... }` 上下文,返回 hooks 表。本插件映射:

| opencode | agentdash 事件 | 说明 |
|---|---|---|
| `tool.execute.after` | `posttooluse` | input.tool 小写名(agentdash 大小写容忍);bash 命中验证门 → gate |
| `session.idle` | `stop` | turn 结束,折叠在途 gate |

所有事件带 `--host opencode` 归属,混用时面板在跑行显示 `[opencode]`。

## 安装

```bash
./install.sh [目标项目目录]
```

行为:AGENTDASH.md 标记段幂等合入 `AGENTS.md` → 插件复制到
`.opencode/plugins/agentdash.js` → gitignore 追加 `.agentdash/`。

## 如实标注

- **子代理边界**:opencode 插件 API 暂无原生子代理事件,在跑 agent 面板
  (dispatched/completed)仅 claude-code / codex 宿主可见;待宿主能力补齐后
  本 kit 跟进,不臆造事件。
- 运行时为 Bun:`Bun.spawn` 喂 stdin JSON;任何失败静默吞掉,宿主零感知。

## 与其他宿主共存

- claude-code-kit / codex-kit 写同一 `.agentdash/`,同一把文件锁;
- 事件按 `host` 字段归因,交错追加、重放折叠语义不变;
- 指令约定单源 `kits/shared/AGENTDASH.md`。
