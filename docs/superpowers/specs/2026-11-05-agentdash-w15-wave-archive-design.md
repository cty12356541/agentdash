# agentdash W15 波次归档（Wave Archiving）设计规格书（契约 v1.2 演进）

- **日期**：2026-11-05
- **状态**：已定稿并进入 W15 实现（`0.13.0`）
- **目标**：在不引入任何新运行时外部依赖（仅 `crossterm`、`ratatui`、`serde`、`serde_json`）的前提下，解决 `agentdash` 长期存在的「单波次原地覆写丢历史、5MB 轮转丢门禁物证、业务仓 `.gitignore` 一刀切忽略 `.agentdash/`」三大痛点，补齐原生波次封存（`agentdash archive`）、历史波次透视（`--wave <id>`）、多根工作区精准归属（`hook.rs`）与七宿主 `.gitignore` 白名单追踪体系。

---

## 1. 方案权衡（Approaches）

| 维度 | 方案 A：极简同构目录（Angle 1） | 方案 B：强完整性索引与双代合并（Angle 2） | 方案 C：全量预渲染与生态直通（Angle 3） | **最终采纳（Judge Synthesis）** |
| :--- | :--- | :--- | :--- | :--- |
| **存储形态** | `.agentdash/waves/<id>/{ledger.json, events.jsonl}`，无索引文件 | `.agentdash/archives/<id>/` + `meta.json`（FNV-1a64）+ `index.json` | `.agentdash/waves/<id>/` + `index.json` + 预渲染 `digest.txt` / `stats.txt` / `graph.svg` / `closeout.md` | **`.agentdash/waves/<id>/` 同构目录 + `index.json` 自愈索引 + `meta.json`/`digest.txt`/`stats.txt`/`graph.svg`（剔除耦合内网的 `closeout.md`）** |
| **事件流处置** | 默认清空活跃 `events.jsonl`，`--keep-events` 保留 | 默认保留活跃 `events.jsonl`（水线切片），`--prune-events` 按需裁剪 | 默认裁剪活跃流，超 256KB 折叠 `tool` 行 | **默认非破坏性保留（`--keep-events` 缺省），凭 `index.json` 水线防重切；显式 `--prune-events` 裁剪已归档窗口** |
| **Git 追踪与隐私边界** | 仅母仓追踪 `waves/*/ledger.json`，7 套 kit 保持忽略 `.agentdash/` | 忽略 `events.jsonl`，追踪 `ledger.json` / `meta.json` / `digest.txt` / `stats.txt` | 7 套 kit 统一迁移为 `/.agentdash/*` + `!` 白名单，全量追踪 `waves/` | **7 套 kit 升级为 `/.agentdash/*` 白名单，但显式忽略 `/.agentdash/waves/*/events.jsonl`（防本机绝对路径泄露与大文件膨胀），并在 `model.rs`/`stats.rs` 支持从 `meta.json` 无损回填** |
| **历史波次读取** | `render` / `oneline` / `stats` 支持 `--wave <id>` | `archive show <id>` 独立子命令 | 直接传路径 `.agentdash/waves/<id>` 或 glob `.agentdash/waves/*` | **同时支持 `--wave <id>` 旗标与直接传 `.agentdash/waves/<id>` 路径（含 glob 多波次对比）** |

---

## 2. 目录布局与数据契约

```text
<repo>/.agentdash/
├── .lock                           # 跨进程文件锁（hook / writeback / archive 共享）
├── config.json                     # 自定义验证门配置（Git 追踪）
├── ledger.json                     # 当前活跃波次任务台账（Git 追踪）
├── events.jsonl                    # 当前活跃波次事件流 <= 5MB（Git 忽略）
├── events.jsonl.1                  # 5MB 单代轮转备份（Git 忽略）
├── pending_gate.json               # 在途验证门暂存槽位（Git 忽略）
└── waves/                          # 历史波次归档根目录（Git 白名单放行）
    ├── index.json                  # 波次归档总览索引（$schema: agentdash.waves.index.v1，Git 追踪，可自愈）
    └── <wave>/                     # 经校验的安全波次目录（如 W14）
        ├── ledger.json             # 归档时刻冻结的任务台账（$schema: agentdash.tasklog.v1，Git 追踪）
        ├── meta.json               # 结构化统计、门禁快照与 FNV-1a64 校验清单（$schema: agentdash.wave.v1，Git 追踪）
        ├── digest.txt              # 纯文本离场摘要快照（render::render_digest，Git 追踪）
        ├── stats.txt               # 纯文本宿主/门禁/周转统计表（stats::render，Git 追踪）
        ├── graph.svg               # 矢量任务 DAG 拓扑图（render::graph::render_graph_svg，Git 追踪）
        └── events.jsonl            # 切属于本波次的去重有序事件流（Git 忽略，防本机私有路径泄露）
```

### 2.1 波次标识安全校验（`validate_wave_id`）
- 去除首尾 ASCII 空白后，字节长度须为 `1..=64`；
- 首字符须为 ASCII 字母或数字 `[A-Za-z0-9]`（禁止前导 `.`、`-`、`_`）；
- 其余字符仅限 `[A-Za-z0-9._-]`，末字符不得为 `.`（防 Windows 静默剥离尾点），且禁止包含子串 `".."`；
- 禁止 Windows 保留设备名（取首个 `.` 之前的前缀做大小写不敏感匹配）：`CON`、`PRN`、`AUX`、`NUL`、`COM1`..`COM9`、`LPT1`..`LPT9`。

### 2.2 `meta.json`（`$schema: "agentdash.wave.v1"`）与新克隆无事件回填
- 当仓库从远端新克隆时，`waves/<wave>/events.jsonl` 因处于 `.gitignore` 而不在磁盘上。
- `model::merge_wave_opts` 与 `stats::run_dir` 在检测到 `wave_dir/events.jsonl` 缺失但 `wave_dir/meta.json` 存在时，自动从 `meta.json` 回填：
  - `gates`（门名、终态 `passed`/`failed`/`unknown`/`running`、`detail`）；
  - `last_gate_passed`（保持 `done_at <= last_gate_passed` 的物证有效，不误标 `done ?`）；
  - `event_span_secs`（支撑速度线计算）；
  - `host_events` 与 `gate_folds`（支撑 `agentdash stats --wave <id>` 完整输出三张统计表）。

---

## 3. CLI 接口与退出码规范

### 3.1 新增子命令 `agentdash archive`
```text
agentdash archive [--wave <id>] [--strict] [--force] [--prune-events | --keep-events] [--no-infer] [PATH]
agentdash archive --list [-l | list] [--json] [PATH]
```
- **封存模式（默认）**：
  - 波次 ID 优先级：`--wave <id>` > `ledger.json` 顶层 `"wave"` 字段；缺失时退 `1`。
  - 默认防覆写（目标波次已存在时退 `1`，传 `--force` 时经三步回滚安全目录交换覆盖并无损合并历史事件）。
  - `--strict`：在 `.agentdash/.lock` 内执行预检，若台账为空/损坏/有警告、存在非 `done` 任务、存在 `exit == null` 未决门禁、存在失败/unknown 门禁、存在未闭合在跑子代理、存在事件警告或任何任务触发 `unattested_done`（`done ?`），则拒绝归档并退 `1`。
  - `--keep-events`（默认） / `--prune-events`：默认不截断活跃 `events.jsonl`；传 `--prune-events` 时在归档原子落盘后仅保留 `ts > window_end` 的后续事件并清理 `events.jsonl.1`。
- **列表模式（`--list` / `-l` / `list`）**：
  - 扫描并自愈 `.agentdash/waves/index.json`，按自然波次序（`W1 < W2 < W9 < W10 < W14`）输出纯文本对齐表（或 `--json` 输出 JSON），恒退 `0`。

### 3.2 既有只读命令扩展 `--wave <id>` 与直接路径透传
- `agentdash render panel|graph|digest [--wave <id>] ...`
- `agentdash oneline [--wave <id>] [PATH]`
- `agentdash stats [--host <name>] [--wave <id>] [PATH]`
- 同时支持直接传入波次目录：`agentdash render panel .agentdash/waves/W14` 或 `agentdash render panel .agentdash/waves/*`。

---

## 4. 双代事件合并与三层时间窗切片算法

在 `.agentdash/.lock`（5 秒严判锁 `with_archive_lock`）保护下：
1. **冲刷本回合已决暂存槽**：若 `pending_gate.json` 中存在带明确整数 `exit` 的槽位，先折叠进 `events.jsonl`。
2. **双代拼接与接缝去重**：按序读取 `events.jsonl.1` + `events.jsonl`，消除相邻重复行与 `.1 -> 当前` 64 行接缝处的重复行，按 `(effective_ts, arrival_seq)` 稳定排序（同秒事件严格保持物理到达序）。
3. **三层下界 `window_start_secs`**：
   - Tier 1（台账含有效 `started_at` 且无早于 `min(started_at)` 的 `done_at`）：取 `min(started_at) - 60s`（若有前序归档波次水线 `wm`，则钳制在 `wm + 1` 与 `min(started_at)` 之间）；
   - Tier 2（无 `started_at` 的 v1.0 台账，但存在前序归档波次水线 `wm`）：取 `min(wm + 1, earliest_task_ts)`；
   - Tier 3（首波且无 `started_at`）：取 `None`（从事件流起点全量纳入）。
4. **上界 `window_end_secs`**：取 `max(seal_now_secs, max_done_at, max_started_at, merged_ts_max)`，绝不在 `max(done_at)` 处截断，确保收口阶段先盖 `done_at` 后跑通过的验证门事件 100% 入档。
5. **跨边界未闭合子代理守卫**：在 `window_start_secs` 之前已 `dispatched` 但尚未 `completed` 的子代理事件自动前插保留，防止窗口内 `completed` 沦为孤儿警告。

---

## 5. 多根工作区路由修复与 `.gitignore` 白名单

1. **`src/hook.rs` 多根工作区目标路径前缀路由**：
   - 在 Antigravity 等无顶层 `cwd`、仅传 `workspacePaths` 的宿主下，优先从工具载荷提取目标路径提示（`Cwd`/`DirectoryPath`/`TargetFile`/`Path`/`SearchPath`/`file_path` 或命令中的工作区绝对路径），按组件边界最长前缀匹配 `workspacePaths`；`Stop` 事件在无 `cwd` 时遍历 `workspacePaths` 折叠所有含在途槽位的根目录。
2. **七宿主 `.gitignore` 白名单迁移**：
   - 统一使用 `/.agentdash/*` + `!/.agentdash/ledger.json` + `!/.agentdash/config.json` + `!/.agentdash/waves/` + `/.agentdash/waves/.tmp-*` + `/.agentdash/waves/.old-*` + `/.agentdash/waves/*/events.jsonl`，支持旧版单行 `.agentdash/` 原地无损升级，并在 `scripts/verify-kits.sh` 中通过 `git check-ignore` 实测校验。
