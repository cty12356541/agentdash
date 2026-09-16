# agentdash W11 计划(证据闭环 + 观测面 + 契约公开)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development。串行派发(R11);执行/审查子代理一律 flash 档。
> **Spec 基线:** docs/superpowers/specs/2026-09-16-agentdash-w11-delta.md(D1-D4 绑定)。
> **Dogfood:** 本会话钩子已活跃(重启后生效),跃迁附 panel;跨会话验证走嵌套。

### 批一(证据地基)

#### T-07 (W11-001):pending 池会话归属
**Files:** `src/hook.rs`(append_pending_slot 记 session_id;pending_slots 读取兼容遗留无字段槽→default 池;on_stop 折叠谓词=同池)、`tests/hook.rs`(双 session fixture:互不折叠;无 session_id 行为不变;遗留格式兼容)。
**验收:** 双 session fixture 互不折叠;遗留/无 session_id 行为字节不变;真机嵌套双会话实证(reports/ 入册);三件套净;单 commit。

#### T-08 (W11-002):claude 宿主退出码证据(排查+修或校准)
**Files:** 视排查结论:`src/hook.rs`(提取修复)或 `README.md`(句校准)+ `docs/superpowers/reports/2026-09-16-w11-exit-evidence.md`(实证入册,必产)。
**验收:** 结论有真机证据;修复路径则活体复验自定义 gate 终态;三件套净;单 commit。
**依赖:** T-07(排除池污染变量)。

### 批二(回放与观测)

#### T-06 (W11-003):无 who completed 配对启发
**Files:** `src/events.rs`(replay 配对:无 who completed → 最老在跑;inferred 标记入 AgentView)、`src/render/panel.rs`(推断标注渲染)、`src/main.rs`+`src/tui.rs`(oneline/watch 的 --no-infer 透传,若架构不便则仅 panel/render 系支持并文档化)、`src/lang.rs`、tests(events 配对/标注/警告降频/--no-infer;W10 既有 who 场景钉测试相应更新并逐条列明)。
**验收:** fixture 钉配对与标注;警告降频;--no-infer 回严格;黄金更新列明;三件套净;单 commit。

#### T-04 (W11-004):agentstats 观测面
**Files:** `src/main.rs`(stats 子命令+--host)、`src/stats.rs`(新:宿主使用率/gate 通过率/任务周转三表,纯文本)、`src/render/mod.rs`(re-export 如需)、`src/lang.rs`、tests/stats.rs。
**验收:** 三表 fixture 钉;--host 过滤;零 ANSI;无 events 空态恒退 0;顺手清 N==1 ⚠ 修复同批;三件套净;单 commit。

### 批三(契约公开)

#### T-05 (W11-005):契约文档化
**Files:** `docs/contract.md`(新:契约 v1.0——ledger/events/config.json 全词表、字段语义、门折叠语义、证据分级表)、`README.md`(链接)、`schema/`(如需微调与文档对齐)。
**验收:** 全词表覆盖(以 src/contract.rs + src/events.rs 为对账基准);版本化;README 链接;纯文档零代码;单 commit。

### 收口

#### T-C (W11-006):0.9.0 收口
版本四处+lock;README 路线 W11 完成态;--strict 降级语义 README 落句(若 T-08 未覆盖);dogfood 台账换 W12 候选;三件套+verify-kits;全分支终审;推送;tag v0.9.0;Release 请示制。
