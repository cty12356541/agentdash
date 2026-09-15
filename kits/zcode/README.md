# agentdash · zcode-kit(公司内部宿主)

把 ZCode 会话事件接进 agentdash 通用任务契约,与 claude-code/codex/opencode
三 kit 写**同一份** `<repo>/.agentdash/`(台账一致性 C1),指令约定同源
`AGENTDASH.md`(harness 一致性 C3,合入 ZCode 原生读取的 `AGENTS.md`)。

## 机制(ZCode 官方 configuration-guide / diagnosing-hooks,2026-09)

工作台级钩子:`<repo>/.zcode/config.json` → `hooks`(配置文件钩子默认禁用,
安装器已置 `enabled: true`;插件钩子则自动启用 runner)。七事件中映射三个:

| zcode 事件 | agentdash 事件 | 说明 |
|---|---|---|
| PostToolUse(全工具) | `posttooluse` | bash 命中验证门 → gate;其余 → tool 行 |
| PreToolUse(matcher `Task\|Agent`) | `pretooluse` | agent dispatched(Task↔Agent 别名互通) |
| Stop | `stop` | 在途 gate 折叠 passed/failed |

所有命令带 `--host zcode` 归属,混用时面板在跑行显示 `[zcode]`。

**如实标注**:zcode 无 SubagentStart/SubagentStop 事件,在跑 agent 面板
(dispatched/completed)对本宿主不可见;`PostToolUseFailure` 暂不映射
(failed 语义已由 turn 末折叠承载)。工作台钩子于**会话启动加载**,安装后
已运行的会话需重开生效。

## 安装

```bash
./install.sh [目标项目目录]
```

行为:AGENTDASH.md 幂等合入 `AGENTS.md` → `.zcode/config.json` 注册三钩子
(**只动 `hooks` 键**,其余配置键原样保留;损坏时备份重建)→ gitignore 追加
`.agentdash/`。需要 `jq` 做既有配置合并(无 jq 时打印手工指引,不吞配置)。

## 与其他宿主共存

- claude-code / codex / opencode kit 写同一 `.agentdash/`,同一把文件锁;
- 事件按 `host` 字段归因(`[zcode]` / `[codex]` / `[claude]`);
- 指令约定单源 `kits/shared/AGENTDASH.md`。

## 降级铁律

agentdash 缺失或任何失败,钩子静默退 0(`|| true` 双保险),绝不阻塞 ZCode。
