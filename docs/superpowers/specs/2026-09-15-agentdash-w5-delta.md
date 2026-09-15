# agentdash W5 规格增量——信任锚落地(done_at 物证交叉核对)

- 日期:2026-09-15
- 状态:维护者指令"w5 推进"(范围授权);来源:W4 收口后信任模型讨论 + 复评清单
- 性质:**契约加法扩展**(任务级可选 `done_at`)+ 一处可见新标记(`?`)+ 两项快速清偿

## 1. 范围

1. **done-at 物证交叉核对**(主feature,回答"agent 是否真的按 harness 完成任务"):
   - 契约:`tasks.<id>` 增**可选** `done_at`(RFC 3339 串)——v1 加法,旧台账零改动;
   - 写回:TUI `d` 键标记 done 时**自动盖章** `done_at=now`;从 done 转 blocked 时摘除本戳;
   - 渲染:done 且自带 done_at、事件源在场、且(**无任何通过门**或 **done_at 晚于最近
     通过门**)→ 任务行尾追加 `?`(自报无物证,非指控);事件源不在场(纯契约)→
     不标记(无证可查不作怀疑,承 W3 速度线同款克制);
   - 详情卡:增"物证"字段(✓ 早于最近通过门 / ? 晚于 / - 无时间戳)。
2. **MSRV 文档修正**(复评发现):README 与 setup SKILL 的"≥1.85"实为低报——
   源码使用 let-chains(1.88 稳定),统一改为 ≥1.88。
3. **clock_slice 收敛**(复评发现):tui.rs 与 panel.rs 的私有双份并入 `render::mod`。

## 2. 设计决策

- **D1 信任分层如实表述**:`done_at` 是**自报时间戳**(写回时机器盖章,但写回动作
  本身可由人或 agent 发起);`?` 的语义是"自报时刻晚于机器侧最近通过门(或事件在场
  而从未有通过门)——物证缺失",不是"作弊指控"。人工维护的台账无 done_at → 不标记
  (不能断言 ≠ 可疑)。
- **D2 事件侧对账锚**:`EventModel` 增 `last_gate_passed`(最近 passed gate 的 ts
  原串,每次 passed 覆盖);`Dashboard` 增 `events_present` + `last_gate_passed` 两个
  事实字段。不走事件尾(tail 容量 10 会滚掉旧门,对账锚必须全窗有效)。
- **D3 写回依赖收口**:writeback 自包含设计放宽为"仅依赖 `model::ts_now` 盖章";
  `tests/writeback.rs` 挂载树随之补齐(model 及其依赖),模块注释同步改口。
- **D4 done 转 blocked 摘戳**:离开 done 终态时摘除自家 `done_at`(写回只管理自己
  盖的章,不动人手写的未知字段)。
- **D5 版本**:契约加法 + 可见新标记 → 收口 bump **0.4.0**(三处同步),tag `v0.4.0`。

## 3. 非目标

per-task done-at 的 hook 侧自动来源(宿主无任务边界事件,二期观察者补)、
codex/opencode kit、Node20 actions 升级、proptest 失败持久化配置、
`render graph --format svg`(候选 W6)。

## 4. 验收

- 契约:无 done_at 的旧台账逐字节投影不变(黄金对照);带 done_at 正确穿透;
- 写回:`d` 盖章(原子写内含 done_at)、重复 done 仍报错、done→blocked 摘戳、
  并发写存活不破;
- 渲染:`?` 四态矩阵(有证/无证/无时间戳/无事件窗)正负断言;图例不误伤旧口径;
- 三件套全绿;dogfood:真实事件窗下两种物证状态各一例上板(回填注明);帧证据入册;
- 收口:0.4.0 + tag + Release 资产 workflow 三跑通过 + 推送前核远端。
