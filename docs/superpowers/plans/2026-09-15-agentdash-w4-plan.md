# agentdash W4 计划(评估清偿:保守化 + 债收敛 + 分发自动化)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development。批内可并行车道,批间屏障;执行/审查子代理一律 flash 档。
> **Spec 基线:** 2026-09-15 W4 规格增量(来源:2026-09-14 深度评估报告;两项裁定项默认不做,授权后插队)。
> **开工前:** dogfood 台账换 W4 候选车道(承 8942f42 惯例);基线 177 测试三件套全绿。

## 批一(行为修正——保守化与详情面板)

### T1 (W4-001):gate 折叠保守化(exit 不可知 → failed)
**Files:** `src/hook.rs`(`exit_code` 语义值"未知"与 0 分离;`on_stop` 折叠三分支:0→passed / 非零或 130→failed / 未知→failed+`(exit unknown)` 尾注)、`tests/hook.rs`(正/负断言:response 缺失、非对象、无退出码字段且 is_error 缺省各一例 → failed;显式 0 → passed;interrupted → 130 → failed;旧单对象暂存兼容不破)、`README.md` 数据契约段 + `kits/claude-code/README.md` 事件映射表(各补一句"exit 不可知记 failed")。
**验收:** 行为变更先红后绿;**既有 passed-by-default 断言逐一列出并说明改判理由**(spec D1);三件套净;单 commit。

### T2 (W4-002):详情面板事件尾 + 过滤一致性裁定落册
**Files:** `src/events.rs`(重放保留 agent+gate 紧凑行,到达序最近 10 条,tool 不入)、`src/model.rs`(`Dashboard::event_tail` 投影)、`src/tui.rs`(`detail_lines` 渲染真实 tail,退役"事件层 W2-008 接入"占位行;`render_detail` 恒查全量模型的裁定写入注释)、`tests/events.rs`(tail 截旧/序/过滤三类断言)+`tests/merge.rs`(投影)+`tests/tui.rs`(占位行消失负断言 + 真实 tail 上板)。
**验收:** 占位行消失(负断言钉死);tail ≤10、只含 agent/gate、按到达序;events 缺失时详情"事件 -"空态;聚焦任务被过滤折叠隐藏时详情仍可开(裁定行为有断言);三件套净。

## 批二(债清偿——纯重构,批间屏障后开工)

### T3 (W4-003):渲染截断收敛
**Files:** `src/render/panel.rs`(删私有 `truncate_width`/`elide`,改用 `render::mod` 公共版;同步清 mod.rs:158 处"批三双轨收敛"待办注释)。
**验收:** 零行为变更,render 断言逐字节黄金对照;177 测试不破;三件套净;单 commit 独立可回滚。

### T4 (W4-004):子进程执行器合一
**Files:** `src/sources/`(新增 `run_capture(dir, program, args, timeout)` 公共实现;`git.rs::run_git`/`remote.rs::run_gh` 改薄封装,`GIT_TIMEOUT`/`GH_TIMEOUT` 参数化)。
**验收:** 零行为变更(git_source/remote 既有断言含超时注入路径全数不破);三件套净;单 commit。

## 批三(风险面加固,与批四可并行)

### T5 (W4-005):手搓解析器 property 测试(proptest dev-dep)
**Files:** `Cargo.toml`(dev-dependencies 增 `proptest`,发布二进制零增依赖)、`tests/`(新增 `tests/proptest_parsers.rs` 或并入既有目标):`utc_timestamp ∘ rfc3339_to_secs` 往返、`civil_from_days`/`days_from_civil` 互逆(Windows 分支 cfg 引入)、`parse_fix_round` 残余性质、`match_word_seq` 样例矩阵 + 随机空白注入不变式。
**验收:** 每性质 ≥256 案例;失败 shrinking 案例钉为点断言回归;三件套净(含 property)。

## 批四(分发链路,与批三可并行)

### T6 (W4-006):Release workflow(tag 触发,三平台资产)
**Files:** `.github/workflows/release.yml`(新:`v*` tag 触发;matrix linux-x64 / macOS 双架构 / Windows;`--release` 构建 + 资产 `<name>-<tag>-<triple>.<ext>` 上传 GitHub Release)、`bin/README.md`(退役"release 资产后续手动挂")、`kits/claude-code/skills/agentdash-setup/SKILL.md`(获取方式与 triple 对齐——现文档写 `windows-gnu`,workflow 若出 `msvc` 以实跑通过者为准回改文档,spec D5)。
**验收:** 测试 tag 实跑(或 dispatch 干跑)+ 下载件 `--version` 自检过;文档三处(bin README / setup SKILL / workflow)资产口径一致;主分支 CI 不受影响。

## 收口

### T7 (W4-007):版本 + 收尾
0.3.0 三处版本号(Cargo.toml / kits plugin.json / marketplace.json);README 路线段补 W4 一句;dogfood 台账更新 W4 真值(note 措辞修正样例:速度行"仅会话期点亮,新 clone 不出属预期降级");帧 spot-check(详情卡真实事件尾可见);三件套;全分支终审;推送(先核远端);tag `v0.3.0`;Release 资产由 T6 workflow 首跑产出并抽验自检。
