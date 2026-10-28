# agentdash W12 规格增量——打磨清算 + 监控补强 + 契约 v1.1 + 生态

- 日期:2026-09-16
- 状态:维护者四轨全选("A/B/C/D 都要")
- 基线:v0.9.0(W11 已发);延后池 W10×12 + W11×21 = 33 条

## 1. 范围

### A 打磨波(延后 Minor 清算,一批一实现者)
从 33 条中挑用户可感/有回归价值的子集(纯报告级计数笔误类不清):①stats CJK 列宽(复用 render::display_width)②--host 匹配敏感性钉测试③inferred 行完成时刻(AgentEntry 增完成 ts,行显示完成时刻而非派发时刻)④汇总警告随 AGENTDASH_LANG 中英⑤oneline 混参报错文案统一(unexpected_flag/extra 归一)⑥契约文档三句补(§3.3 折叠计数单位=每次终态/§4.4 槽侧空白同无字段/§2.5 重复引用警告行为)⑦README 快览"零丢失"句按契约§1 口径校准。

### B 监控与观测补强
①--strict 降级源退 1:warnings>0(缺契约/坏契约/坏事件行)→ 退 1(monitor 完整性;行为变更,README --strict 句同步反转,digest 测试更新)②stats ③表无 events 时也渲染(去早退,空 ①② 表照常)③watch 支持 --no-infer(透传 tui)。

### C 契约 v1.1(started_at)
task 增可选 `started_at`(ISO8601,三格式容忍同 done_at);stats ③ 周转表:有 started_at+done_at → 真时长(中位/最大/均值三数);部分有 → 分列;全无 → 现状 N/A。schema/contract.md 同步 v1.1(加法,旧台账零影响黄金钉)。

### D 生态与分发
①官方目录提交材料刷新至 0.9.0(README/plugin.json 元数据复核,validate 零警告,给维护者的填表清单——提交仍是维护者网页动作)②codex/opencode 退出码证据实测:本机可装则装真测,装不了如实维持"未实测"分级(不虚构)③cursor e2e:依赖维护者 cursor-agent login,材料备好即止(blocker 如实标注)。

## 2. 裁决

- D1 打磨批单 commit 批量清(同形小修,一审查面);行为变更项(③④⑤)各带 RED
- D2 B① 行为反转属规格明令:README 既有"不计入"句随同反转;--strict 语义=failed/blocked/降级警告 任一 → 1
- D3 契约版本 1.1 与 crate 版本独立;C 落地后 contract.md 版本头 bump 1.1 + 变更记事
- D4 收口版本:默认 **0.10.0**;若维护者欲立 1.0 里程碑,收口时改(一句话的事)
- D5 D 轨产出以"材料+指引+实测报告"为验收,不含代理提交表单/代理 login

## 3. 非目标

新宿主 kit;编排/云端/Web;契约其他字段;A 批纯报告级笔误类条目。

## 4. 验收

- A:所列 7 项各带测试/文档钉;三件套净
- B:①strict 三态钉(failed/blocked/warnings→1;净→0)②无 events 帧渲染钉 ③watch --no-infer 端到端
- C:有 started_at 夹具三数钉;旧台账黄金不变;contract v1.1 记事
- D:validate 零警告;填表清单入册;实测报告(或如实"未实测"维持)
- 全波:三件套 + verify-kits;终审;收口 0.10.0;tag/Release 请示制
