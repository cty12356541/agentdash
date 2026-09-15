# agentdash · cursor-kit(W9;Cursor 宿主)

把 Cursor 会话事件接进 agentdash 仪表盘(多宿主混用同面板归因)。

项目级钩子:`<repo>/.cursor/hooks.json` → `hooks`(`version:1`;项目级钩子在
Cursor 内信任本工作区后生效,安全边界由宿主强制)。API 依据 Cursor 官方
docs §Agent Hooks(1.7 起)。

| cursor 事件 | agentdash 事件 | 说明 |
|---|---|---|
| `afterShellExecution` | `posttooluse` | 载荷顶层 `command`+`output`,二进制归一成 bash 视图(gate 提取/tool 行) |
| `postToolUseFailure` | `posttoolusefailure` | 失败证据配对:命令可得时与在途 gate 暂存槽顶替,失败侧落真证据(W9-003) |
| `subagentStart` | `subagentstart` | 在跑 agent 面板 dispatched |
| `subagentStop` | `subagentstop` | completed(who 取 `subagent_type`) |
| `stop` | `stop` | 在途 gate 逐槽折叠终态 |

所有命令带 `--host cursor` 归属,混用时面板在跑行显示 `[cursor]`。
`preToolUse`/`postToolUse` 等其余事件暂不注册:前者与 agentdash 的派发行
语义重复(subagentStart 原生更准),后者不含 `command` 字段无法归因 gate。

**如实标注**:Cursor shell 载荷(afterShellExecution)无绿门禁退出码证据
(`command`/`output`/`duration`,无 exit 字段)——`passed` 侧折叠维持
`failed (exit unknown)`(不臆造 0),detail 留 stdout 末行物证;失败侧经
`postToolUseFailure` 配对落真证据,但官方词表未载明该载荷是否携带
`command`(仅 error_message/failure_type/duration/is_interrupt),缺
command 时不可归因、零写入。

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
