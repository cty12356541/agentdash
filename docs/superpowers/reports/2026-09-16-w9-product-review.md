# agentdash W9 产品评估报告(产品定位与功能再评估)

- 日期:2026-09-16
- 范围:W9 全量交付后的定位复盘(六宿主矩阵、证据分级、CLI 双语、巡检工具)
- 结论:**定位升级建议——从「agent 进度仪表盘」到「跨宿主 agent 工作契约
  及其仪表盘」;0.8 主打多仓聚合 + 离场摘要 + 用户自定义 gate**

## 一、W9 实测暴露的产品真相

W9 的全部工作恰是一次产品验证:多宿主把齐(zcode/cursor/deepseek)、证据
分级落地、巡检工具化。三点被真机实测确认的差异化:

1. **证据诚实性是硬通货**。`不虚报通过`、`exit unknown`、done_at 物证
   `?` 交叉核对——别家没有的立场。W9 把它做成了**可分级的硬事实**:
   deepseek 绿/红门禁全真证据(实测 passed exit 0 / failed exit 101),
   zcode/cursor 绿侧如实 unknown。
2. **契约即护城河**。DSH 官方桥直接复用 Claude 形制 hooks.json——业界
   事实上向同一形态收敛,agentdash 是把它收拢为标准的一方。每新增一宿主
   的边际成本已验证到近零(deepseek-kit 的 hook.rs 零改动)。
3. **工程化即信任**。安装器幂等三态、`verify-kits.sh` 六 kit 130 断言、
   降级铁律(hook 永不阻塞宿主)。用户敢装,源于此。

## 二、短板(同等诚实)

1. **单仓视角**:多仓并行跑 agent 无聚合视图——当前架构下最大价值空洞。
2. **异步价值缺位**:面板是"打开终端才有",而多 agent 典型场景是
   "跑一夜、早上看结果"——无通知/摘要/digest。
3. **gate 词表硬编码**:cargo/go/npm/gh 五条写死二进制,用户的 pytest /
   make check / gradle 进不了契约。
4. **台账孤岛**:与 GitHub Issues/Projects 等外部追踪器零互通。
5. **证据毛边残留**:zcode/cursor 绿侧 unknown、复合命令只登记按序首中
   gate(均已如实标注,对用户仍是毛边)。

## 三、竞争坐标

| 坐标 | 玩家 | 相对位置 |
|---|---|---|
| 单宿主会话统计 | DSH session-stats/sqlite、Cursor/Claude 内建 | 深而窄;DSH 自带看板是最大侵蚀风险 |
| 任务看板/编排 | Vibe Kanban、Conductor 类 | 它们管调度,本产品只观察——互补不冲突 |
| 通用任务追踪 | GitHub Projects、Linear | 无互通,平行世界 |
| **跨宿主本地契约** | **基本空位** | **唯一占位者** |

风险:宿主自带看板持续变强。对策即定位本身——宿主看板永远只看自己,
本产品卖跨宿主 + 仓库级事实,且数据格式开放可被任何工具消费。

## 四、定位建议

**从「agent 进度仪表盘」升级为「跨宿主 agent 工作契约及其仪表盘」**,
类 `.editorconfig` 之于编辑器配置:vendor-neutral、repo-local、file-first。
不往编排/看板卷(红海失焦),不做云端账号(破坏 repo-local 卖点)。

## 五、功能取舍(0.8 候选,按性价比排序)

1. **多仓聚合**:`render panel` 支持 PATH glob(`~/projects/*/`),
   一屏看全部仓的在跑/失败/阻塞。纯渲染层,数据已就位。
2. **离场摘要 `render digest`**:"你不在时发生了什么"——折叠失败的门、
   done 的任务、亮 ? 的物证;配合 cron 即夜间监控。
3. **用户自定义 gate**(2026-09-16 用户决策升格 0.8 主打):`.agentdash/config`
   声明命令词表/正则,验证门契约开放出去——契约标准化的最后一块硬骨头。
4. **`agentstats` 观测面**:宿主使用率、gate 通过率、任务周转——
   events.jsonl 里已躺着的数据,低成本观测面。
5. **契约文档化**:schema/ 词表公开版本化,第三方宿主可照写——防御性
   最强的一步。

克制不做:编排调度、云端同步、Web UI(三期再议,digest 顶住异步场景)。

## 六、验证真值(评估基线)

| 项 | 值 |
|---|---|
| 宿主矩阵 | 六行(claude-code/codex/opencode/zcode/cursor/deepseek),deepseek 真机端到端已通 |
| 测试 | 224 通过 / 0 失败(含 cursor 5 + 字符串契约 3 + 语言 4) |
| 巡检 | verify-kits.sh 六 kit 130 断言 exit 0 |
| 证据分级 | deepseek 全真(实测)/ claude 对象字段 / zcode+cursor 绿侧如实 unknown |
| 待验余量 | zcode 失败配对重开复核;cursor 端到端待 `cursor-agent login`;本仓钩子配置含新 Failure 注册,重开生效 |

## 七、下一步

W10 候选车道已开(见台账):0.8 主打 1+2+3(多仓聚合 + digest +
自定义 gate,03 为 2026-09-16 用户决策升格),4-5 视进度进退。版本节奏维持"波次收口才 bump"。
