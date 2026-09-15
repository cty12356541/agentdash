# agentdash · codex-kit(OpenAI Codex CLI)

把 Codex CLI 会话事件接进 agentdash 通用任务契约,与 claude-code-kit 写**同一份**
`<repo>/.agentdash/`(台账一致性 C1),指令约定同源 `AGENTDASH.md`(harness 一致性 C3)。

## 机制(2026-09 调研)

Codex CLI 的 lifecycle hooks:`<repo>/.codex/hooks.json`(repo 级,项目需 trust,
命令钩子需 `/hooks` 一次性审查),payload 走 **stdin JSON**,公共字段含
`session_id` / `cwd` / `hook_event_name` / `tool_name` / `tool_input` /
`tool_response`——与 agentdash hook 的输入契约同构,零转换直喂。

| codex 事件 | agentdash 事件 | 说明 |
|---|---|---|
| PostToolUse | `posttooluse` | bash 命中验证门 → gate;其余工具 → tool 行 |
| SubagentStart | `subagentstart` | agent dispatched(who 取 agent_type) |
| SubagentStop | `subagentstop` | agent completed |
| Stop | `stop` | 在途 gate 折叠 passed/failed |

所有命令带 `--host codex`——多宿主混用时事件可按宿主归因,面板在跑行显示
`▶ <who> [codex]`。

## 安装

```bash
./install.sh [目标项目目录]
```

行为:AGENTDASH.md 标记段幂等合入 `AGENTS.md` → `.codex/hooks.json` 注册
(已存在他人钩子时**不吞用户文件**,备份 + 打印合并指引)→ gitignore 追加
`.agentdash/`。

## 与其他宿主共存

- claude-code-kit / opencode-kit 写同一 `.agentdash/`,同一把文件锁;
- 事件按 `host` 字段归因,混用时交错追加、重放折叠语义不变;
- 指令约定单源 `kits/shared/AGENTDASH.md`,三套安装器合入各自的指令文件。

## 降级铁律

agentdash 缺失或任何失败,钩子静默退 0(`|| true` 双保险),绝不阻塞 Codex。
