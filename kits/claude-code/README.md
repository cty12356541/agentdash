# agentdash · claude-code 集成包

把 Claude Code 会话事件落成 agentdash 通用任务契约(spec §4.2),供 `agentdash`
TUI 渲染任务 DAG 与验证门状态。hook 只产出数据,投影无副作用。

## 前提

**`agentdash` 二进制在 PATH**——hook 直调二进制子命令,零 Python/Node 等运行时
前置(spec §6 修订)。缺失时安装:

```bash
cargo install --path <agentdash 仓库根目录>   # 或从 Releases 下载对应平台二进制放入 PATH
agentdash --version                           # 自检
```

## 组成

| 文件 | 作用 |
|---|---|
| `hooks/hooks.json` | PostToolUse/PostToolUseFailure/PreToolUse/Stop/SubagentStop 五钩子注册(插件形态,直调 `agentdash hook <event>`) |
| `skills/agentdash/SKILL.md` | `/agentdash` 点播渲染 + 状态跃迁附图约定 |
| `install.sh` / `install.ps1` | 装进目标项目 `.claude/`:幂等注册 settings.json hooks + `.gitignore` 幂等追加 `.agentdash/` |
| 仓库 `tests/hook.rs` | hook 套件:Rust 集成测试(fixture 回放 + 并发零丢失),Python 测试随垫片一并退役 |

## 安装

### 方式 A:安装脚本(装进目标项目 `.claude/`)

```bash
# 类 Unix
./install.sh [目标项目目录]
# Windows PowerShell
.\install.ps1 [-Target <目标项目目录>]
```

### 方式 B:插件市场(Claude Code)

```bash
cargo install --path <agentdash 仓库根>   # 前置:二进制已在 PATH
claude plugin marketplace add cty12356541/agentdash
claude plugin install agentdash@agentdash-marketplace
```

插件体即本目录(`hooks/hooks.json` 五钩子 + `skills/agentdash/`),不用安装脚本、
不改目标项目 settings.json。**注意**:hooks.json 的 `agentdash hook <event>` 直调
PATH 上的二进制,市场包**不内嵌二进制**——缺二进制时 hook 按降级铁律静默跳过,
需先 `cargo install --path` 或从 Releases 下载放入 PATH(Windows release 资产
后续手动挂;预构建分发策略见仓库 `bin/README.md`)。

安装动作:检测 `agentdash` 在 PATH(缺失给安装指引并退出)→ skill 复制到
`<目标>/.claude/skills/agentdash/` → 五钩子以固定命令 `agentdash hook <event> || true`
幂等写入 `<目标>/.claude/settings.json`(**无路径 baked**;老版本 `record_event.py`
的注册与文件残留一并清理)→ 目标仓 `.gitignore` 幂等追加 `.agentdash/`。

幂等:重复执行只刷新自家注册(以命令含 `agentdash hook` 识别;旧 `record_event.py`
注册同视为自家并替换),不动 settings.json 其他内容;损坏的 settings.json 先备份为
`settings.json.bak-agentdash` 再重建。`install.sh` 合并既有 settings.json 需要 `jq`
(无 jq 时不改用户文件,打印手工合并指引;settings.json 不存在则最小重建,无需 jq);
`install.ps1` 用 PowerShell 原生 JSON,无任何额外依赖。

## 事件映射(spec §4.2)

| hook | 条件 | 事件 |
|---|---|---|
| PreToolUse | Task/Agent 工具派发(matcher `Task\|Agent`) | `agent` event=dispatched(`who`=tool_input 的 agentType/subagent_type,缺省 `agent`;`task`=description 截 80 字符,缺省省略该字段;其余工具零写入静默) |
| PostToolUse | bash 且命令命中 `cargo test`/`clippy`/`fmt`、`go test`、`npm test`、`gh pr checks`(词边界 + 词间空白) | `gate` state=running;退出码+一行摘要暂存 `.agentdash/pending_gate.json`(hook 为一次性进程,折叠须经落盘交接) |
| Stop | 有在途 gate | 折叠为 `gate` passed/failed(exit + detail 摘要行;退出码不可知记 failed + `(exit unknown)` 尾注,不虚报),消费后删除暂存 |
| PostToolUse | 其他任何工具 | `tool` phase=end + exit + summary(命令/描述/文件路径,截 80 字符) |
| SubagentStop | — | `agent` event=completed(载荷带 `agent_name` 则透传 `who`) |

`ts` 为 ISO8601 本地时区秒级(如 `2026-09-13T21:00:00+08:00`);中文等非 ASCII 原文落盘。

## 降级铁律

hook 自身任何失败(损坏/空 stdin、非 UTF-8 字节、非对象载荷、损坏暂存、IO 错误)
一律静默退出 0(`hooks.json` 另带 `|| true` 双保险),绝不阻塞会话、绝不向宿主报错。

**并发安全**:多个 hook 进程并发追加同一 events.jsonl,经 `.agentdash/.lock` 文件锁
自旋(`create_new` 先到先得,上限 2s 重试后按降级铁律退化直接写;单行整写一次
`write_all`)。8 线程 × N 行并发灌入零丢失由 `tests/hook.rs` 钉住。

## 测试

hook 套件是仓库的 Rust 集成测试(子进程回放真实二进制):

```bash
cargo test --test hook
```

覆盖:五钩子 fixture 回放(含 gate running→passed/failed 折叠、Task/Agent 派发
dispatched 行)、gate 命令匹配表
(词边界/多空白/复合命令)、退出码与摘要提取变体、UTF-8 中文往返、损坏输入降级、
事件名回退分派、hooks.json 清单(五事件 + matcher + 二进制直调 + `|| true`)、并发零丢失。
