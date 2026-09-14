# agentdash W3 规格增量——验证发现清偿 + 模型扩展

- 日期:2026-09-14
- 状态:维护者当日指令"123 都做 W3 也做,并且注意好远程提交"(范围授权);基线 v1 设计 + W2 T10 验证报告
- 性质:v1 契约**加法扩展**,不改既有字段语义

## 1. 范围(来源:T10 发现 2/5/7/9 + 终审延后清单)

1. **多里程碑模型**(T10 发现 2):ledger 增**可选** `milestones: [{id, title, tasks:[…]}]`;缺省时由 wave+全任务合成单里程碑(向后兼容,v1 台账行为不变)。≥2 里程碑时速度线(tasks/hour)生效——W2-002 的承诺就此可达。
2. **面板屏障行**(发现 5):panel 渲染 barriers(紧凑形态实现者定,验收=可见+可测);graph 已有,不重复实现分层算法。
3. **hook dispatched 入口**(发现 7):kit 增 **PreToolUse**(matcher 覆盖 Task/Agent 工具)→ 提取 agentType/subagent_type → `agent dispatched` 事件;"在跑 agent"画面闭环。
4. **时区一致**(发现 9):页眉 generated_at 与事件 ts 统一本地时区偏移格式。
5. **终审微修清单**:main.rs 注释处数;README git-only 降级补 ⚠ 半句;3 处 fixture 钉 root;F1 负路径断言;图例 ⊘ 槽;fix_round 仅抑匹配前缀(残余 note 保留);空 events 文件与空态引导文案互扰修正。

## 2. 设计决策

- **D1 契约兼容**:milestones 为可选字段,schema(agentdash.tasklog.v1.json)加法更新;旧台账零改动照跑。冲突规则:tasks 同时被 milestones 引用时,以 milestones 分组为准,余任务入"未分组"。
- **D2 空态收紧**:EMPTY_GUIDANCE 仅当**三源文件级皆无**(无 ledger、无 events 文件、无 git);events 空文件=源存在→出缺台账警告,不再同时出"无数据源"引导(消文案互扰)。
- **D3 project 上模型**:Dashboard 增 project 字段(git 仓根名→cwd 目录名→"agentdash"),退役渲染层 cwd 回退;页眉/graph 标题过 elide(宽度钳制内截断)。
- **D4 dispatched 事件**:复用 events.rs 既有 agent 契约(`kind:agent, event:dispatched, who`),who 取载荷 tool_input 的 agentType/subagent_type/name,缺省 "agent"。
- **D5 版本**:W3 收口统一 bump **0.2.0**(Cargo.toml + kits plugin.json + marketplace.json),tag `v0.2.0`,Release 挂 Windows 资产。

## 3. 非目标

远程源扩展、codex/opencode kit、写回操作增强、多仓聚合(仍属 v1 二/三期)。

## 4. 验收

- 全部新行为有 RED→GREEN 测试;148 既有测试不破(除 D2 空态收紧涉及的既有断言,须逐一列出并说明)
- 三件套全绿;dogfood 台账更新至 W3 真值;帧级 spot-check(多里程碑面板含速度线)
- 收口:0.2.0 + tag + Release + 推送远端(推送前核远端,禁 force)
