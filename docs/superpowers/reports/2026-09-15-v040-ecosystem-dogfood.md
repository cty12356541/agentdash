# agentdash v0.4.0 发布件生态 dogfood 验证报告

- 日期:2026-09-15
- 对象:**GitHub Release 下载的 v0.4.0 aarch64 资产**(非本地构建)+ `install.sh`
- 场景:全新 git 项目,模拟真实用户自 setup skill 起步的完整剧本
- 结论:**端到端通过;发现并修复 1 处发布链路缺陷(F1)**

## F1(本报告唯一缺陷,已修)

`kits/claude-code/install.sh` 在 git 内为 100644(无可执行位),README 承诺的
`./kits/claude-code/install.sh` 直接执行报 permission denied——克隆用户必撞。
修复:git index `--chmod=+x`(100755,本报告同 commit 入库);`install.ps1` /
`dash-side.bat` 经由解释器调用,不需位。

## 验证矩阵

| 环节 | 结果 |
|---|---|
| 资产下载 → `--version` | `agentdash 0.4.0`,内件恒名 `agentdash` |
| install.sh 首跑 | fresh 路径:settings.json 四钩子注册 + skill 复制 + gitignore 追加 |
| install.sh 二跑 | merged 路径,幂等:各钩子**恰 1** 条,无重复注册 |
| 事件管线(PostToolUse ×3 / PreToolUse / SubagentStop / Stop) | 全部落盘;Edit 行 `exit:null`(W4-001 语义);cargo-test 同门**双槽暂存**折叠为 failed+passed(后到覆盖);agent dispatched/completed 配对 |
| 无台账降级 | `⚠ missing ledger.json` + git 伪任务单链(2 commits)+ gate 上板;`watch --once` 非 tty 出整帧(AD-ERR-004 路径) |
| 起草台账(深语义) | 车道/状态正常;**物证标记在新项目同样精准**:T1(done_at 早于通过门)无标、T2(晚于锚)打 `?`、active 无标 |

## 未覆盖(如实)

`c` 键聚焦投递(tmux send-keys / prompt.txt)为交互路径未实按;`install.ps1`
与 Windows 全链路在本机(macOS)不可测;宿主为真实 Claude Code 会话时的
hook 触发由 W2 T10 双场景报告承袭。

## 记录

- 全程事件与面板产物在 `/tmp/agentdash-dogfood/proj`(临时目录,不入库);
- 本仓 `.agentdash/events.jsonl` 同期累积真实 gate(clippy/test passed)。
