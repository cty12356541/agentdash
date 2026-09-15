# agentdash · cursor-kit(W9;Cursor 宿主)

把 Cursor 会话事件接进 agentdash 仪表盘(多宿主混用同面板归因)。

项目级钩子:`<repo>/.cursor/hooks.json` → `hooks`(`version:1`;项目级钩子在
Cursor 内信任本工作区后生效,安全边界由宿主强制)。API 依据 Cursor 官方
docs §Agent Hooks(1.7 起)。

| cursor 事件 | agentdash 事件 | 说明 |
|---|---|---|
| `afterShellExecution` | `posttooluse` | 载荷顶层 `command`+`output`,二进制归一成 bash 视图(gate 提取/tool 行) |
| `subagentStart` | `subagentstart` | 在跑 agent 面板 dispatched |
| `subagentStop` | `subagentstop` | completed(who 取 `subagent_type`) |
| `stop` | `stop` | 在途 gate 逐槽折叠终态 |

所有命令带 `--host cursor` 归属,混用时面板在跑行显示 `[cursor]`。
`preToolUse`/`postToolUse` 等其余事件暂不注册:前者与 agentdash 的派发行
语义重复(subagentStart 原生更准),后者不含 `command` 字段无法归因 gate。

**如实标注**:Cursor 载荷(shell 事件)无退出码证据(`command`/`output`/
`duration`,无 exit 字段)——gate 折叠按铁律落 `failed (exit unknown)`,
detail 留 stdout 末行物证,不虚报通过(与 zcode 宿主同辙,台账 W9-03 跟踪)。

## 安装

```bash
./kits/cursor/install.sh [目标项目目录]
```

行为:AGENTDASH.md 幂等合入 `AGENTS.md`(Cursor 1.6+ 原生读取)→
`.cursor/hooks.json` 幂等注册四钩子(只动 hooks 键,`version` 缺失补 1,
其余配置原样)→ `.gitignore` 幂等追加 `.agentdash/`。

- 事件按 `host` 字段归因(`[cursor]` / `[zcode]` / `[codex]` / `[claude]` /
  `[opencode]`);
- 钩子自身任何失败静默退出 0(`|| true` 双保险),绝不阻塞宿主;
- 一键巡检五 kit:`bash scripts/verify-kits.sh`(仓库根)。
