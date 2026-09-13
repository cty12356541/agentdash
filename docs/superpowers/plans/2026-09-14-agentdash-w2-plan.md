# agentdash W2 计划(交互跃迁 + 信息厚度)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development。三批次推进,批内并行车道、批间屏障;全部 flash 档。

**Spec 基线:** v1 设计 + 2026-09-13 信息需求分析 + 维护者 2026-09-14 指令(全部优化项入册)。
**承接 W1 债:** watch interval 可配 / 单槽位 gate / events 轮转 / render 双轨收敛(批三收尾)。

## 批一(信息接线——数据已存在,只差上板)

### T1 (W2-001):agents + gates 上板
**Files:** `src/model.rs`(Dashboard.agents: Vec<AgentView{who,task,since}>、gates: Vec<GateView{name,state,detail}> 由 EventModel 映射)、`src/render/panel.rs` + `oneline.rs`(agents 区块:who 列表+时长;nag 计数接真值;gates 区块:名称+Running/Passed/Failed+detail 摘要)、`tests/render_panel.rs`/`tests/merge.rs` 增断言。
**验收:** fixture(ledger+含 2 agent+1 gate 的 events)→ panel 显示两 agent 与 gate 状态;oneline `·2ag` 真值。

### T2 (W2-002):fix-round/⚑逐任务/warnings/速度线
**Files:** `src/model.rs`(TaskView.note 解析 fix round N/M → fix_round: Option<(u,u)>;since 逐任务化(契约任务=ledger mtime,事件任务=first_seen);warnings 贯通 contract→Dashboard)、`src/render/panel.rs`(任务行后缀 R1/5、⚑ 停滞标记、⚠ warnings 区块、轨迹速度线 tasks/hour)、`tests/` 增断言。
**验收:** note="fix round 2/5" → 行尾 R2/5;超过阈值任务 ⚑;契约损坏 → ⚠ 行可见;≥2 里程碑时速度行出现。

## 批二(交互跃迁——TUI)

### T3 (W2-003):任务详情面板(A1+A2)
详情态(d/⏎ 或双击节点):右分栏卡片 = label/state/lane/note/fix-round/since/关联 gate/该任务事件 tail(最近 10 条,agent+gate 过滤);Esc 返回。纯函数 detail_lines() 可测。
### T4 (W2-004):多波次滚动 + help(A4+A6)
↑/↓ 在 milestones 间切换 DAG 视图(历史波次重渲染);`?` help 覆盖层(全键位);按键提示行更新。
### T5 (W2-005):过滤与车道折叠(A5)
`/` 进入过滤子串(lane/state/label 匹配);tab 折叠/展开全部完成车道(panel 与 graph 同步)。

## 批三(边界扩展——各自带小设计决策)

### T6 (W2-006):写回操作(A3)
d/b/m 键:done/blocked/note 写回 ledger.json(带确认行 + 文件锁 + 原子替换);spec 修订:投影层唯一的显式写副作用,需用户确认 UX。
### T7 (W2-007):远程源(B4)
gh api PR/checks 缓存 120s(`.agentdash/cache/gh.json`);断网降级 + staleness 标记;panel 增 PR 区块。
### T8 (W2-008):Windows 送对话通道(A7)+ W1 债收尾
⏎:无 tmux 时写 `.agentdash/prompt.txt` + 打印;watch interval 位置参数;pending gate 多槽位;events.jsonl 轮转(上限 5MB 滚动);render_graph_with 双轨收敛删除。

## 本机级安装(推广验证,随批一并行)
release 构建 → `~/.cargo/bin/agentdash.exe`(PATH);用户级 hooks(~/.claude/settings.json 三事件直调);`~/.claude/skills/agentdash/`;fixture 载荷验证落点正确。

### T9 (W2-009):插件市场全生态链路
仓库转公开;.claude-plugin/{marketplace,plugin}.json(插件体=kits/claude-code 布局 + bin/ 预构建 Windows 二进制,README 注明他平台 cargo install);`claude plugin marketplace add cty12356541/agentdash` → `claude plugin install` 到干净环境,真装真用。
### T10 (W2-010):computer-use 双场景视觉验证
场景一 dogfood:agentdash 仓自身侧栏 watch(真数据);场景二 独立项目模拟:tempdir 新仓(git init + 手写 ledger + 模拟 events)跑 watch/graph/panel 截图核验。两场景各出截图与文字报告。
