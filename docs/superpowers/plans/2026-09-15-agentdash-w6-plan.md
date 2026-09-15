# agentdash W6 计划(输出可达性收口)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development。批内可并行车道,批间屏障;执行/审查子代理一律 flash 档。
> **Spec 基线:** 2026-09-15 W6 规格增量(范围 = 历次挂账;宿主 kit 列非目标)。
> **开工前:** dogfood 台账换 W6 候选车道;基线 195 测试三件套全绿。

## 批一(解析与设施)

### T1 (W6-001):`±HHMM` 基本格式容忍
**Files:** `src/model.rs`(rfc3339_to_secs 尾缀第三形态)、`tests/merge.rs`(点断言:接受/拒绝矩阵)。
**验收:** spec D1 只加不改;既有断言不破;三件套净。

### T2 (W6-004):proptest 失败回归落盘
**Files:** `tests/proptest_parsers.rs`(FileFailurePersistence::Direct("proptest-regressions"))。
**验收:** 配置生效;6 性质全绿;三件套净。

## 批二(主 feature)

### T3 (W6-002):`render graph --format svg`
**Files:** `src/render/graph.rs`(render_graph_svg:Visual→hex、肘形连线、XML 转义)、`src/main.rs`(parse_render_args 纯函数 + cmd_render 接线)、`README.md` 命令表、`tests/render_graph.rs`(CLI 集成:CARGO_BIN_EXE)。
**验收:** spec D2/D3;svg 合法且含全部任务 id;ansi 缺省逐字节不变;错误路径退 2。

## 批三(审查记录)

### T4 (W6-003):install.ps1 静态审查
**Files:** 无改码(审查结论入 `docs/superpowers/reports/2026-09-15-w6-closeout.md`)。
**验收:** -Depth 10 / 无 BOM / 损坏备份 / 幂等清理 / matcher 一致五项逐条核对。

## 收口

### T5 (W6-005):版本 + 收尾
0.5.0 三处版本;README 路线补 W6;dogfood 台账真值;svg 渲染样张帧证据入册;三件套;终审;推送(核远端);tag `v0.5.0`;资产抽验。
