# agentdash 插件安装手册(其他项目接入指南)

> 把 agentdash 接入**任意项目**:每个宿主一段,复制命令即可。安装全部
> 幂等(重复执行只刷新自家注册,不动其他内容);数据落在项目内
> `.agentdash/`,跟仓库走。宿主真跑后的验证方法见
> [`host-testing-guide.md`](host-testing-guide.md),契约细节见
> [`contract.md`](contract.md)。

## 0. 前置:安装二进制(唯一的应用级依赖)

```bash
cargo install --path .        # 本仓库根执行;或从 Releases 下载放入 PATH
agentdash --version           # 自检:应打印版本号
```

hook 是**直调 PATH 上的 `agentdash`**——二进制缺失时钩子静默失败(不阻塞
宿主),所以装 kit 前先确认自检通过。升级 = 重新 `cargo install` + 重跑
对应 kit 的 install.sh(刷新注册)。

## 1. 通用安装流程

```bash
cd <agentdash 仓库根>
./kits/<宿主>/install.sh /path/to/你的项目     # 缺省装进当前目录
```

每个安装器都做三件事(幂等):

1. `AGENTDASH.md` 约定段合入宿主指令文件(`CLAUDE.md`/`AGENTS.md`,带
   `agentdash:begin/end` 标记,重复安装整段刷新);
2. 注册钩子(**只动钩子键**,你自己的其他配置原样;损坏 JSON 自动备份为
   `*.bak-agentdash`);
3. `.gitignore` 追加 `.agentdash/`(已有相关行则跳过)。

安装器要求:目标目录已存在(装进既有项目)、`agentdash` 在 PATH(缺了会
拒绝并打印指引)。

## 2. 逐宿主安装

### 2.1 Claude Code

```bash
./kits/claude-code/install.sh /path/to/项目     # Windows 用 install.ps1
```

产物:`.claude/settings.json`(四钩子,`--host claude`)、
`.claude/skills/agentdash/`、`CLAUDE.md`。
宿主动作:**重开 Claude Code 会话**生效。
市场版(可选,二选一):`claude plugin marketplace add cty12356541/agentdash`
→ `claude plugin install agentdash@agentdash-marketplace`(只装 hooks+skill,
二进制仍需自备)。

### 2.2 Codex CLI

```bash
./kits/codex/install.sh /path/to/项目
```

产物:`.codex/hooks.json`(四钩子,`--host codex`)、`AGENTS.md`。
宿主动作:在 Codex 内 **trust 该项目**,并经 `/hooks` 一次性审查(宿主
安全边界)。已有他人 `hooks.json` 时安装器不吞文件,备份后打印手工合并指引。

### 2.3 opencode

```bash
./kits/opencode/install.sh /path/to/项目
```

产物:`.opencode/plugins/agentdash.js`(启动自动加载)、`AGENTS.md`。
宿主动作:无(重开生效)。

### 2.4 ZCode(公司内部)

```bash
./kits/zcode/install.sh /path/to/项目
```

产物:`.zcode/config.json`(只动 hooks 键,`enabled:true` 置位,四钩子含
`PostToolUseFailure`)、`AGENTS.md`。
宿主动作:**重开 ZCode 会话**(工作台钩子于会话启动加载)。

### 2.5 Cursor

```bash
./kits/cursor/install.sh /path/to/项目
```

产物:`.cursor/hooks.json`(`version:1`,五钩子含 `postToolUseFailure`)、
`AGENTS.md`。
宿主动作:在 Cursor 内**信任该工作区**(项目级钩子的安全边界);CLI 侧
另需 `cursor-agent login`。

### 2.6 DeepSeek Harness(DSH)

```bash
./kits/deepseek/install.sh /path/to/项目
# 产出 .deepseek/agentdash-hooks.json + AGENTS.md + .gitignore
```

挂载桥(应用级 profile,一次性):

```bash
dsh plugin --profile <你的profile> add @deepseek-ai/dsh-hooks-claude-code
dsh plugin --profile <你的profile> add @deepseek-ai/dsh-hook-protocol   # 桥的依赖,需显式装
```

组合里登记(两种方式二选一):

```yaml
# 方式 A:profile 的 cordis.patch.yml 追加(insert 形态)
- insert:
    - id: agentdash-hooks
      name: '@deepseek-ai/dsh-hooks-claude-code'
      config:
        configPath: /绝对路径/项目/.deepseek/agentdash-hooks.json

# 方式 B:单次运行 --patch 叠加(不落盘)
dsh --profile <profile> --patch ./hooks.patch.yml "任务..."
```

注意:`configPath` 是**进程级、启动时一次读取**,相对路径自 DSH 启动
目录解析(建议绝对路径);钩子运行在会话工作区。

## 3. 装完验证

```bash
cd /path/to/项目
agentdash render panel     # 或 watch / stats / render digest
```

然后在宿主里让 agent 跑一条命令(如 `echo hello`),面板/事件即应有
`--host <宿主>` 归属的记录。详细对项(每宿主期望什么、退出码证据分级、
排障表)见 [`host-testing-guide.md`](host-testing-guide.md)。

## 4. 非验证门命令进面板(可选)

内置验证门词表:cargo test/clippy/fmt、go test、npm test、gh pr checks。
其他项目(pytest / make check / gradle …)在 `<项目>/.agentdash/config.json`
声明即可,六宿主共用:

```json
{ "gates": [ { "name": "pytest", "match": ["pytest"] } ] }
```

语义与格式详见 [`contract.md`](contract.md) §6(配置损坏整表静默回退内置)。

## 5. 卸载

| 宿主 | 卸载动作 |
|---|---|
| claude-code | 删 `.claude/settings.json` 中自家四段(或整个 hooks 键,若无人共用)、删 `.claude/skills/agentdash/`、删 `CLAUDE.md` 标记段 |
| codex | 删 `.codex/hooks.json`(装前有他人内容则恢复 `.bak-agentdash`) |
| opencode | 删 `.opencode/plugins/agentdash.js` |
| zcode | 删 `.zcode/config.json`(或仅删 hooks 键;删除后 `enabled` 一并消失) |
| cursor | 删 `.cursor/hooks.json` |
| deepseek | 删 `.deepseek/` + profile 内 `dsh plugin remove` 两包 + 删组合里的 insert 条目 |

通删:各宿主指令文件(`CLAUDE.md`/`AGENTS.md`)中的
`<!-- agentdash:begin … agentdash:end -->` 标记段。历史数据
`<项目>/.agentdash/`(事件/台账)卸载钩子后保留,删不删自定。

## 6. 常见问题

| 现象 | 原因与处置 |
|---|---|
| 跑了命令没事件 | `agentdash` 不在 PATH(hook 静默失败)→ 补装二进制后**重开宿主会话** |
| 装了没生效 | 钩子于会话启动加载(zcode/cursor 等)→ 重开 |
| 升级 agentdash 后行为没变 | 二进制已更新但宿主会话是旧的 → 重开会话;kit 注册无需重装 |
| 多个项目想一起看 | `agentdash render panel ~/projects/a ~/projects/b`(多仓精要视图) |
| 想先验证再接入 | `bash scripts/verify-kits.sh`(临时目录全流程体检,不碰真实项目);逐宿主真跑见 [`host-testing-guide.md`](host-testing-guide.md) |
