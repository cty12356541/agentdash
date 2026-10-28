# agentdash 契约 v1.1(公开版)

- **契约版本:1.1**(2026-09-16 v1.0 定稿;2026-10-28 v1.1 增订)。契约版本与 crate 版本**相互独立**:凡事件词表、
  状态机、折叠/归池/配对语义、证据分级的变更,必须 bump 契约版本并更新本文档;
  纯加法的可选字段不改既有读者语义,亦须在本文档记档。
- 台账自身的版本载体是 `$schema: agentdash.tasklog.v1`(见 §2,v1.1 为向后兼容纯加法,`$schema` 保持不变)。
- **读者**:第三方宿主作者。照本文即可实现接入(发事件 / 写台账 / 自定义验证门),
  无需阅读 agentdash 源码;每条语义均以 `src/contract.rs`、`src/events.rs`、`src/hook.rs`、
  `src/model.rs`、`src/stats.rs`、`src/tui.rs`、`src/render/` 为对账基准。
- 运行时解析是**容错语义**;发布给集成包的**严格校验面**是
  [`schema/agentdash.tasklog.v1.json`](../schema/agentdash.tasklog.v1.json)(JSON Schema 2020-12)。
  两者关系:Schema 管类型/必填/枚举(未知字段两侧皆容忍),内核管降级与警告。
- **v1.1 变更记事(相对 v1.0)**:
  1. **任务开始时刻 `tasks.<id>.started_at`**(§2.2、§2.6):可选 ISO 8601 / RFC 3339 字符串,解析与容错降级规则同 `done_at`;供 `agentdash stats` ③ 任务周转表在 `done_secs >= start_secs` 时计算真实周转时长(中位 / 最大 / 均值),全账无合法计时对时与 v1.0 输出逐字节一致。
  2. **第七宿主 Antigravity(`--host agy`)接入契约**(§1、§4、§5、§7):强制单行 JSON `stdout` 回执(`{"decision":"allow"}` / `{}` / `{"decision":"stop"}`)、`workspacePaths` 优先含 `.agentdash` 根目录回退、`conversationId` → `session_id` 会话归因、`transcript.jsonl` 末尾有界 64KB(`step_index` / `"Command exited with code N."`)退出码提取 + `Stop` 延迟补查、`invoke_subagent` 与子会话 `Stop`(`fullyIdle: true`)完成配对。
  3. **精确口径澄清与观测/渲染同步**(W12-delta A⑥ / W13 / W14):补齐 §2.5 重复任务引用降级警告与首见保留、§3.3 门折叠统计单位(= 每次终态折叠事件)、§4.4 暂存槽空白会话 ID 视同省略(`default` 池);同步记录事件行 `session_id`、`opencode` `metadata` 退出码、推断配对 `completed_at` 与双语汇总警告、TUI 右栏详情门 `unknown` 第三态、声明式 `milestones[].tasks` 波次过滤、`watch --no-infer`、`render digest --strict` 遇 `warnings > 0` 退 `1`,以及宽 DAG 字符画布动态列宽(移除 240 列上限)。

## 0. 总则(降级铁律)

1. **可信序**:契约 > 事件 > git 快照;任何单源缺失/损坏只降级为警告行,不失败、不白屏。
2. **hook 不阻塞**:hook 进程自身任何失败(空/损坏 stdin、非对象载荷、IO 错误)一律
   **静默退出 0**,绝不向宿主报错、绝不阻塞会话;`--host agy` 模式下所有出口另保证输出合法单行 JSON `stdout`(§4)。
3. **未知字段容忍**:ledger 与事件行中的未知字段一律忽略(前向兼容)。
4. **证据诚实**:退出码不可知 → 折叠 `failed` + `(exit unknown)` 尾注,**绝不臆造 0**;
   失败事件在场即非零证据(缺码落 1、中断落 130),**绝不落 0**。
5. **增强自损**:增强结构(`milestones`、富态)损坏只折损自身(警告 + 兜底),不拖垮整账。
6. **有界工作**:自定义 gate 配置超 64KB 视同形状错整表回退,Antigravity `transcript.jsonl` 尾部扫描上限 64KB、子会话完成扫描上限 5MB——单次 hook 恒为有界工作量,绝不读无界文件。

## 1. 目录约定

约定目录 `<repo>/.agentdash/`,落盘根目录解析优先级:
1. 载荷顶层非空 `cwd`;
2. 载荷顶层 `workspacePaths` 字符串数组(Antigravity 多根工作区:优先选取首个已存在 `.agentdash` 子目录的根路径,否则取首个非空路径);
3. 进程当前工作目录。

**宿主载荷务必携带 `cwd` 或 `workspacePaths`**,否则事件可能落入进程启动目录。

| 文件 | 写入方 | 作用 |
|---|---|---|
| `ledger.json` | agent 或人 | 任务台账(§2) |
| `events.jsonl` | `agentdash hook` | 会话事件流,一行一 JSON 追加写(§3、§4) |
| `config.json` | 人或 agent(可选) | 用户自定义验证门词表(§6) |
| `pending_gate.json` | hook | 在途 gate 交接暂存,槽位数组(§4.5);内部文件,格式跨进程兼容 |
| `.lock` | hook / 写回 | 跨进程文件锁 |
| `events.jsonl.1` | hook | 轮转代(§4.6) |

- **锁**:`.lock` 先到先得(`create_new` 原子建),被占自旋(步进 2ms、上限 2s);
  锁文件 mtime 超 10s 判陈锁(持有方崩溃残留)→ 摘除后重抢;最终拿不到 → 退化直接写
  (单行小写入近似原子)。hook 追加与 ledger 写回共用同一把锁,跨进程串行。
- **轮转**:`events.jsonl` 严格超过 5MB 时滚动为 `events.jsonl.1`(单代保留,新轮转覆盖
  旧 `.1`);任何失败静默——轮转缺失只损失历史留存,不阻塞追加。

## 2. `ledger.json`(任务台账)

### 2.1 顶层字段

| 字段 | 类型 | 必填 | 语义 |
|---|---|---|---|
| `$schema` | string | 否 | 恒 `agentdash.tasklog.v1`;不识别的值 → 警告,按 v1 最小核解析(不报错) |
| `wave` | string | 否 | 波次编号(如 `W23`) |
| `title` | string | **是** | 波次标题;缺失 → 台账损坏 |
| `profile` | string | 否 | 语义超集标识(如 `sdd`);声明后 `review`/`fix-round` 富态有效 |
| `note` | string | 否 | 根级波次注记,自由文本。内核按未知字段容忍语义忽略(不解析、写回原样保留),供人读/流程留痕;Schema 未单列(`additionalProperties` 容忍) |
| `lanes` | array | 否 | 车道列表,数组顺序即展示顺序 |
| `tasks` | object | 否 | 任务表:`id → 规格`(§2.2) |
| `barriers` | array | 否 | 屏障列表(§2.4) |
| `milestones` | array | 否 | 声明式里程碑分组(§2.5) |

未知顶层字段一律忽略。

### 2.2 任务规格

| 字段 | 类型 | 必填 | 语义 |
|---|---|---|---|
| `label` | string | **是** | 动词短语标签 |
| `state` | string | **是** | 任务状态(§2.3);未知状态串 → 台账损坏 |
| `note` | string | 否 | 附加说明(如 `fix round 2/5`) |
| `started_at` | string | 否 | 开始自报时刻,RFC 3339 / ISO 8601 串(§2.6,v1.1 新增) |
| `done_at` | string | 否 | 完成自报时刻,RFC 3339 串(§2.6) |

任务规格内未知字段一律忽略。

### 2.3 状态机

- **最小核**:`pending → active → done`;终态 `done`、`blocked`。
- **富态**:`review`(复核中)、`fix-round`(返修轮)——仅当顶层声明 `profile` 后有效;
  未声明 `profile` 时自动降级为 `active`,并逐任务记降级警告(警告按任务 id 排序,保证
  输出确定)。
- 全部合法词:`pending` / `active` / `review` / `fix-round` / `done` / `blocked`。

### 2.4 车道与屏障

- **车道** `lanes[]`:`{"name": 必填, "tasks": [任务id…]}`;`tasks` 缺省空表。任务在
  `lanes[].tasks` 中重复出现时以首次出现为准,未被任何车道引用的任务按 `id` 字典序尾接。
- **屏障** `barriers[]`:`{"id": 必填, "after": [任务id…], "unlocks": [任务id…]}`;
  `after`/`unlocks` 缺省空表。一期只承载语义数据(渲染 DAG 的屏障边),**不做调度校验**。
- **任务 id**:字符串或整数引用,统一按字符串解释;其它类型(布尔/null/嵌套结构)→
  台账损坏。

### 2.5 里程碑(可选,增强字段)

- `milestones[]`:`{"id": 必填, "title": 必填, "tasks": [任务id…]}`。
- 分组语义:被引用的任务按此归组;未被任何里程碑引用的任务进「未分组」尾组;同一任务
  在 `milestones[].tasks`(同组或跨组)被重复引用时**保留首次出现(以首见为准),并产生
  一条降级警告**(`ledger: milestone '<id>' re-references task '<task_id>' (already in '<owner>'); first wins`;
  对比 `§2.4` `lanes[].tasks` 重复引用仅保留首见、不发警告)。未知任务 id 静默跳过。
- 缺省(或 `null`,或结构损坏降级)时由 `wave` + 全任务**合成单里程碑**兜底;结构损坏另
  记警告——增强字段坏了不拖垮整账(总则 5)。

### 2.6 `done_at` 与 `started_at`(自报时刻与周转计时)

- **时间格式与解析容错**:`done_at` 与 `started_at`(v1.1 新增)均为 ISO 8601 / RFC 3339
  字符串;解析统一容忍三形态:`Z`、`±HH:MM`(扩展)、`±HHMM`(基本格式,`date +%z` 等),
  偏移按绝对时刻折算为纪元秒;形态不符、空白、字段越界或年份 0 → 视为不可解析(不报错、
  不损毁台账,字段原串穿透保留)。
- **写回盖章**:TUI `d` 键写回时自动盖 `done_at`(本地时区偏移格式
  `YYYY-MM-DDTHH:MM:SS±HH:MM`);离开 `done` 即摘除 `done_at`(键语义由写回定义;人手写的
  同键同权,一并摘除),既有 `started_at` 与未知字段、未触及任务均原样保留。写回为原子替换
  (先写 `.tmp` 回读校验再 rename);幂等命中(已是目标态/备注原样)显式报错,不静默。
- **物证交叉核对(`done_at`)**:内核不作真伪判定,仅供渲染层物证交叉核对:任务 `done` 且自带
  可解析 `done_at`、事件源在场,且(**窗内无任何通过门** 或 **`done_at` 晚于最近通过门**)→
  任务行打 `?`(自报无物证);无 `done_at`(人工维护,不可断言)、事件源不在场(无证可查不作
  怀疑)或时刻不可解析 → 不标。
- **真实周转统计(`started_at` + `done_at`,W14-07)**:`agentdash stats` ③ 任务周转表对状态为
  `done` 且同时携带可解析 `started_at` 与 `done_at`、满足 `done_secs >= start_secs` 的任务
  折算真实周转秒数(`dur = done_secs - start_secs`),输出中位 / 最大 / 均值(及样本数)与
  `<id> <label> · <dur> (<started_at> → <done_at>)` 明细;同账中缺失、不可解析或
  `done_secs < start_secs`(时钟倒拨)但有 `done_at` 的完成行附注 `(无有效 started_at)` 且不混入
  样本;当全账不存在任何合法 `(started_at, done_at)` 计时对(v1.0 旧台账)时,输出与 v1.0
  逐字节一致(`任务周转: 契约无 started_at,仅列完成时刻(不虚构时长)`);无 `done_at` 的
  `done` 任务不列明细行,以 `N/A(无 done_at): <count>` 收口。

### 2.7 损坏与非致命偏差

| 输入 | 处置 |
|---|---|
| 非法 JSON / 缺 `title` / 类型不符 / 未知状态串 / 任务 id 类型非法 | **台账损坏**:该源降级为警告行,不拖垮整体渲染 |
| 未知 `$schema` | 警告,按 v1 最小核解析 |
| 富态而无 `profile` | 逐任务降级 `active` + 警告 |
| `milestones` 结构损坏 | 警告 + 空表(单里程碑兜底) |
| `milestones[].tasks` 重复引用同一任务 | 警告 + 保留首见(`first wins`) |

## 3. `events.jsonl`(会话事件流)

一行一 JSON,UTF-8 追加写;`ts` 保留原串不做时区运算,乱序容忍 = 按到达序处理;
重放对行内未知字段一律忽略。

### 3.1 事件词表(`kind`)

#### `gate`(验证门)

| 字段 | 必填 | 词表 | 语义 |
|---|---|---|---|
| `gate` | **是** | 门名(§6) | 门标识;缺失 → 残缺行 |
| `state` | **是** | `running` / `passed` / `failed` | 未知或缺省 → 残缺行 |
| `detail` | 否 | 自由文本(≤80 字符) | 摘要;终态行落盘 |
| `exit` | 否 | 数字或 `null` | 退出码证据;`running` 在途行无此字段;`null`/缺失 = exit 不可知 |
| `ts` / `host` / `session_id` | 否 | | 公共字段(§3.2) |

**折叠语义**:同名 `gate` 后到状态覆盖先到(每门名一个终态槽);`running` 表示在途。
**对账锚**:最近一次 `passed` 行的 `ts`(全窗有效,供 §2.6 物证核对);**无 `ts` 的
`passed` 行销毁既有锚**——当前证据说不了谎,也不借旧证。

#### `agent`(子代理)

| 字段 | 必填 | 词表 | 语义 |
|---|---|---|---|
| `event` | **是** | `dispatched` / `completed` | 未知或缺省 → 残缺行 |
| `who` | dispatched 必填 | 代理名 | **主键**;`completed` 可缺省(→ 推断配对,§3.4) |
| `task` | 否 | ≤80 字符 | 可选注记;宿主 `SubagentStop` 载荷天然无 task,只发 `who` 即满足重放契约 |
| `ts` / `host` / `session_id` | 否 | | 公共字段(§3.2) |

**主键语义**:`dispatched` 按 `who` 首见入表(同 `who` 再派只刷新 `task` 注记;
`first_seen`/`host` 保留**首见**不覆盖;再派同时复活被推断完成的行并清空 `completed_at`);
`completed` 按 `who` 移除(与 `task` 注记无关);未在册 `who` 的 `completed`(幽灵)不产生表变更,
该行仍计生效并入事件尾。

#### `tool`(工具回执)

| 字段 | 必填 | 词表 | 语义 |
|---|---|---|---|
| `tool` | **是** | 工具名(小写) | 缺失 → 残缺行 |
| `phase` | **是** | `start` / `end` | `end` 计数,`start` 静默忽略;缺相/未知相 → 残缺行 |
| `exit` | 否 | 数字或 `null` | 退出码证据 |
| `summary` | 否 | ≤80 字符 | 一行摘要 |
| `ts` / `host` / `session_id` | 否 | | 公共字段(§3.2) |

### 3.2 公共字段与残缺行

- `ts`:ISO 8601 本地时刻带时区偏移(如 `2026-09-13T21:00:00+08:00`);原串保留。
- `host`:宿主归属戳;`--host` 显式传入时盖到该进程产出的**每条**事件上,未传则字段
  省略(向后兼容)。统计口径:缺失/空白戳归 `unknown` 桶(诚实标注,不猜归属)。
- `session_id`:会话归属戳(W13 / W14-01);载荷携带非空顶层 `session_id` 或回退
  `conversationId`(Antigravity)时写入每条事件行,缺席/空白则字段省略(与遗留行逐字节同形)。
- **残缺行**(非合法 JSON / 缺关键字段 / 类型不符 / 未知 `kind`)一律丢弃并收集警告,
  **绝不中断重放**;警告格式 `line {n}: <原因>`,行号按物理行从 1 起计;空行静默跳过
  不产生警告。
- **活动窗**:全部已知 `kind` 事件的可解析 `ts` 取极值(残缺行中可解析的 `ts` 亦算);
  合格窗判定 = 极值差存在(≥2 条不同 `ts`)。

### 3.3 生效行口径(消费端统计)

生效 = 重放实际消费的行:`gate` 合法行、`agent` 生效 `dispatched`/`completed` 行
(**含推断配对**)、`tool` `end` 相位行。残缺行与未知 `kind` 不计。门终态折叠另按
`(门名 × 宿主)` 分桶:`passed` / `failed`(带显式数字退出码)/ `unknown`(`exit` 为 `null`
或缺失的失败折叠)各一桶——证据缺失不与真失败混计。**门折叠统计的计数单位是事件流中到达的
每一次终态折叠事件**(`passed` / `failed` / `unknown` 每到达一行各计 1 次,`running` 在途行
不计;不同于面板门视图按门名「后到覆盖先到」只保留最新单槽)。

### 3.4 无 `who` completed 的推断配对(缺省开启)

- 缺 `who` 的 `completed` 行配给**最老在跑**的 `dispatched` 行(表首即派发首见序,
  FIFO;已被推断完成的行跳过,不再吃配对)。
- 配对行显式标注(`▶⇢✓ <who> … (inferred)`),时间戳优先显示该匿名 `completed` 行的完成时刻
  `completed_at`(缺失时回退首次派发时刻 `first_seen`,同 `who` 再派复活时清零):配对只是 FIFO
  启发,**不是实测配对**;计数走完成侧、不占在跑口径。
- 无在跑可配 → 保持逐行丢弃警告;配对成功 → 逐行警告降频,回放收尾出一条汇总警告(随
  `AGENTDASH_LANG` 双语切换:默认中文 `N 个无 who completed 已推断配对`,`en*` 为
  `N anonymous completed events paired by inference`,经面板/摘要渲染前缀 `⚠ `)。
- **严格模式** `--no-infer`(`render panel|graph|digest`、`oneline` 与 `watch` 全观测面均支持):
  关闭启发,无 `who` 的 `completed` 一律严格丢弃 + 逐行警告,行为与启发落地前**逐字节一致**。

## 4. hook 事件入口(生产者契约)

```
# 常规六宿主(不消费 stdout)
agentdash hook [--host <name>] <event> || true

# Antigravity 第七宿主(Windows 经 cmd /c 调起且强制把 stdout 当 JSON 解析,不挂 || true)
agentdash hook --host agy <event>
```

- stdin 读全量 JSON 载荷;非 UTF-8 字节按 replacement 降级;空/损坏 stdin、非对象载荷
  → 静默退 0(总则 2)。
- **Antigravity 强制 JSON `stdout` 回执(`--host agy` / `--host antigravity`,大小写不敏感,W14-01)**:
  宿主把每次 hook 子进程的 `stdout` 作 JSON 解析;无论正常处理还是空/损坏 stdin、非对象载荷、未知事件,
  进程在恒退 `0` 前**必须向 `stdout` 写出单行合法 JSON**:
  - `pretooluse` → `{"decision":"allow"}`
  - `posttooluse`(及其他非拦截/未知事件)→ `{}`
  - `stop` / `subagentstop` → `{"decision":"stop"}`
- 事件名解析:CLI 参数优先(trim + 小写归一),其次载荷 `hook_event_name`,最后防御
  回退(载荷有非空 `tool_name` 即按 `posttooluse`);三者皆无 → 不做任何事。
  旗标与事件名顺序容忍;`--host` 空白值视同未传。**恒退 0**。
- **事件词表**(六者):`posttooluse` / `posttoolusefailure` / `pretooluse` /
  `subagentstart` / `stop` / `subagentstop`;未知事件名静默。
- **载荷词表三家**:
  1. Claude / ZCode / Codex / DeepSeek / opencode 系:`tool_name` + `tool_input` / `tool_response`;
  2. Cursor 系:`afterShellExecution` 顶层 `command` + `output`(hook 归一成 bash 视图走同一条路,退出码证据缺失恒 `None`,不臆造);
  3. Antigravity 系(`--host agy`):camelCase 顶层 `conversationId` / `workspacePaths` / `transcriptPath` / `stepIdx` / `fullyIdle` + `toolCall`(`{"name", "args"}`)。

### 4.1 各事件产出行

| 事件 | 触发 | 产出行 |
|---|---|---|
| `posttooluse` | bash 命中验证门 | `gate` `running` 行 + 暂存槽(§4.5),同临界区写入 |
| `posttooluse` | bash 未命中 | `tool` 行(`phase=end`,`summary`=命令本身) |
| `posttooluse` | 非 bash 工具 | `tool` 行(`summary` 取 `tool_input` 的 `description`/`file_path`/`command`/`pattern` 首个非空) |
| `posttooluse`(cursor) | `afterShellExecution` | 按 bash 视图走上两行(`hook_event_name` 保留宿主原名,据此 + 顶层非空 `command` 判定) |
| `posttooluse`(agy) | `toolCall.name == "run_command"` | 取 `args.CommandLine`/`command`,按顶层 `error` → `tool_response`/`result` → `transcriptPath` 末尾 64KB 提取退出码与摘要(§4.2)走 bash 分流;缺码命中门时暂存槽附 `transcript` + `step_idx` 供 `Stop` 延迟补查 |
| `posttooluse`(agy) | 其余非空 `toolCall.name` | `tool` 行(`summary` 取 `Path`/`TargetFile`/`SearchPath`/`toolSummary`/`Query`/`query`/`Role`/`Url`/`CommandLine` 首个非空;带顶层 `error` 时记 `exit=1`) |
| `posttoolusefailure` | 命令可归因 + 命中验证门 | 顶替同池在途槽为新证据(位置不变,并清掉延迟 `transcript` 键);无在途槽 → 补 `running` 行 + 槽原子对;命令不可得或非门 → **零写入**(宁缺毋造) |
| `pretooluse` | 工具 ∈ `Task`/`Agent`/`invoke_subagent` | `agent` `dispatched`(`Task`/`Agent` 取 `agentType`/`subagent_type`/`name` 与 `description`;`invoke_subagent` 取 `args.Role`/`TypeName` 与 `args.Prompt`,截 80,`who` 缺省 `agent`,`task` 缺省省略);其余工具零写入 |
| `subagentstart` | 原生派发事件(codex/cursor/dsh 等) | `agent` `dispatched`(`who` = `agent_type`/`agent_name`/`subagent_type`/`who` 首个非空,缺省 `agent`;无 `task`,不臆造) |
| `subagentstop` | 子代理结束 | `agent` `completed`(`who` = `agent_name`/`subagent_type`/`who` 透传;载荷无身份则省略 `who`),并折叠该子会话池的在途 gate 槽 |
| `stop` | 会话回合结束 | 折叠**本会话池**在途槽(§4.3、§4.4;含 Antigravity 暂存 `transcript` 延迟补查);`--host agy` 且 `fullyIdle: true` 时另在锁内有界扫描 `events.jsonl`(≤5MB),若存在未闭合派发且本 `conversationId` 非派发方、未发过完成事件,则补记一条匿名 `agent` `completed`(由 §3.4 配对闭合) |

### 4.2 退出码提取(response 与轨迹视图)

- **字符串 response**(DSH 桥渲染契约):尾部标记 `\n[exit code: N]` → `N`;
  `\n[killed by signal: X]` → `130`;**两者皆无 = 干净退出 0**。标记须锚定整串尾,
  正文中段的同形文本不受影响;标记独占全串的退化形(`[exit code: N]` 前无正文)亦认。
- **对象 response**:`interrupted: true` → `130`;顶层 `status`/`exit_code`/`exit` 首个整数;
  嵌套 `metadata` 对象的 `exit`/`exit_code`/`exitCode`/`status` 首个整数(`opencode` 结构化输出,W13);
  `is_error: true` → `1`;全缺 → `null`(落盘为 `null`,不臆造 0)。
- **Antigravity 有界 `transcript.jsonl` 提取**(W14-01):顶层非空 `error` 优先提取非零退出码
  (`Exit code N` / `Command exited with code N` / `exit status N`,无码或 0 落 `1`);否则定位
  `transcriptPath` 文件末尾至多 **64KB** 逆序扫描 JSON 行,匹配 `"type": "GENERIC"`(传入 `stepIdx`
  时要求行内 `step_index == stepIdx`),从 `"content"` 中提取 `"Command exited with code N"`(止于
  `.`、空白或串尾)作为真实整数退出码,并取 `"Command output:"` 之后末条非空行作摘要(非零无输出兜底
  `exit N`)。若 `PostToolUse` 触发时尚未刷盘,暂存槽记录 `transcript` + `step_idx` 于 `Stop` 折叠前
  二次补查;仍不可得则保持 `None`。
- **失败事件证据链**(`posttoolusefailure`):response 显式码 > 顶层 `error`/
  `error_message` 串头 `Exit code N`(容忍 `Error: ` 前缀;**仅认非零码**——失败事件
  在场即非零证据,`Exit code 0`/非数字视同无码)> `is_interrupt: true` → `130` >
  缺码落 `1`。
- **一行摘要**(≤80 字符,按字符截):字符串 response 取去尾标正文末非空行;对象取
  `stdout` 末非空行,空则 `stderr`;失败侧无 response 时取 `error` 串末非空行(Antigravity
  顶层 `error` 取首条非空行)。

### 4.3 折叠语义(Stop → 终态)

对每个在途槽产出一行 `gate` 终态(`ts`/`gate`/`state`/`exit`/`detail`):

| 暂存 `exit` | 折叠终态 |
|---|---|
| `0` | `passed`(detail 原样) |
| 非零(含 130) | `failed`(detail 原样) |
| `null` / 缺失 | `failed` + detail 尾注 `(exit unknown)`(detail 空则单独 `(exit unknown)`)——宁可误报失败,不虚报通过 |

### 4.4 会话池归属(W11-001 / W14-01)

- 载荷会话标识取顶层 `session_id` 优先,缺席回退 `conversationId`(trim 后非空)→ 槽位记录
  `session` 字段;缺席或纯空白 → 不落字段,即 **default 池**(与遗留格式同形,零迁移)。
- **配对谓词与空白槽位规则**:槽位无 `session` 字段,**或槽位上 `session` 为空串/纯空白字符串时,
  一律视同无字段(`default` 池)** → 任意会话的 `Stop` 均可折叠、均可顶替(防手写或脏档残留为永久孤儿槽);
  槽位含非空 `session` → 仅同会话的 `Stop`/`PostToolUseFailure` 可折叠、可顶替。
- `stop` 折叠只动本池:他会话在途槽**原位写回**暂存,由其归属会话的 Stop 折叠;治并发
  会话互相污染退出证据。

### 4.5 `pending_gate.json`(交接暂存)

- 槽位数组,每槽 `{"gate", "exit", "detail"}`(+ 会话自报时的 `"session"`,以及 Antigravity
  `exit=null` 待补查时的 `"transcript"` 与可选 `"step_idx"`);同刻多个在途 gate 各占一槽。
- 旧单对象格式读入兼容(包一层成单槽);其余(损坏/缺失)视为空——折叠不产生幽灵
  事件。
- `stop` 无论解析成败都**消费删除**暂存(写回失败按降级铁律整体消费——折叠仍只做
  一次);暂存失败只损失折叠,不损 `events.jsonl`。
- `running` 行与暂存槽同临界区写入:保证 Stop 折叠读到的槽与 `running` 行配对。

### 4.6 轮转与并发

见 §1(5MB 滚动单代;`.lock` 自旋 + 陈锁自愈 + 超时退化直接写)。

## 5. 证据分级表与观测/渲染语义(七宿主)

`exit` 证据的有无是**宿主载荷能力**,不是 dash 的取舍;折叠一律如实落盘。

| 宿主 | 绿侧(命令成功)证据 | 红侧(命令失败)证据 |
|---|---|---|
| deepseek(DSH 桥) | 字符串回执尾契约(§4.2),双侧**真码**(信号 → 130,无尾标 = 0) | 同左,真码 |
| antigravity(`--host agy`) | `transcriptPath` + `stepIdx` 末尾 64KB 扫描 `"Command exited with code 0"` + `Stop` 延迟补查 → **真码 `0`**(轨迹未刷盘/不可读时如实落 `unknown`) | 顶层 `error` 解析非零码(缺码落 1)或 `transcript.jsonl` 非零码 + `Stop` 延迟补查 → **真码** |
| opencode | `tool_response.metadata`(`exit`/`exit_code`/`exitCode`/`status`,W13)→ **真码** | 同左,真码 |
| claude-code | `tool_response` 对象仅 `stdout`/`stderr`/`interrupted`/`isImage`/`noOutputExpected` 五键、**无退出码字段** → 绿门如实折叠 `failed (exit unknown)` | 命令失败不触发 `PostToolUse`,改发 `PostToolUseFailure`(无 `tool_response`):真码在顶层 `error` 串头 `Exit code N`(仅认非零;101 实证),中断 130,缺码 1 |
| zcode | 对象 `tool_response` 无退出码键 → 绿侧 unknown | `PostToolUseFailure` 已注册,走 §4.2 失败证据链 |
| cursor | `afterShellExecution` 合成视图仅 `stdout` → 绿侧 unknown | `postToolUseFailure` 已注册(顶层 `command` 归因),走 §4.2 失败证据链 |
| codex | 通用提取链(§4.2)按载荷如实落盘,有码则真码、无码则 unknown | 同左 |

- 消费端可凭 `exit` 是否为数字区分「真失败」与「证据缺失」(stats 面 `failed`/`unknown`
  分桶);绿侧 `passed` 的可达性取决于宿主:需真码的宿主可让命令回显 `$?` 或采用
  字符串尾契约。
- **渲染语义(W12-009 / W14-06)**:`state=failed` 且 `exit=null` 的折叠在观测面统一渲染为
  `unknown` 第三态(不占真失败色,有码真失败保持 `✗`):
  - `panel` / `digest`:渲染暗色 `? <gate> · <detail> (unknown)`(剥离原 detail 尾部 `(exit unknown)` 防双注);
  - `watch` TUI 右栏详情卡(W14-06):渲染暗灰 `? <gate> unknown`(带非空 detail 时为 `? <gate> unknown · <detail>`,同样剥除重复 `(exit unknown)` 尾注);
  - `oneline` 无门态面,`stats` 面保持既有 `passed` / `failed` / `unknown` 三列口径,事件流与 `exit_code` 解析零改动。
- **watch TUI 交互、声明式波次过滤与 `--no-infer`(W12-010 / W14-06)**:
  - 鼠标交互:滚轮按列路由滚动面板主区与详情右栏,面板视图左键点击任务行即聚焦(与键盘 `f` 同语义,`render_panel_rows` 行映射与渲染同源零漂移),图视图点击节点聚焦,换视图/关详情滚移归零;交互失败静默降级(§0)。
  - 声明式 `milestones[].tasks` 波次过滤(W14-06):`↑`/`↓` 切换波次时,若当前里程碑声明了非空 `milestones[].tasks`,按任务 `id` 精确匹配过滤(兼容跨车道异构任务编号);仅当 `tasks` 为空时回退 `wave` 前缀段匹配(`W2` 匹配 `W2-003`,不吃 `W25-T1`);无里程碑或全不匹配时回退显示全量任务。
  - `watch --no-infer`(W14-06):常驻与 `--once` 视图支持 `--no-infer`(任意参数位),与 `render`/`oneline` 严格配对口径完全一致。
- **宽 DAG 动态字符画布宽度(W14-02)**:`render graph`(`--format ansi` 及 TUI 图视图)按拓扑布局最右节点动态计算字符画布列宽 `(max_col + 2).max(width).max(4)`,移除早期 240 列硬上限,多车道宽波次右侧节点框与连线永不静默截断。
- **`render digest --strict` 完整性退出码(W14-04 / W12-delta B①)**:`--strict` 在存在任一失败验证门(`state == "failed"`,含 `unknown`)、`blocked` 任务**或任一降级/事件警告(`warnings > 0`,含缺失/损坏 `ledger.json`、损坏 `milestones`、无 `profile` 富态、残缺事件行、推断配对汇总警告)** 时退出 `1`,全净时退出 `0`。
- **七宿主注册面参考**:
  - `claude-code`:五事件(`PostToolUse` / `PostToolUseFailure` / `PreToolUse(Task|Agent)` / `SubagentStop` / `Stop`);
  - `cursor`:五事件(`afterShellExecution` / `postToolUseFailure` / `subagentStart` / `subagentStop` / `stop`);
  - `deepseek`:五事件(DSH 桥 Claude 形制);
  - `codex`:四事件(`PostToolUse` / `SubagentStart` / `SubagentStop` / `Stop`);
  - `zcode`:四事件(`PostToolUse` / `PostToolUseFailure` / `PreToolUse(Task|Agent)` / `Stop`);
  - `opencode`:Bun 插件(`tool.execute.after`);
  - `antigravity`:三事件(`PreToolUse[invoke_subagent]` / `PostToolUse[*]` / 扁平 `Stop`,顶层 `"agentdash"` 命名空间,`--host agy` 单行 JSON `stdout` 回执)。

## 6. `config.json`(用户自定义验证门)

- 路径 `<repo>/.agentdash/config.json`;hook 每次调用**现读**,改完即生效(无缓存、
  无热重载概念)。文件缺失 = 仅内置。
- 格式:

```json
{"gates": [{"name": "pytest", "words": ["pytest"]}, {"name": "make-check", "words": ["make", "check"]}]}
```

- **校验与上限**(先于匹配 enforce,保证有界工作量):
  - `gates` 数组至多 **32** 条;`"gates": []` 是合法空表(等价仅内置)。
  - 每条 `{"name", "words"}`:`name` 限 `[a-z0-9-_]+` 且非空,**截 40 字符**(超长是
    归一化截断,字符集越界才是裁断);`words` 为 **1..=8** 个非空字符串。
  - 文件大小 ≤ **64KB**,超出视同形状错。
  - **任一形状错/越界/文件损坏/超大 → 整表静默回退内置**(零警告、零事件,hook 照常
    退 0;总则 6)。
- **匹配**:用户表**先于**内置表、按声明序 first-match-wins;自定义名与内置同名即覆盖。
  自定义 `words` 走与内置**同一部词序列匹配机**,不引入正则。

### 6.1 内置六门

| 门名 | 词序列 |
|---|---|
| `cargo-test` | `cargo test` |
| `cargo-clippy` | `cargo clippy` |
| `cargo-fmt` | `cargo fmt` |
| `go-test` | `go test` |
| `npm-test` | `npm test` |
| `gh-pr-checks` | `gh pr checks` |

### 6.2 匹配语义(词序列机)

- 词须在命令里**连续**出现且**词边界完整**(`cargo testing` 不匹配 `cargo-test`;
  `仪表盘cargo` 不产生伪边界);词间只容一段空白或前后可围空白的一枚 `&&`/`;`,
  **不跨任何中间 token**(`npm run test` 不匹配 `npm-test`,
  `go build ./... && test` 不匹配 `go-test`)。
- 后段连续即命中:`cargo build && cargo test` 匹配 `cargo-test`;
  `cd x && cargo clippy -- -D warnings` 匹配 `cargo-clippy`。
- 首词多处出现时回溯重试:`cargo; cargo test` 命中后段。
- 词字符 = ASCII 字母数字、`_` 及 ≥0x80 字节(UTF-8 连续字节,CJK 相邻等价
  `\w` 语义)。

## 7. 第三方宿主接入清单

1. 宿主 hook 事件 → `agentdash hook --host <你的宿主名> <event> || true`(命令直调
   二进制,无脚本运行时依赖;恒退 0,不阻塞会话。若宿主如 Antigravity 强制解析 `stdout` JSON,
   使用 `--host agy` 并不挂 `|| true`)。
2. 载荷务必携带 `cwd`(或 `workspacePaths` 落盘目录)与 `session_id`(或 `conversationId` 会话池;
   缺席/空白即 default 池)。
3. 事件映射:`PostToolUse→posttooluse`、失败事件→`posttoolusefailure`、
   子代理派发→`pretooluse`(`Task|Agent|invoke_subagent`)或 `subagentstart`、子代理结束→
   `subagentstop`、回合结束→`stop`;载荷词表对齐 §4(`tool_name` 形制、`afterShellExecution`
   Cursor 形制或 `toolCall` Antigravity 形制)。
4. 退出码证据:能带就带(对象键 `status`/`exit_code`/`exit`、`metadata.exit`、字符串尾契约或
   `transcript.jsonl` 有界回执);带不了就如实缺席——契约保证不虚报,只标 unknown。
5. 验证门词表需要扩展时写 `.agentdash/config.json`(§6),不改二进制。
