# agentdash 一期(W1)实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 交付 spec §9 一期:Rust 内核(契约/事件/git 源/合并/渲染/TUI)+ claude-code-kit + gate 事件 + oneline;验收 = 本机侧栏常驻跑通 Claude Code 场景 + 三平台 CI 绿。

**Architecture:** 单 crate `agentdash`,模块 contract/sources/model/render/tui;集成包独立目录 `kits/claude-code/`。算法移植自 claude-dash(已验语义)。

**Tech Stack:** Rust 2024 / ratatui + crossterm / serde + serde_json / stable ≥1.85。

**Spec:** `docs/superpowers/specs/2026-09-13-agentdash-v1-design.md`(§4 契约、§7 错误规范、§8 测试为验收依据)

## Global Constraints

- clippy `all + pedantic -D warnings` 归零;`cargo fmt --check` 归零
- src 禁 `unwrap()`;`expect()` 仅限"构造上不可达"且同行注释论证
- 子进程(git)一律显式 UTF-8(`String::from_utf8_lossy` 或 `-c core.quotepath=false` 等效手段)
- 提交:`<type>(<scope>): 中文 subject (W1-00N)`,一次任务一原子提交,精确 add
- 降级铁律 [AD-ERR-001]:任何源损坏 → 警告行,不 panic 不白屏
- Claim ≤ Evidence;未运行项显式标注

---

### Task 1 (W1-001):脚手架与 CI

**Files:** `Cargo.toml`、`src/main.rs`、`.github/workflows/ci.yml`、`rust-toolchain.toml`、`clippy.toml`(如需)
**Interfaces(Produces):** crate `agentdash` 可构建;子命令骨架 `agentdash <render|oneline|watch> [path]`(本期先空壳返回 0);CI 三平台 test+clippy+fmt

- [ ] Step 1: `cargo init --name agentdash`;依赖 ratatui/crossterm/serde/serde_json(serde derive)
- [ ] Step 2: clippy 门与 fmt 配置;`src/main.rs` 子命令骨架 + `--help`
- [ ] Step 3: CI workflow(push/PR:ubuntu/windows/macos × `cargo test --workspace` + clippy -D warnings + fmt --check)
- [ ] Step 4: 验证:`cargo test`(空测试过)、clippy/fmt 双 0
- [ ] Step 5: 提交 `chore: 脚手架与三平台 CI (W1-001)`

### Task 2 (W1-002):契约层 ledger.json

**Files:** `src/contract.rs`、`schema/agentdash.tasklog.v1.json`、`tests/contract.rs`
**Interfaces:** `pub fn parse_ledger(text:&str)->Result<Ledger,ContractError>`;`Ledger{wave:Option<String>,title,profile:Option<String>,lanes,tasks:HashMap<String,TaskSpec>,barriers}`;`TaskSpec{label,state:TaskState,note:Option<String>}`;`TaskState{Pending,Active,Review,FixRound,Done,Blocked}`(Review/FixRound 仅 profile 声明时非错误)

- [ ] Step 1: 失败测试四组:合法全量 / 损坏 JSON→`ContractError::Corrupt` / 最小核(空 tasks) / 富态无 profile→降级为 Active+警告
- [ ] Step 2: 实现 serde 模型 + 校验(未知字段忽略;状态机合法迁移集校验留 Note 级,损坏即 Corrupt)
- [ ] Step 3: JSON Schema 文件与实现一致性测试(用合法样例交叉)
- [ ] Step 4: 验证 + 提交 `feat(contract): ledger.json 解析校验与 schema (W1-002)`

### Task 3 (W1-003):事件层 events.jsonl

**Files:** `src/events.rs`、`tests/events.rs`
**Interfaces:** `pub fn replay(lines:impl Iterator<Item=String>)->EventModel`;产出:活跃 agent 表(身份+task+首见时刻)、gate 终态(`HashMap<gate名,GateState{Running|Passed{detail}|Failed{detail}}>`)、tool 计数;半行容忍(尾行残缺→丢弃+警告)

- [ ] Step 1: 失败测试:gate running→passed 折叠;agent dispatched→completed 配对;残缺尾行;乱序 ts 容忍
- [ ] Step 2: 实现 + 验证 + 提交 `feat(events): 事件流重放与 gate 折叠 (W1-003)`

### Task 4 (W1-004):git 观察者源

**Files:** `src/sources/git.rs`、`tests/git_source.rs`
**Interfaces:** `pub fn snapshot(repo:&Path)->GitFacts`(分支、head 短 SHA、脏计数、ahead/behind、近 N 提交);无 .git → `GitFacts::absent()`;命令显式 UTF-8

- [ ] Step 1: 失败测试:fixture 仓库(tmpdir `git init`+2 提交)断言字段;非 git 目录→absent
- [ ] Step 2: 实现 + 验证 + 提交 `feat(sources): git 快照观察者 (W1-004)`

### Task 5 (W1-005):模型合并

**Files:** `src/model.rs`、`tests/merge.rs`
**Interfaces:** `pub struct Dashboard{tasks:Vec<TaskView>,milestones,warnings,gates,git,generated_at}`;`pub fn merge(repo:&Path)->Dashboard`(契约>事件>git 可信序;无契约→git 伪任务"最近提交"单链;警告收集)

- [ ] Step 1: 失败测试:三源齐全/仅 git/全无(空态+引导文案)/契约损坏(警告+事件层照常)
- [ ] Step 2: 实现 + 验证 + 提交 `feat(model): 多源合并与降级 (W1-005)`

### Task 6 (W1-006):渲染移植(oneline + panel + graph)

**Files:** `src/render/{oneline,panel,graph}.rs`、`tests/render_*.rs`
**Interfaces:** `render_oneline(&Dashboard)->String`(无 ANSI);`render_panel(&Dashboard,width)->String`;`render_graph(&Dashboard,width)->String`。移植清单:最长路径分层(环保险并入末层)、框化节点 ┬/▼、汇流行 ┴/└┘ 接合、`display_width`(东亚宽=2)、里程碑标题截断、宽度钳 40..120

- [ ] Step 1: 失败测试(移植 claude-dash 断言语义):黄金快照(w25 样例 ledger)、宽字符框缘、跨层边路由、46 列零溢出、oneline 无 ANSI
- [ ] Step 2: 实现(算法从 claude-dash `render_graph.py`/`render_panel.py` 翻译,行为以测试为准)
- [ ] Step 3: 验证 + 提交 `feat(render): panel/graph/oneline 渲染移植 (W1-006)`

### Task 7 (W1-007):TUI watch

**Files:** `src/tui.rs`、`src/main.rs`(接线)
**Interfaces:** `pub fn watch(repo:&Path,interval_secs:u64)->io::Result<()>`;ratatui 差分重绘;键 f/c/⏎/g/q;SGR 鼠标点击→任务命中(几何同 render_graph 布局);分级刷新(模型 5s 档内 git 只在 30s 边界重取);⏎ 一期打印提示(tmux 检测→send-keys,无则复制文本);Ctrl-C/q 干净退出(终端状态还原)

- [ ] Step 1: 可测逻辑先钉:键→动作映射、命中测试(纯函数)、刷新节拍(时钟注入)
- [ ] Step 2: tui 主循环实现;`--once` 冒烟(管道下渲染一帧退出)
- [ ] Step 3: 验证:单测 + 本机手动冒烟(记录输出)+ 提交 `feat(tui): ratatui watch 差分重绘与键鼠 (W1-007)`

### Task 8 (W1-008):claude-code-kit

**Files:** `kits/claude-code/hooks/hooks.json`、`kits/claude-code/hooks/record_event.py`(或小型静态 shim 调二进制)、`kits/claude-code/skills/agentdash/SKILL.md`、`kits/claude-code/install.sh|.ps1`、`kits/claude-code/README.md`、`kits/claude-code/tests/`
**Interfaces:** hook 消费 PostToolUse/Stop/SubagentStop 载荷 → 追加 `.agentdash/events.jsonl`;**gate 提取**:bash 命令匹配(cargo test|clippy|fmt|go test|npm test|gh pr checks)→ `gate` 事件 running(Stop 时折叠 end+exit+摘要行)

- [ ] Step 1: fixture 回放测试:三钩子样例载荷 → 断言 events.jsonl 行(含一条 gate running→passed)
- [ ] Step 2: skill 文档(点播+跃迁附图约定,承 /dash)+ 安装脚本
- [ ] Step 3: 验证 + 提交 `feat(kits): claude-code 集成包与 gate 提取 (W1-008)`

### Task 9 (W1-009):集成验收与发布件

- [ ] Step 1: 端到端:fixture 仓库跑 `agentdash watch --once` / `render panel|graph` / `oneline`,输出与黄金快照比对
- [ ] Step 2: 本机侧栏常驻验证:Windows Terminal 分屏启动器(移植 dash-side 思路)+ 桌面级视觉核查(截图)
- [ ] Step 3: README(定位/安装/三工具路线)+ `cargo dist` 或手工 release 脚本占位
- [ ] Step 4: 三平台 CI 绿(推送临时分支验证)
- [ ] Step 5: 提交 `docs+chore: 一期验收与发布件 (W1-009)`;登记波次台账(本仓 `.agentdash/ledger.json` 首个真实样本,dogfood)

---

## 二期预告(不在本计划)

codex-kit / opencode-kit、远程源(gh/PR/CI 缓存)、观察者兼底完善、分发矩阵铺满
