# agentdash W3 计划(验证发现清偿 + 模型扩展)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development。批内可并行车道,批间屏障;执行/审查子代理一律 flash 档。
> **Spec 基线:** 2026-09-14 W3 规格增量(维护者当日授权)。

## 批一(微修收口——单实现者一批)

### T1 (W3-001):终审微修批(7 项)
**Files:** `src/main.rs`(注释处数 3→4)、`README.md`(git-only 降级句补 ⚠ missing 半句)、`tests/render_panel.rs`×2+`tests/render_graph.rs`×1(fixture 钉 `root: Some(...)` 去 cwd 依赖)、`tests/merge.rs`+`tests/render_panel.rs`(F1 负路径:有台账无 events 不出 missing 警告;台账在但不可读仅 unreadable 一条)、`src/render/panel.rs`(图例增 `⊘{blocked}` 槽,统计行口径不变)、`src/model.rs`(parse_fix_round 返回匹配区间,面板仅抑前缀保留残余 note)、`src/model.rs`(D2 空态收紧:三源文件级皆无才 EMPTY_GUIDANCE;空 events 文件=有源→只警告)。
**验收:** 行为变更(图例/残余 note/空态收紧)先红后绿;列出 D2 触及的既有断言清单及理由;三件套净;单 commit。

## 批二(功能补齐)

### T2 (W3-002):面板屏障行
**Files:** `src/render/panel.rs`(车道区渲染 barriers 紧凑行,形态实现者定)、`tests/render_panel.rs`(fixture:2 屏障可见且可断言)。
**验收:** B1/B2 在 panel 可见;与 graph 语义一致(after→unlocks 对);三件套净。

### T3 (W3-003):PreToolUse dispatched 入口
**Files:** `kits/claude-code/hooks/hooks.json`(增 PreToolUse,matcher Task/Agent)、`src/hook.rs`(pretooluse:tool 名匹配→提取 who→dispatched 事件;降级铁律照旧)、`kits/claude-code/README.md` 事件映射表、`tests/hook.rs`(fixture 回放断言)。
**验收:** Task/Agent 派发→events.jsonl 出 dispatched 行(who 正确);非 agent 工具不产事件;静默退 0;hooks.json 清单测试同步。

## 批三(模型扩展)

### T4 (W3-004):多里程碑 + 速度线 + project 上模型
**Files:** `src/contract.rs`+`schema/agentdash.tasklog.v1.json`(可选 milestones,加法)、`src/model.rs`(里程碑分组/缺省合成/D1 冲突规则;Dashboard.project 字段 D3;速度线 ≥2 里程碑生效)、`src/render/panel.rs`(轨迹区多里程碑 + 速度行;页眉/graph 标题 elide)、`src/render/mod.rs`(退役 cwd 回退)、`tests/contract.rs`/`tests/merge.rs`/`tests/render_panel.rs`。
**验收:** v1 旧台账输出不变(黄金对照);milestones 台账分组正确;≥2 完成里程碑出速度行;页眉 project 为仓名且超宽截断;三件套净。

## 收口

### T5 (W3-005):版本 + 收尾
0.2.0 三处版本号;README 路线/契约段更新;dogfood 台账更新 W3 真值(多里程碑样例入 dogfood);帧 spot-check(速度线可见);三件套;全分支终审;推送(先核远端);tag `v0.2.0`;Release Windows 资产(git credential token 走 API,或装 gh)。
