# agentdash W8 规格增量——zcode-kit(公司内部宿主)

- 日期:2026-09-15
- 状态:维护者指令"zcode 也要支持(公司内部使用极多)";API 依据 ZCode 官方
  configuration-guide / diagnosing-hooks 技能(本机权威文档,2026-09)
- 性质:新增第四套 kit;内核零改动(`--host` 与词表已通用)

## 1. ZCode 宿主事实(调研结论,实现依据)

- 工作台级钩子配置:`<repo>/.zcode/config.json` → `hooks`,结构
  `{ enabled?, events: { <Event>: [ { matcher?, hooks: [...] } ] } }`;
  **配置文件钩子默认禁用,必须 `hooks.enabled: true`**(安装器负责置位)。
- 事件恰七个:`SessionStart / UserPromptSubmit / PreToolUse / PermissionRequest /
  PostToolUse / PostToolUseFailure / Stop`;**无** SubagentStart/SubagentStop。
- matcher:大小写敏感正则,匹配工具名(`Bash`/`Agent`…,别名 Task↔Agent);
  省略 = 全事件。`type:"command"` 走 shell;`|| true` 双保险语义可用。
- 指令文件:`<repo>/AGENTS.md` 原生加载(与 codex/opencode 同合入路径)。

## 2. 映射与一致性

| zcode 事件 | agentdash 事件 | 说明 |
|---|---|---|
| PostToolUse(全工具) | `posttooluse` | bash 命中验证门 → gate;其余 → tool 行 |
| PreToolUse(matcher `Task\|Agent`) | `pretooluse` | agent dispatched(Agent/Task 别名互通) |
| Stop | `stop` | 在途 gate 折叠 |

所有命令 `--host zcode`;一致性三支柱(C1 同台账同锁 / C2 host 归因 /
C3 AGENTDASH.md 单源)与 W7 完全同构。**如实标注**:zcode 无子代理事件,
在跑 agent 面板对 zcode 宿主不可见;PostToolUseFailure 暂不映射(failed 语义
已由 turn 末折叠承载,避免双计)。

## 3. 设计决策

- **D1 安装器触碰面最小**:config.json 是客户端主配置——安装器只动 `hooks`
  键(节点存在则仅确保 `enabled:true` 并刷新自家事件块),其余键原样;无
  config → fresh 写入(仅含 hooks)。
- **D2 钩子生效时机**:工作台钩子于会话启动加载;已运行会话不热载——安装器
  输出提示"重开会话生效",dogfood 以真实重开验证为准。
- **D3 版本**:新 kit → 收口 bump **0.7.0**(三处 + marketplace),tag `v0.7.0`。

## 4. 非目标

PostToolUseFailure / PermissionRequest / UserPromptSubmit 映射(待使用反馈)、
zcode 插件形态打包(marketplace 发布另议)、GUI 适配。

## 5. 验收

- 安装器:fresh / 二跑幂等(hooks 各恰 1、enabled:true)/ AGENTS.md 三路径
  (created/appended/refreshed);bash -n;执行位 100755;
- config.json 合并不破坏既有键(fixture 含 mcp/skills 时逐键保留);
- 面板 `[zcode]` 归属(载荷回放);README 宿主矩阵加行;
- 三件套全绿;报告入册;0.7.0 + tag + 资产抽验。
