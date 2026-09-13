# agentdash · claude-code 集成包

把 Claude Code 会话事件落成 agentdash 通用任务契约(spec §4.2),供 `agentdash`
TUI 渲染任务 DAG 与验证门状态。hook 只产出数据,投影无副作用。

## 组成

| 文件 | 作用 |
|---|---|
| `hooks/hooks.json` | PostToolUse/Stop/SubagentStop 三钩子注册(插件形态,`${CLAUDE_PLUGIN_ROOT}`) |
| `hooks/record_event.py` | 事件记录器:读 stdin JSON 载荷 → 追加 `<cwd>/.agentdash/events.jsonl` |
| `skills/agentdash/SKILL.md` | `/agentdash` 点播渲染 + 状态跃迁附图约定 |
| `install.sh` / `install.ps1` | 装进目标项目 `.claude/` 并幂等注册 settings.json hooks |

## 安装

```bash
# 类 Unix
./install.sh [目标项目目录]
# Windows PowerShell
.\install.ps1 [-Target <目标项目目录>]
```

安装动作:复制 `record_event.py` → `<目标>/.claude/agentdash/hooks/`、skill →
`<目标>/.claude/skills/agentdash/`;探测 Python(py -3/python/python3)后把三钩子
以绝对路径写入 `<目标>/.claude/settings.json`。幂等:重复执行只刷新自家注册
(以命令含 `agentdash` + `record_event.py` 识别),不动其他内容;损坏的
settings.json 会先备份为 `settings.json.bak-agentdash` 再重建。

插件市场分发时直接用 `hooks/hooks.json`(`claude plugin marketplace add`),无需安装脚本。

## 事件映射(spec §4.2)

| hook | 条件 | 事件 |
|---|---|---|
| PostToolUse | bash 且命令命中 `cargo test`/`clippy`/`fmt`、`go test`、`npm test`、`gh pr checks` | `gate` state=running;退出码+一行摘要暂存 `.agentdash/pending_gate.json`(hook 为一次性进程,折叠须经落盘交接) |
| Stop | 有在途 gate | 折叠为 `gate` passed/failed(exit + detail 摘要行),消费后删除暂存 |
| PostToolUse | 其他任何工具 | `tool` phase=end + exit + summary(命令/描述/文件路径,截 80) |
| SubagentStop | — | `agent` event=completed |

`ts` 为 ISO8601 本地时区;行 JSON `ensure_ascii=False` 原文落盘。

## 降级铁律

入口重配 UTF-8 stdio(中文 Windows 默认 GBK,承 claude-dash 教训);hook 自身
任何失败静默退出 0(`hooks.json` 另带 `|| true` 双保险),绝不阻塞会话;
一切损坏输入(非 JSON、非对象载荷、损坏暂存)降级跳过不抛。

## 测试

```bash
python -m unittest discover -s kits/claude-code/tests -v
```

fixture 回放三钩子样例载荷,断言 events.jsonl 行序与字段(含 gate
running→passed/failed 折叠)、UTF-8 中文往返、损坏输入降级与 hooks.json 清单。
