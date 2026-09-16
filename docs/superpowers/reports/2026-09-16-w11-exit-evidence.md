# W11-002 实证报告——claude 宿主退出码证据(载荷捕获 + 提取补链 + 校准)

- 日期:2026-09-16
- 任务:W11-002(spec `2026-09-16-agentdash-w11-delta.md` §1.2)
- 方法:抛弃式临时仓 + 双钩子 tee 捕获 + 嵌套 headless 会话(claude v2.1.268,win32);
  全程不触碰真实仓/真实钩子配置;捕获后临时目录已清理
- 结论:**绿侧退出码在载荷中确实缺席(校准);红侧退出码在 `error` 串头、此前提取漏掉(已修)**

---

## 1. 捕获的原始载荷(原文引用)

### 1.1 成功命令 → `PostToolUse`(嵌套会话真跑 `cargo bench`,进程真实退出 0)

```json
{"session_id":"a300f077-06a3-4ad0-b5df-1586aa0619a2","cwd":"...\\w11-002-capture",
 "hook_event_name":"PostToolUse","tool_name":"Bash",
 "tool_input":{"command":"cargo bench","timeout":600000,"description":"Run cargo benchmarks"},
 "tool_response":{"stdout":"    Finished `bench` profile [optimized] target(s) in 0.01s\n     Running unittests src\\lib.rs (...)\n\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s","stderr":"","interrupted":false,"isImage":false,"noOutputExpected":false},
 "duration_ms":1485}
```

`tool_response` 全部键:**`interrupted` / `isImage` / `noOutputExpected` / `stderr` / `stdout`**
——**没有 `status`/`exit_code`/`exit`/`is_error`,任何形态下都没有退出码**。
三种命令(成功 bench / 成功 fmt --version / 失败 test)的捕获逐一核对,键集合恒同。

### 1.2 失败命令 → `PostToolUseFailure`(嵌套会话真跑 `cargo test`,进程退出 101)

关键发现:**失败命令不触发 PostToolUse**(双钩子在场,零事件落盘;transcript 证物
`02f0723b-….jsonl` `2026-09-16T06:04:35.763Z`);改触发 `PostToolUseFailure`,载荷
**没有 `tool_response` 字段**,真码在顶层 `error` 串头:

```json
{"hook_event_name":"PostToolUseFailure","tool_name":"Bash",
 "tool_input":{"command":"cargo test --manifest-path Cargo.toml"},
 "error":"Exit code 101\n    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.02s\n...\ntest result: FAILED. 0 passed; 1 failed; ...\nerror: test failed, to rerun pass `--lib`",
 "is_interrupt":false,"duration_ms":1532}
```

同串在 session transcript 侧的形态带 `Error: ` 前缀:`"Error: Exit code 101\n…"`。

## 2. 09-14 `passed exit:0` vs 09-16 `failed(exit unknown)` 之解

**宿主载荷两天同形;变的是提取语义。**

- 09-14 13:13 折叠行(`.agentdash/events.jsonl` 第 1–2 行):
  `{"detail":"rustfmt 1.9.0-stable (8bab26f4f6 2026-07-14)","exit":0,"gate":"cargo-fmt","state":"passed",…}`
  当日二进制的 `exit_code()` 还是乐观默认:**`i64::from(is_error == Some(true))`
  ——无退出码键 → 0**("退出码…不可知 → 0",W4 之前的注释原文)。
  该默认由 **W4-001(commit `5a6886d`)于 2026-09-15 13:37** 改为保守 `None → failed(null)`;
  折叠发生在改动**前一天**,故拿到 `passed exit:0` 的是"不可知被臆写成 0",非真证据。
- 09-14 载荷同形的直接物证:当日 session transcript
  (`projects/C--Users-LENOVO/0968f9d3-….jsonl`,`version: 2.1.268`,
  `2026-09-14T05:13:38.370Z`)的 `toolUseResult`:
  `{"stdout":"…rustfmt 1.9.0-stable (8bab26f4f6 2026-07-14)…","stderr":"","interrupted":false,"isImage":false,"noOutputExpected":false}`
  ——与今日捕获同形,同样无退出码字段。
- 09-16 W10-004 活体折叠(`events.jsonl` 第 456–457 行):
  `{"detail":"test result: ok. 0 passed; … (exit unknown)","exit":null,"gate":"cargo-bench","state":"failed",…}`
  ——保守语义如实标注 unknown;同批 09:39:09 的多门齐折还叠加了 pre-W11-001
  池污染(他会话 Stop 互折),池归属已由 W11-001 修复,本次捕获复验时已排除
  (单会话单池,折叠行为不变)。

## 3. 分支落地

### 3.1 红侧 = 分支 a(载荷有码、提取漏)→ 修 `src/hook.rs`

RED:3 个捕获形状测试先红(`tests/hook.rs`:`failure_event_claude_error_head_carries_real_exit` /
`…error_prefix_variant` / `…zero_or_garbage_falls_to_nonzero_default`)。
GREEN:失败证据链改为——response 显式码 > `error`(或 `error_message`)串头
`Exit code N`/`Error: Exit code N` 解析(仅认非零码,失败在场绝不落 0)> 中断 130 > 缺码 1;
detail 多行 `error` 取末非空行(承 `summary_line` 语义)。新增
`exit_code_of_error_head()`;kit 补注册 `PostToolUseFailure`(claude-code kit
原缺此事件,W9-003 时误认为 claude 无此事件):`hooks/hooks.json`、`install.sh`、
`install.ps1`(顺手补齐该表缺失的 `--host claude` 归属)、`scripts/verify-kits.sh`
(131 断言全过)、清单测试改五事件。

**活体复验**(嵌套 headless,真跑):失败侧终态从 `failed (exit unknown)` 升级为——

```json
{"detail":"error: test failed, to rerun pass `--lib`","exit":101,"gate":"cargo-test","host":"claude","kind":"gate","state":"failed","ts":"2026-09-16T14:19:45+08:00"}
```

### 3.2 绿侧 = 分支 b(码确实缺席)→ README 校准

成功侧退出码在 claude 载荷中**不存在**;凭"PostToolUse 只在成功时触发"推断 0 属
臆造(单宿主单版本经验,且 zcode 同形制失败侧也发 PostToolUse,见 W9-003 测试),
违背 W4-001 D1 铁律。README「claude-code / deepseek 折叠出真实退出码」句校准为:
deepseek(DSH 桥字符串回执尾契约)绿门禁真码;claude-code 绿门禁如实
`failed (exit unknown)`,红门禁经 `PostToolUseFailure` 真码(101 实证)。
若未来要绿侧 passed,需宿主侧改进(如 wrap 命令回显 `$?`)或契约增补,另行立项。

## 4. 验收

- 三件套:**259 测试全绿**(256 + 3 新增)/ `clippy --all-targets` 零警告 / `fmt --check` 干净
- `scripts/verify-kits.sh`:六 kit PASS 131 / FAIL 0
- 活体复验:红侧 exit 101 真证据(上文);绿侧维持诚实 unknown(14:20:25 行)
- 单 commit;临时捕获目录(`%TEMP%\w11-002-capture`)与临时钩子注册已清理
