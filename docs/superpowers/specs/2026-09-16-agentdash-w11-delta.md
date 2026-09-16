# agentdash W11 规格增量——证据闭环 + 观测面 + 契约公开

- 日期:2026-09-16
- 状态:维护者指令开波("w11开");候选池 04-08 全量
- 基线:v1 设计 + W10 终审(0d30667..a221298,Ready to tag;3 Important 路由本波)

## 1. 范围与执行序(依赖排序)

1. **07 pending 池会话归属**(地基):hook 载荷带 `session_id`(claude 形制既有字段)——pending_gate.json 槽位记录归属会话;Stop 只折叠**本会话**槽位(无 session_id 的遗留槽位保持现行为:首个 Stop 折叠全部,向后兼容);治嵌套会话互相污染退出证据。载荷无 session_id 的宿主:全归 "default" 池,行为同今日。
2. **08 claude 宿主退出码证据**:07 落地后实测排查——本宿主 Bash 载荷的退出码究竟缺在载荷还是缺在提取(split_string_response/对象路径);能修则修,不能修则 README 证据分级句校准(与 W10 终审 Important-3 合并处置)。产出=实证结论入册。
3. **06 无 who completed 配对启发**:回放层把无 who 的 completed 配给**最老在跑**的 dispatched 行;推断配对在 agent 行显式标注(如 `▶⇢✓` 或 done 行标 `(inferred)`);配对类警告保留但降频(每会话一条汇总而非逐行)。
4. **04 agentstats 观测面**:新命令 `agentdash stats [PATH]`——宿主使用率(按 host 字段)/gate 通过率(按门名+宿主)/任务周转(pending→done 时长,依赖 done_at);纯文本表;`--host <name>` 过滤;零 ANSI。
5. **05 契约文档化**:`docs/contract.md` —— 契约 v1 正式公开版(两文件 + config.json 的事件词表/字段语义/门折叠语义/证据分级),版本号化(contract v1.0),README 链接;第三方宿主可照写。

## 2. 顺手清(W10 终审 3 Important + 邻接 Minor,随批)

- N==1 无匹配 ⚠ 抑制:补 3 行修复 + 测试(终审 Important-1,spec 字面偏差)
- --strict 降级源语义:warnings>0 亦退 1(monitor 对坏路径必须红),README 补句
- README「折叠出真实退出码」句:随 08 结论校准
- 邻接 Minor:⑨同词遮蔽测试 ⑩缺 config 专测 ⑥failed 字面量统一(随触即清,不单开任务)

## 3. 设计裁决

- **D1 session 池**:槽位键 = session_id 或 "default";Stop 折叠谓词 = 同池;不迁移旧格式(读取时无 session 字段的槽按 default 归池,语义不变)。
- **D2 stats 只读**:stats 是纯投影,不写任何文件;数据源=events.jsonl(复用 replay),无 events 时输出空态行恒退 0。
- **D3 配对启发可关**:`--no-infer` 旗标关掉启发回到严格丢弃(供不愿看推断行的用户);缺省开启。
- **D4 版本节奏**:收口 bump **0.9.0**(四处),tag v0.9.0,Release 请示制。

## 4. 非目标

编排/云端/Web UI(既定克制);宿主新 kit;06 的跨仓聚合配对;04 的历史趋势图。

## 5. 验收

- 07:嵌套双会话实测互不折叠(真机);无 session_id 载荷行为不变(测试钉)
- 08:实证结论入册(reports/);若修复,claude 宿主自定义 gate 终态可达 passed(活体复验)
- 06:fixture 钉配对/标注/警告降频/--no-infer;panel 黄金不变(无 who 场景的既有钉测试相应更新并列明)
- 04:三表 fixture 钉;--host 过滤;零 ANSI;恒退 0
- 05:contract.md 覆盖两文件+config.json 全词表,版本化,README 链接
- 三件套全绿;verify-kits 六 kit exit 0;收口 0.9.0 + tag + Release(请示制)
