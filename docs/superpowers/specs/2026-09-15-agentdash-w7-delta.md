# agentdash W7 规格增量——多宿主 kit(codex / opencode)与跨宿主一致性

- 日期:2026-09-15
- 状态:维护者指令"各种宿主版本都要,考虑台账一致性与 harness 一致性";API 依据
  2026-09 调研(Codex lifecycle hooks:repo 级 `.codex/hooks.json`、stdin JSON 载荷
  含 cwd/tool_name/tool_input、`SubagentStart/SubagentStop` 原生事件;opencode:
  `.opencode/plugins/*.js`、`tool.execute.after`、`session.idle`)
- 性质:内核加法(事件可选 `host` 归属 + `subagentstart` 事件名)+ 两套新 kit +
  claude-code kit 归属补齐;**台账与 harness 跨宿主单源**

## 1. 一致性设计(用户核心约束)

- **C1 台账一致性 = 同文件同锁同词表**:三宿主 kit 全部写 `<repo>/.agentdash/`
  同一台账与事件流,共用 `.agentdash/.lock`;各家宿主原生事件**映射**到
  agentdash 规范四词表(posttooluse/pretooluse/stop/subagentstop)+ 新增
  `subagentstart`(agent dispatched 的直接入口,Codex 原生事件对齐)。混用
  工具时事件交错追加于同一 events.jsonl,渲染按既有重放语义折叠。
- **C2 宿主归属 = 事件可选 `host` 字段**:`agentdash hook --host <name> <event>`
  (claude/codex/opencode);hook 落盘的每条事件带 `host`;agent 活跃表携带
  首次派发的 host,面板在跑行显示 `[host]`。未传旗标 → 字段省略(向后兼容)。
- **C3 harness 一致性 = 单一真源 `AGENTDASH.md`**:跃迁约定 + 诚实约束 +
  数据契约指针写一份(`kits/shared/AGENTDASH.md`),三套安装器以**标记段
  幂等合入**宿主各自的指令文件(Claude Code → `CLAUDE.md`;codex/opencode →
  `AGENTS.md`,两家原生读取)。约定改一处,三宿主同步。

## 2. 各宿主映射(spec 调研依据)

| 宿主 | 原生机制 | 映射到 agentdash |
|---|---|---|
| claude-code(既有) | hooks.json 四事件 | posttooluse / pretooluse(Task\|Agent)/ stop / subagentstop;`--host claude` 补齐 |
| codex | `.codex/hooks.json`(repo 级,需 trust + `/hooks` 审查;matcher 省略=全事件) | PostToolUse(全工具)→ posttooluse;SubagentStart → **subagentstart**;SubagentStop → subagentstop;Stop → stop;均 `--host codex` |
| opencode | `.opencode/plugins/agentdash.js` | `tool.execute.after` → posttooluse(tool 小写名,agentdash 已大小写容忍);`session.idle` → stop;均 `--host opencode`;子代理边界:宿主插件 API 暂无原生事件 → 如实标注为待宿主能力 |

## 3. 设计决策

- **D1 `--host` 旗标**:`resolve_event` 前解析(旗标与事件名顺序容忍);
  `host` 仅在显式传入时落盘;subagentstart → `agent dispatched`(who 取
  `agent_type`/`agent_name`/`subagent_type`/`who` 首个非空,缺省 `agent`)。
- **D2 agent 活跃表带 host**:dispatched 首见的 host 入表(再派刷新 task 注记
  不改 host);completed 移除。`AgentView.host: Option<String>`,面板在跑行
  `▶ <who>[ · <host>][ · <task>]…`。
- **D3 codex 安装器保守策略**:repo 级 `.codex/hooks.json` 若已存在且含
  agentdash → 仅刷新自家块(TOML/JSON 混层不由安装器猜测,给出手工指引);
  已存在但无 agentdash → 备份后打印合并指引,不吞用户文件;不存在 → fresh。
  并明确提示:repo 级 hooks 需 Codex trust + `/hooks` 一次性审查。
- **D4 opencode 插件降级铁律**:插件内任何异常(Bin 缺失/进程失败)一律
  静默吞掉,绝不阻塞宿主工具执行;stdin 喂 JSON,事件名与宿主载荷字段
  按调研文档如实映射,未覆盖能力不臆造。
- **D5 版本**:内核加法 + 两套 kit → 收口 bump **0.6.0**(三处同步 + marketplace
  description 更新),tag `v0.6.0`。

## 4. 非目标

各家宿主 GUI/IDE 变体的适配、opencode 子代理事件(待宿主能力)、远程源扩展、
事件 host 维度的渲染过滤 UI(仅在跑行展示,过滤 UI 待真实多宿主使用反馈)。

## 5. 验收

- 内核:`--host` 旗标解析(旗标位置容忍)、subagentstart → dispatched、
  host 落盘与省略兼容、agent 表 host 透传面板;既有 195 断言不破;
- kit 资产:三套 hooks/插件/安装器 + 共享 AGENTDASH.md 单源;opencode 插件
  过 `node --check`;
- dogfood:同一 scratch 项目用 codex 与 opencode 两套载荷交错喂入 → 同一
  events.jsonl、面板 `[codex]`/`[opencode]` 并现、台账单一;
- 三件套全绿;报告入册;0.6.0 + tag + 资产抽验。
