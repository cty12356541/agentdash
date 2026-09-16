# agentdash W10 规格增量——0.8 产品化(多仓聚合 + 离场摘要 + 自定义 gate)

- 日期:2026-09-16
- 状态:范围经维护者指令推进台账(2026-09-16);03 升格系 2026-09-16 用户决策(52fceb2 入账)
- 基线:v1 设计 + W9 产品评估(docs/superpowers/reports/2026-09-16-w9-product-review.md,定位升级为「跨宿主 agent 工作契约及其仪表盘」)
- 性质:渲染层扩展 + hook 契约开放;核心契约(ledger/events schema)零破坏

## 1. 范围(0.8 主打三项;候选 04/05 延后 W11)

1. **多仓聚合**:`render panel` 接受多位置参数 + 自研 glob 展开(`~/projects/*/`),
   多仓时渲染**精要视图**(每仓:页眉统计/在跑 agent/失败门/blocked 任务/⚠ 警告),
   单仓输出**逐字节不变**(黄金钉死)。shell 无关——agentdash 自行展开模式。
2. **离场摘要**:`render digest [--strict] [PATH]`——纯文本无 ANSI("你不在时发生了
   什么":失败门+detail/在跑 agent/blocked 任务/done 任务含 ?物证标记/⚠);
   `--strict` 在存在失败门或 blocked 任务时退 1(cron 夜间监控用;本仓首个
   内容性退出码,显式规格化)。
3. **用户自定义 gate**:`.agentdash/config.json` 声明词表,验证门契约从内置 6 条
   开放。词序列匹配(复用既有 match_word_seq/chain_matches 机制,不引入正则);
   用户声明**优先于**内置(先查用户表);文件损坏/形状错→**整表静默回退内置**
   (降级铁律,ledger 03 note 既定);六宿主自动受益(hook 是同一二进制)。

## 2. 设计裁决

- **D1 glob 自研**:不引依赖。仅路径分量级 `*`(如 `projects/*/`),字面前缀
  read_dir 逐段展开;无 `**`;模式无匹配→该模式产一行 ⚠(非崩溃,恒退 0)。
  oneline/watch 不做多仓(仅 panel)。
- **D2 digest 纯文本 + --strict**:无 ANSI(oneline 先例);单 PATH(多仓 digest
  → 用法错退 2);--strict 语义=failed gates>0 或 blocked>0 → 1。
- **D3 config.json 非 config.toml**:v1 设计散文提及 config.toml,但 crate 依赖
  封闭(无 TOML 解析器)——落 **`.agentdash/config.json`**(serde_json 复用),
  v1 该行由本增量取代。schema:`{"gates":[{"name":"pytest","words":["pytest"]}]}`,
  name 限 `[a-z0-9-_]+` 截 40;words ≤8 词;上限 32 条;任一越界/形状错→整表
  回退。自定义表查先于内置表(first-match-wins 语义下用户优先)。
- **D4 版本节奏不变**:波次收口才 bump **0.8.0**(三处+lock),tag v0.8.0,
  Release 资产(发布动作届时单独请示)。

## 3. 非目标

候选 04(agentstats)/05(契约文档化)延后 W11;watch/oneline 多仓;正则 gate;
TOML;编排/云端/Web UI(评估报告既定克制)。

## 4. 验收

- 单仓 `render panel <path>` 输出与 0.7.0 **逐字节不变**(黄金)
- 多仓精要视图四要素(在跑/失败/blocked/⚠)齐备且缺项零残留;glob 展开含 CJK
  目录名;无匹配模式 ⚠ 行恒退 0
- digest 输出零 ESC;--strict 两态退出码钉死
- 自定义 gate:fixture 回放命中用户词表;损坏 config 静默回退(钉死);用户优先
  于内置;verify-kits.sh 六 kit 巡检照常 exit 0
- dogfood:本仓 config.json 声明活体 gate 并经嵌套会话真跑验证;跃迁附 panel
- 三件套全绿;收口 0.8.0 + tag + Release(请示制)
