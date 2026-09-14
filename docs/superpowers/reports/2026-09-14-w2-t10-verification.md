# W2 T10 双场景验证报告(精简版)

- 日期:2026-09-14 · agentdash 0.1.0 @ 145e3dc(feat/w2-t9t10,后并 main)
- 方法:帧捕获(`watch --once` / `render panel|graph` / `oneline`,ANSI+纯文本双份)+ 逐元素对账;原始证据 26 文件存于本地 `.superpowers/sdd/2026-09-14-agentdash-w2-plan/evidence/`(git 忽略区,不入库)
- 披露:场景一事件为 fixture 载荷回放(钩子中途安装不激活当值会话);场景二为全新模拟仓;cargo test(143/0)与 clippy(0 警)数字为真机实跑

## 场景一(dogfood,本仓真台账)

W2 台账(4 车道 10 任务 2 屏障)+ 7 行回放事件 + git HEAD。页眉统计/健康三门(含 1 模拟失败门)/轨迹进度条/车道任务行/DAG 分层与屏障边/oneline 计数**逐项对账通过**;`watch --once` 与 `render panel` 输出字节相同。

## 场景二(独立模拟仓 + 降级)

git init 仓 + R1 台账(3 车道 8 任务含 1 blocked、2 屏障)+ 8 行事件。四帧对账通过;降级双验:
- **删台账**:exit 0 不白屏,git 伪任务链兜底,事件层照常 —— 但**无警告行**(见发现 1)
- **坏台账**:exit 0,`⚠ corrupt ledger.json: …` 警告行出现

## 发现与处置

| # | 发现 | 处置 |
|---|---|---|
| 1 | 契约**缺失**无警告行,违背 spec AD-ERR-001(损坏才有 ⚠) | ✅ 已修(2b9fe58):`⚠ missing ledger.json`,仅 git 仓亦出 |
| 3 | 页眉项目名硬编码 agentdash | ✅ 已修(2b9fe58):git 仓根名→cwd→兜底 |
| 6 | blocked 与 pending 同形;fix-round note 与 R 尾缀重复 | ✅ 已修(2b9fe58):`⊘` 独立符号;解析后不重复出 note 原文 |
| 2 | 单台账恒 1 里程碑 → W2-002 速度线不可达 | → W3(多里程碑模型) |
| 5 | 面板不画屏障行(panel.rs TODO) | → W3 |
| 7 | hook 无 dispatched 入口,"在跑 agent"出不了画面 | → W3(PreToolUse 思路) |
| 9 | 页眉 UTC 与事件本地时区并存 | → W3 顺手 |

## 结论

两场景全部画面元素可追溯至三源数据,无"图与数据冲突";降级铁律(缺/坏恒退 0 不白屏)实测成立。W2 验收通过;未决项全部移交 W3。
