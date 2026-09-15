---
name: agentdash-setup
description: Install and verify the agentdash binary that this plugin's hooks call. Use when `agentdash --version` fails, when the dashboard shows no events, or when the user asks to install/setup/修复 agentdash. Guides PATH placement per OS, runs the self-check, and confirms hooks will record events.
---

# agentdash 安装引导 / SETUP

插件的四个钩子(PreToolUse/PostToolUse/Stop/SubagentStop)直调 PATH 上的
`agentdash` 二进制。二进制缺失时钩子按降级铁律静默跳过——**不报错、但也不记事件**。
本技能引导完成安装与自检。

## 步骤

1. **探测**:`agentdash --version`(bash 直接跑)。输出 `agentdash 0.x.y` 且退 0 → 已装,跳到第 4 步
2. **安装**(按平台):
   - **Windows**:从 [Releases](https://github.com/cty12356541/agentdash/releases) 下载
     `agentdash-v<x>-x86_64-pc-windows-msvc.zip`(W4-006 起资产由 Release workflow
     自动挂载,triple 与之一致),解压 `agentdash.exe` 放入 PATH 目录
     (如 `%LOCALAPPDATA%\Programs\agentdash`,或在 PowerShell 里:
     `New-Item -ItemType Directory -Force "$env:LOCALAPPDATA\Programs\agentdash"; Expand-Archive <zip> -DestinationPath 上述目录 -Force`,再把该目录加入用户 PATH 后重开终端)
   - **macOS / Linux**:`cargo install --path <仓库根>`(Rust ≥1.88,源码用
     let-chains),或 Releases 对应
     target triple 压缩包解压入 `/usr/local/bin`
   - 仓库地址:https://github.com/cty12356541/agentdash
3. **自检(必过)**:`agentdash --version` 退 0;失败则检查 PATH 与解压完整性,勿继续
4. **钩子验证**:在当前项目目录构造一次真实工具回调(如在会话里跑一条 bash 命令),
   然后 `agentdash oneline` —— 出现 `[dash] <project> …` 即事件管线已通
5. **引导台账(可选,深度语义)**:无 `.agentdash/ledger.json` 时面板会显示
   `⚠ missing ledger.json` 与 git 伪任务链。可按仓库 README 数据契约 v1 帮用户起草
   (wave/title/lanes/tasks/barriers/milestones),起草后 `agentdash render panel` 核验

## 诚实约束

只装二进制与验证,不改宿主其他配置;一切失败如实报告版本与 PATH 状态,不得谎报已装。
