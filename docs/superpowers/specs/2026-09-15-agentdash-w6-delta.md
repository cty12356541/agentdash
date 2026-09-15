# agentdash W6 规格增量——输出可达性收口

- 日期:2026-09-15
- 状态:维护者指令"未实现的继续实现"(范围 = 历次挂账候选;二期宿主 kit 需调研,列非目标)
- 性质:解析容忍(加法)+ 新输出格式(加法)+ 审查记录 + 测试设施

## 1. 范围

1. **`±HHMM` 基本格式容忍**(W5 收口发现):`rfc3339_to_secs` 增认无冒号偏移
   (`+0800`)——`date +%z` 与部分工具的事实输出;越界字段仍拒解析。
2. **`render graph --format svg`**(W5 收口候选):矢量 DAG 输出,复用
   `layout_layers` 单一几何源;`--format ansi|svg`(缺省 ansi,`--format=X`
   同形);panel 不支持 svg(明确报错);README 命令表同步。
3. **install.ps1 静态审查**(生态 dogfood F1 姊妹项):与 install.sh 语义逐项
   对齐核查(-Depth 10 / 无 BOM / 损坏备份 / 幂等清理 / matcher 一致),结论
   入报告;无缺陷则零改码。
4. **proptest 失败回归落盘**(W4 复评注记):`FileFailurePersistence::Direct`
   指向 crate 根 `proptest-regressions/`,CI 失败案例可回放。

## 2. 设计决策

- **D1 解析只加不改**:既有两形态(`Z` / `±HH:MM`)语义逐字节不变;`±HHMM`
  为第三形态,字段越界(时>23/分>59/长度≠5)拒解析——"解析不了不怀疑"原则
  之上的纯增广。
- **D2 svg 几何同源**:坐标 = 字符几何 ×(列宽 8px,行高 24px)等宽近似;节点框
  /文字/状态色(Visual→hex)与连线肘形路由(父底中 → 汇流行 → 子顶 ▼)均由
  `layout_layers`/`edges_of` 派生,与 ansi 输出同源不同渲染。
- **D3 svg 仅 graph**:panel 是文本版面,svg 化无收益;`render panel --format
  svg` 报错退出 2(与未知 view 同级处理)。
- **D4 版本**:输出格式 + 解析加法 → 收口 bump **0.5.0**(三处同步),tag `v0.5.0`。

## 3. 非目标

codex-kit / opencode-kit(需宿主扩展 API 调研与维护者定向)、远程源扩展、
Node20 actions 升级、per-task done_at 的 hook 自动来源。

## 4. 验收

- 解析:`+0800`/`-0530` 接受;`+080`/`+08000`/越界拒绝;既有黄金断言不变;
- svg:对仓内 dogfood 台账出合法 XML(含全部任务 id 与 `<svg` 根);ansi 缺省
  输出逐字节不变;`--format` 错误路径退出 2;
- proptest:配置生效(回归文件写 crate 根),6 性质照常全绿;
- 三件套全绿;帧证据(svg 渲染样张)入册;收口 0.5.0 + tag + 资产抽验。
