# agentdash W4 规格增量——评估清偿:保守化 + 债收敛 + 分发自动化

- 日期:2026-09-15
- 状态:维护者指令"先出优化计划"(2026-09-14 深度评估报告为范围来源);**两项裁定项默认不做**(§3),维护者一句话授权即可插队
- 性质:**一处行为修正**(gate 折叠保守化)+ 一处模型加法(event_tail)+ 其余零行为变更(重构/测试/分发)

## 1. 范围(来源:2026-09-14 深度评估 P1–P3 清单)

1. **gate 折叠保守化**(评估 P1):`agentdash hook stop` 折叠在途 gate 时,退出码**不可知**(response 缺失/非对象/无任何退出码字段)现按 0 记 `passed`——与全项目"不虚报"立场相悖。改为记 `failed`,detail 补 `(exit unknown)` 尾注;显式 `exit 0` 仍 `passed`,`interrupted→130` 与非零退出路径不变。
2. **详情面板事件尾接入**(评估 P2):详情卡"事件"区现为占位行(`事件层 W2-008 接入`,引用车道号已失义)。`Dashboard` 增 `event_tail` 只读投影(最近 10 条 agent+gate 事件,到达序),详情渲染真实 tail;不改 events.jsonl 落盘契约。
3. **渲染截断收敛**(评估 P3):`panel.rs` 私有 `truncate_width`/`elide` 删除,并入 `render::mod` 公共版(注释里"批三双轨收敛"的未竟事项)。
4. **子进程执行器合一**(评估 P3):`sources/git.rs::run_git` 与 `sources/remote.rs::run_gh` 的"spawn + 读线程 + 轮询超时"双胞胎提取公共 `run_capture(dir, program, args, timeout)`。
5. **dogfood note 复现性**(评估 P2):dogfood 台账 note 现断言"events.jsonl 在场、速度行点亮",但事件产物不入库,新 clone 必不出速度行——措辞修正为"仅会话期成立,新 clone 不出属预期降级"。
6. **解析器 property 测试**(评估 P3):手搓 RFC 3339 / Hinnant 民法 / 词序列匹配是全仓最脆弱面,引入 **dev-dependency** `proptest`(不进发布二进制,不违背零运行时依赖哲学)做性质测试。
7. **Release 自动化**(评估 P3):bin/README 承认"release 资产后续手动挂"——新增 tag 触发的三平台构建 workflow,资产命名与 `agentdash-setup` SKILL 指引对齐,退役"手动挂"措辞。

## 2. 设计决策

- **D1 gate 保守化(fail-closed)**:`exit_code()` 在取不到任何退出码证据时返回语义值"未知"(不再是 0);Stop 折叠:显式 0 → `passed`,非零/interrupted(130)→ `failed`,**未知 → `failed` + detail 尾注 `(exit unknown)`**。理由:验证门的可信度高于完备性,载荷形态漂移时宁可误报失败也不虚报通过。README"数据契约"段与 kits README 事件映射表同步一句。旧 `pending_gate.json` 单对象格式兼容路径不破(既有断言钉住)。
- **D2 event_tail 上模型**:`EventModel` 重放时保留**全部已知 kind 中 agent+gate 两类**的紧凑行(`kind / name(who 或 gate)/ state / ts 原串`),到达序,取最近 10 条;`Dashboard::event_tail` 投影,`detail_lines` 渲染;`tool` 事件不入 tail(计数已另承)。**过滤一致性裁定**:详情恒查全量模型(`dash.tasks` 原表),与主视图的过滤/折叠/波次折算解耦——聚焦是用户显式动作,不随视图折算丢失;此裁定写入 `render_detail` 注释。
- **D3 纯重构零行为变更**:W4-003/004 两条收敛任务各自单 commit,既有 177 测试全数不破;输出逐字节不变(以既有 render 断言为黄金对照)。
- **D4 性质测试面**:`utc_timestamp ∘ rfc3339_to_secs` 往返;`civil_from_days`/`days_from_civil` 互逆(Windows 分支经 cfg 引入测试);`parse_fix_round` 残余性质;`match_word_seq` 对样例矩阵 + 随机空白注入的不变式。失败案例 shrinking 后钉为点断言回归。
- **D5 Release 链**:tag `v*` 触发;matrix 构建 `x86_64-unknown-linux-gnu`、`x86_64-apple-darwin`、`aarch64-apple-darwin`、Windows triple;**现 setup SKILL 写 `windows-gnu`,workflow 默认出 `msvc`——以实跑通过者为准回改文档,禁止文档与资产各说各话**;资产名 `<name>-<tag>-<triple>.<ext>`;首跑以测试 tag 验证下载件 `--version` 自检过。
- **D6 版本**:收口统一 bump **0.3.0**(gate 折叠是可见行为变更 + event_tail 模型扩展,minor 合理;不取 0.2.2)。三处同步:Cargo.toml / kits plugin.json / marketplace.json。

## 3. 非目标(本轮明确不做)

- **裁定项 A**:EAW 子集表 → `unicode-width` crate(`render/mod.rs` TODO(deps))。默认保留子集表:换表增发布依赖,收益是罕见区段宽度正确——维护者裁定。
- **裁定项 B**:CI 供应链检查(cargo-deny / cargo-audit)。默认不做;要做则起步 warn 不阻塞。
- 沿用非目标:codex/opencode kit、远程源扩展、多仓聚合;另新列:events 轮转多代化(单代是文档声明取舍)、`contract_tasks` O(n²)(当前规模不值)。

## 4. 验收

- 全部行为变更(D1/D2)先红后绿;**177 既有测试为基线不破**(D1 涉及的既有 passed-by-default 断言须逐一列出并说明改判理由)
- 三件套全绿(含 property 测试);纯重构任务输出逐字节黄金对照
- 帧 spot-check:详情卡出现真实事件尾;dogfood 台账更新至 W4 真值(含 note 措辞修正样例)
- 收口:0.3.0 + tag + Release 资产 workflow 首跑通过 + 推送远端(推送前核远端,禁 force)
