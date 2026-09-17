# 宿主实测指南(六工具手工验证手册)

> 目的:在真实宿主里验证 agentdash 的事件闭环与证据分级。所有命令可直接
> 复制;每节末尾都给出**期望结果**,跑完对照即可。
> 契约细节见 [`docs/contract.md`](contract.md);本手册只讲"怎么测、看什么"。

## 0. 前置(一次性)

```bash
# 二进制在 PATH(hook 直调二进制,这是最常见故障点)
cargo install --path .        # 或从 Releases 下载放入 PATH
agentdash --version           # 自检:应打印 agentdash <版本号>
```

事件落在**项目目录**下 `.agentdash/events.jsonl`,观测命令(`render panel` /
`stats` / `render digest` / `watch`)都在项目根执行即可。

## 1. 先跑一键巡检(不动真实项目)

```bash
bash scripts/verify-kits.sh          # 六 kit 安装体验证,130+ 断言
bash scripts/verify-kits.sh --keep   # --keep 保留临时目录供人工检查
```

期望:末行 `六宿主 kit 安装体验证全部通过`,exit 0。这一步验证的是安装器
与二进制契约;**宿主真跑**(下文)验证的是宿主载荷——两者互补,都要测。

## 2. 各宿主实测

通用套路(每个宿主相同):

```bash
# ① 在 scratch 项目里装对应 kit(不要装进真实项目先)
rm -rf /tmp/e2e-<宿主> && mkdir -p /tmp/e2e-<宿主>
./kits/<宿主>/install.sh /tmp/e2e-<宿主>

# ② 在该宿主里让 agent 跑两条命令(一条成功、一条失败):
#    成功: echo hello-agentdash
#    失败: cargo test        (空目录秒失败;或 false)
# ③ 回到项目根看结果:
cat /tmp/e2e-<宿主>/.agentdash/events.jsonl | tail -5
cd /tmp/e2e-<宿主> && agentdash render panel && agentdash stats
```

### 2.1 Claude Code(`kits/claude-code`)

- 安装:`./kits/claude-code/install.sh /tmp/e2e-claude`(或在项目里 `/plugin`
  装市场版)。
- 在项目目录启动 `claude`,让它执行上面两条命令。
- 期望(events.jsonl):
  - 每条命令 `kind":"tool"`(非 gate)或 `kind":"gate"`(命中词表);
  - `host":"claude"`;失败命令红侧有**真退出码**(`error` 串头 `Exit code N`
    解析)→ 折叠 `failed` 且 `exit=N`;
  - 派子代理时 `kind":"agent"` dispatched/completed。
- 面板:成功门 `✓`(有真码时)或 `?`、失败门 `✗`。

### 2.2 Codex CLI(`kits/codex`)

- 安装:`./kits/codex/install.sh /tmp/e2e-codex`。
- **宿主侧动作**:在 Codex 内 trust 该项目,并经 `/hooks` 一次性审查
  (repo 级钩子的安全边界,宿主强制)。
- 期望:`--host codex` 的 tool/gate 事件;派子代理时 `SubagentStart` →
  面板"在跑"可见。

### 2.3 opencode(`kits/opencode`)

- 安装:`./kits/opencode/install.sh /tmp/e2e-opencode`(Bun 插件,启动
  自动加载)。
- 期望:`--host opencode` 的 tool 事件;**在跑 agent 面板不可见**(宿主
  暂无子代理事件,如实标注)。

### 2.4 ZCode(`kits/zcode`,公司内部)

- 安装:`./kits/zcode/install.sh /tmp/e2e-zcode`。
- **必须重开会话**:工作台钩子于会话启动加载,已运行会话不生效。
- 期望:
  - 每条命令 `--host zcode` 事件;
  - 成功命令的绿门禁 → `? (unknown)`(**已知缺口**:zcode 载荷无退出码
    字段,不虚报;详见 contract §5 证据分级表);
  - **失败命令** → `PostToolUseFailure` 配对(2026-09-16 新注册)→ 折叠
    `✗` 带真码。**这是当前待复核项,测到结果请记录**。
- 注意:失败命令若直接以非零退出,面板可能出现红 ✗ 或 ⚠,属预期。

### 2.5 Cursor(`kits/cursor`)

- 前置:`cursor-agent login`(独立 CLI 认证与 IDE 不通用)。
- 安装:`./kits/cursor/install.sh /tmp/e2e-cursor`。
- 在 Cursor 内**信任该工作区**(项目级 `hooks.json` 的安全边界),Agent
  里跑两条命令;或 CLI:`cd /tmp/e2e-cursor && cursor-agent -p "run: echo ok"`。
- 期望:`afterShellExecution` 载荷(顶层 `command`+`output`,无 tool_name)
  被二进制归一成 bash 视图 → `--host cursor` 的 tool/gate 事件;绿门禁
  `? (unknown)`(载荷无退出码,已知);失败命令若 `postToolUseFailure`
  载荷带 `command` 则归因(官方词表未载明,**待验,测到请记录**)。

### 2.6 DeepSeek Harness(`kits/deepseek`,DSH)

- 安装:`./kits/deepseek/install.sh /tmp/e2e-dsh`(产出
  `.deepseek/agentdash-hooks.json`)。
- **挂载桥**(进程级,一次性):
  ```bash
  DSH_BIN=$(find ~/.npm/_npx -path "*node_modules/.bin/dsh" | head -1)
  "$DSH_BIN" plugin --profile headless add @deepseek-ai/dsh-hooks-claude-code
  "$DSH_BIN" plugin --profile headless add @deepseek-ai/dsh-hook-protocol   # 桥的依赖,实测需显式装
  ```
  再建补丁文件 `/tmp/e2e-dsh/hooks.patch.yml`(configPath 用**绝对路径**):
  ```yaml
  - insert:
      - id: agentdash-hooks
        name: '@deepseek-ai/dsh-hooks-claude-code'
        config:
          configPath: /tmp/e2e-dsh/.deepseek/agentdash-hooks.json
  ```
- 单发真跑:
  ```bash
  cd /tmp/e2e-dsh
  "$DSH_BIN" --profile headless --patch hooks.patch.yml \
    "Use the shell tool to run exactly: echo ok. Then reply: done"
  ```
- 期望(证据最全的宿主):
  - `--host deepseek` 的 tool 事件;gate **绿 `✓ exit=0`、红 `✗ exit=N`**
    (字符串回执尾契约,皆无标记=干净 0);
  - 派子代理时 `subagentStart/Stop`(桥原生),面板可见。

## 3. 观测面速查(任一宿主跑完都用这些看)

```bash
cd <项目>
agentdash render panel        # 健康区:✓ 通过 / ✗ 真失败(有码) / ? 无证据
agentdash render graph        # 任务 DAG
agentdash stats               # 三表:宿主使用率 / gate 通过率(passed/failed/unknown) / 任务周转
agentdash render digest       # 离场摘要(纯文本);--strict 有失败门退 1(cron 夜间监控)
agentdash watch               # 常驻 TUI:滚轮滚动、面板行点击聚焦、g 换视图 / 过滤 ? 帮助 q 退出
```

健康区符号语义:**✓ = 有退出码 0 证据;✗ = 有非零码真失败;? = 宿主没发
退出码,无证据**(不是失败)。逐宿主证据能力见
[`docs/contract.md`](contract.md) §5 证据分级表。

## 4. 不启动宿主的二进制侧测试(模拟载荷)

不经宿主直接喂载荷,验证二进制解析与落盘(与真跑互补:这个测的是
agentdash,真跑测的是宿主发什么):

```bash
T=$(mktemp -d) && cd "$T"
printf '{"tool_name":"bash","tool_input":{"command":"cargo test"},"cwd":"%s"}' "$T" \
  | agentdash hook --host zcode posttooluse
printf '{"hook_event_name":"afterShellExecution","command":"echo hi","output":"hi","cwd":"%s"}' "$T" \
  | agentdash hook --host cursor posttooluse
printf '{"hook_event_name":"PostToolUseFailure","tool_input":{"command":"go test ./..."},"error_message":"boom","cwd":"%s"}' "$T" \
  | agentdash hook --host zcode posttoolusefailure
cat .agentdash/events.jsonl     # 应见 gate running / tool(cursor) / gate running
```

## 5. 排障速查

| 症状 | 原因 | 处置 |
|---|---|---|
| 跑了命令但 events.jsonl 没动静 | `agentdash` 不在 PATH(hook 静默失败) | `cargo install --path .` 后重开宿主会话 |
| 装了 kit 没生效 | 钩子于会话启动加载(zcode/cursor 等) | 重开会话 / 重新信任工作区 |
| 事件落到了别的目录 | hook 按载荷 `cwd`(或缺省进程 cwd)落盘 | 在项目根启动宿主;DSH 桥钩子在会话工作区运行 |
| 绿门禁显示 `?` | 该宿主载荷无退出码字段(zcode/cursor) | 如实标注非故障;换 claude/deepseek 可见 ✓ |
| 损坏配置被改名 | 安装器保守策略:备份 `.bak-agentdash` 不吞文件 | 按指引合并或删除备份重装 |
| 非 Rust/Go/JS 项目的验证门不进面板 | 内置词表外需声明 | `.agentdash/config.json` 自定义词表,见 contract §6 |
