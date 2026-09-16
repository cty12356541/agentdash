# agentdash W10 计划(0.8 产品化)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development。批内串行派发(R11 先例:同仓并发撞 index 锁);执行/审查子代理一律 flash 档。
> **Spec 基线:** docs/superpowers/specs/2026-09-16-agentdash-w10-delta.md(D1-D4 裁决绑定)。
> **Dogfood 纪律:** 跃迁附 panel;本控制器会话钩子不活跃(先于插件安装启动),事件流验证走嵌套会话(R5 先例),如实披露。

### T1 (W10-001):多仓聚合——panel 多 PATH + glob + 精要视图
**Files:** `src/main.rs`(parse_render_args 接受 N 位置参数+glob 展开调用量;cmd_render 单仓走原路/多仓走精要)、`src/render/panel.rs`(新 pub render_brief(dash,width)->String:页眉统计/在跑 agent/失败门/blocked/⚠,缺项零残留)、`src/render/mod.rs`(flat re-export)、tests(render_panel 增:单仓黄金字节不变;精要四要素;空仓 ⚠;新 tests/glob.rs:分量级 `*` 展开含 CJK、无匹配 ⚠)。
**验收:** 单仓字节不变黄金;N 仓精要齐四要素;glob 自展开(shell 无关);无匹配 ⚠ 恒退 0;三件套净;单 commit。

### T2 (W10-002):离场摘要——render digest
**Files:** `src/main.rs`(digest 臂+--strict 解析;多 PATH→用法错退 2)、`src/render/digest.rs`(新:纯文本,节=页眉统计/失败门 ✗+detail/在跑 agent/blocked/done 含 ?物证/⚠;oneline 式无 ANSI)、`src/render/mod.rs`(re-export)、tests/digest.rs(零 ESC 断言;节内容 fixture 钉;--strict 两态退出码;多 PATH 退 2)。
**验收:** 输出零 ESC;节与数据源逐项对账;--strict failed/blocked→1 否则 0;三件套净;单 commit。

### T3 (W10-003):用户自定义 gate——.agentdash/config.json
**Files:** `src/hook.rs`(load_custom_gates(dir)->Vec 静默读取;gate_name_with(command,custom);用户表先查;形状/越界→整表回退)、tests/hook.rs(用户词表命中/损坏回退/优先级/上限裁断)、tests/proptest_parsers.rs(自定义表不变式)、README.md(契约段 config.json,标注取代 v1 config.toml 散文)。
**验收:** fixture 回放命中用户 gate 名;损坏 config 静默仅内置(钉死);用户优先;verify-kits.sh 六 kit exit 0;三件套净;单 commit。

### T4 (W10-004):收口——0.8.0 + dogfood 活体 + 发布
版本三处+lock 0.8.0;README 路线 0.8 完成态;本仓 .agentdash/config.json 声明活体 gate(如 cargo-bench)经嵌套会话真跑落帧;dogfood 台账换 W11 候选;三件套;全分支终审;推送;tag v0.8.0;Release 资产(请示制)。
