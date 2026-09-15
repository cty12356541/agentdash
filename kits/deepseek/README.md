# agentdash · deepseek-kit(W9-006;DeepSeek Harness 宿主,桥接形制)

把 DeepSeek Harness(DSH)会话事件接进 agentdash 仪表盘。

## 机制:DSH 桥复用 Claude Code 形制

DSH 的 hooks 体系是桥接架构(官方 `packages/hooks`):`dsh-hooks-claude-code`
包读一份 Claude Code 形制的 `hooks.json`,在 agent 运行 corresponding 时刻执行
其中的命令钩子。本 kit 产出该形制的文件,命令全部带 `--host deepseek` 归属。

| DSH 桥事件 | agentdash 事件 | 说明 |
|---|---|---|
| `PostToolUse` | `posttooluse` | gate 提取/tool 行(载荷为 claude 命令钩子子集) |
| `PreToolUse`(matcher `Task\|Agent`) | `pretooluse` | 子代理派发行 |
| `SubagentStart` | `subagentstart` | 在跑 agent 面板 dispatched(桥原生支持) |
| `SubagentStop` | `subagentstop` | completed |
| `Stop` | `stop` | 在途 gate 逐槽折叠终态 |

面板在跑行显示 `▶ <who> [deepseek]`;gate 折叠语义与 claude-code 宿主一致
(载荷含工具回执,退出码证据有望真实在案)。

## 安装

```bash
./kits/deepseek/install.sh [目标项目目录]
```

行为:AGENTDASH.md 幂等合入 `AGENTS.md`(DSH 为 AGENTS.md 原生)→
`.deepseek/agentdash-hooks.json` 幂等写入五钩子(本 kit 独占文件;他人内容
在场则备份 + 指引)→ `.gitignore` 幂等追加 `.agentdash/`。

**挂载(一次性,进程级)**:在 DSH 组合里登记桥包,`configPath` 指向本 kit
产出的文件(安装器末尾会打印这段):

```yaml
- name: '@deepseek-ai/dsh-hooks-claude-code'
  config:
    configPath: ./.deepseek/agentdash-hooks.json
```

注意(官方 config-catalog 语义):`configPath` 相对路径自 **DSH 进程启动
目录**解析,配置启动时一次读取(项目级 per-session 自动发现是官方 TODO);
钩子在会话工作区内运行,桥自动导出 `CLAUDE_PROJECT_DIR`。

## 如实标注

- 桥未注册 failure 类事件:claude 形制 `PostToolUse` 回执本就带
  工具回执(退出码证据在案),gate 折叠不落 `exit unknown`。
- 一键巡检六 kit:`bash scripts/verify-kits.sh`(仓库根)。
