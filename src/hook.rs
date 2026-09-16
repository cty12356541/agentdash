//! 集成包钩子入口:`agentdash hook <event>`(spec §6 修订:二进制直调,零 Python 前置)。
//!
//! 读 stdin 全量 JSON 载荷,按事件追加一行(spec §4.2)到 `<cwd>/.agentdash/events.jsonl`:
//! - posttooluse:bash 命中验证门(cargo test/clippy/fmt、go test、npm test、gh pr checks)
//!   → `gate` running 行 + 退出码/一行摘要暂存 `pending_gate.json` **槽位数组**
//!   (同刻多个在途 gate 各占一槽,Stop 折叠须经落盘交接;旧单对象格式读入兼容;
//!   槽位记录归属会话,W11-001);
//!   否则 `tool` phase=end + exit + summary。载荷词表两家:Claude/ZCode 系
//!   `tool_name`+`tool_input`/`tool_response`;Cursor 系 `afterShellExecution`
//!   顶层 `command`+`output`(W9,归一成 bash 视图)
//! - posttoolusefailure(zcode/cursor,W9-003):工具失败事件与在途 gate 配对,
//!   暂存槽顶替为真失败证据(缺码落 1、中断 130);无在途槽补 running+暂存对
//! - stop:把在途 gate 逐槽折叠为各自 passed/failed(exit + detail 摘要行),消费后删除暂存;
//!   退出码不可知(暂存 exit 为 `null`)记 `failed` + detail 尾注 `(exit unknown)`——
//!   不虚报通过(W4-001 D1)。折叠按**会话池**(W11-001):载荷自报 `session_id`
//!   的槽只由同会话的 Stop 折叠,并发会话不再互折退出证据;无 `session_id` 的
//!   槽(default 池,与遗留格式同形)任意 Stop 可折叠,行为与昔日一致
//! - pretooluse:子代理派发工具(`Task`/`Agent`)→ `agent` dispatched(who=
//!   `agentType`/`subagent_type`/`name`,缺省 `agent`;task=`description` 截 80,
//!   缺省省略)
//! - subagentstop:`agent` completed(载荷带 `agent_name`/`who` 则透传)
//!
//! events.jsonl 轮转(W2-008):追加前检查文件大小,超过 5MB 滚动为
//! `events.jsonl.1`(覆盖旧 .1)再新建,防单文件无限增长。
//!
//! 验证门词表(W10-003):内置六门之外,`.agentdash/config.json` 可声明用户
//! 自定义词表(词序列同机,不引入正则);用户表**先于**内置匹配(first-match-
//! wins 下用户优先,同名即覆盖)。任一形状错/越界/文件损坏 → **整表静默回退
//! 内置**(降级铁律),配置按 hook 进程现读、改完即生效。
//!
//! 铁律:自身任何失败(损坏/空 stdin、非对象载荷、IO 错误)一律静默退出 0,
//! 绝不向宿主报错阻塞会话。并发追加经 `.agentdash/.lock` 文件锁自旋
//! (`create_new` 循环,上限重试后按降级铁律退化直接写)保证零丢失;
//! 持有方崩溃残留的超龄陈锁(mtime 超阈值)由后续获取方摘除自愈。

use std::borrow::Cow;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime};

use serde_json::{Map, Value, json};

/// 摘要字段与 detail 的最大字符数(承 Python 参考实现语义)。
const MAX_SUMMARY: usize = 80;
/// 目录约定(spec §4):`<repo>/.agentdash/` 下的三个文件名。
const LOCK_NAME: &str = ".lock";
const PENDING_NAME: &str = "pending_gate.json";
const EVENTS_NAME: &str = "events.jsonl";
/// 轮转落点:`events.jsonl.1`(单代保留,新轮转覆盖旧 .1)。
const EVENTS_ROTATED_NAME: &str = "events.jsonl.1";
/// 轮转阈值(W2-008):events.jsonl 严格超过 5MB 才滚动。
const ROTATE_BYTES: u64 = 5 * 1024 * 1024;
/// 锁自旋参数:上限重试后退化直接写(绝不让宿主 hook 长等待)。
const LOCK_MAX_WAIT: Duration = Duration::from_secs(2);
const LOCK_SLEEP: Duration = Duration::from_millis(2);
/// 陈锁判定阈值:锁文件 mtime 距今超过该值视为持有方崩溃残留(正常持锁临界区
/// 是毫秒级单行写入),可摘除自愈——否则此后每个 hook 都要白等 `LOCK_MAX_WAIT`。
const LOCK_STALE: Duration = Duration::from_secs(10);
/// 用户自定义 gate 配置(W10-003):`.agentdash/config.json`。
const CONFIG_NAME: &str = "config.json";
/// 用户 gate 表上限:条数 32;词数 1..=8;名 `[a-z0-9-_]+` 截 40 字符。
const GATE_MAX_ENTRIES: usize = 32;
const GATE_MAX_WORDS: usize = 8;
const GATE_MAX_NAME_CHARS: usize = 40;
/// 配置文件大小上限(合法 32 条远小于该值;超出视同形状错整表回退——
/// 单 hook 有界工作量,绝不读无界文件)。
const GATE_CONFIG_MAX_BYTES: u64 = 64 * 1024;

/// hook 子命令入口(W7-001):`agentdash hook [--host <name>] <event>`——旗标
/// 与事件名顺序容忍;`host` 显式传入时盖到本进程产出的每条事件上(多宿主
/// 归属,未传则字段省略,向后兼容)。恒退 0。
#[must_use]
pub fn run(rest: &[String]) -> ExitCode {
    let mut host: Option<String> = None;
    let mut event: Option<String> = None;
    let mut iter = rest.iter();
    while let Some(arg) = iter.next() {
        if arg == "--host" {
            host = iter.next().cloned().filter(|h| !h.trim().is_empty());
        } else if let Some(value) = arg.strip_prefix("--host=") {
            host = Some(value.to_owned()).filter(|h| !h.trim().is_empty());
        } else if event.is_none() {
            event = Some(arg.clone());
        }
    }
    let host = host.as_deref();
    // 读失败不退出:按空载荷降级,后面 JSON 解析自然跳过
    let mut bytes = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut bytes);
    // 非 UTF-8 字节按 replacement 降级(承 Python 版 stdio errors=replace),保住可解析部分
    let raw = String::from_utf8_lossy(&bytes);
    let Ok(payload) = serde_json::from_str::<Value>(&raw) else {
        return ExitCode::SUCCESS; // 损坏/空 stdin:静默
    };
    let Some(obj) = payload.as_object() else {
        return ExitCode::SUCCESS; // 非对象载荷:静默
    };
    match resolve_event(event.as_deref(), obj).as_deref() {
        Some("posttooluse") => on_post_tool_use(obj, host),
        Some("posttoolusefailure") => on_post_tool_use_failure(obj, host),
        Some("pretooluse") => on_pre_tool_use(obj, host),
        Some("subagentstart") => on_agent_dispatched(obj, host),
        Some("stop") => on_stop(obj, host),
        Some("subagentstop") => on_subagent_stop(obj, host),
        _ => {} // 未知事件名:静默
    }
    ExitCode::SUCCESS
}

/// 宿主归属戳(W7-001):host 显式传入时写入事件,未传字段省略(向后兼容)。
fn stamp(mut event: Value, host: Option<&str>) -> Value {
    if let Some(h) = host {
        event["host"] = Value::String(h.to_owned());
    }
    event
}

/// 事件名解析:CLI 参数优先(trim + 小写归一),其次载荷 `hook_event_name`,
/// 最后 `tool_name` 防御回退;三者皆无 → 不做任何事。
fn resolve_event(arg: Option<&str>, payload: &Map<String, Value>) -> Option<String> {
    if let Some(name) = arg.map(str::trim).filter(|n| !n.is_empty()) {
        return Some(name.to_ascii_lowercase());
    }
    if let Some(name) = payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        return Some(name.to_ascii_lowercase());
    }
    if payload
        .get("tool_name")
        .and_then(Value::as_str)
        .is_some_and(|t| !t.is_empty())
    {
        return Some("posttooluse".to_owned());
    }
    None
}

/// 事件目录:载荷 `cwd` 优先,退化进程当前目录。
fn events_dir(payload: &Map<String, Value>) -> PathBuf {
    let base = match payload.get("cwd").and_then(Value::as_str) {
        Some(cwd) if !cwd.trim().is_empty() => PathBuf::from(cwd),
        _ => std::env::current_dir().unwrap_or_default(), // 取不到时为相对路径,仍指向进程 cwd
    };
    base.join(".agentdash")
}

/// 会话归属(W11-001):载荷顶层 `session_id`(claude 形制既有字段)非空即取;
/// 缺席/空白 → `None`(default 池)。
fn session_id_of(payload: &Map<String, Value>) -> Option<&str> {
    payload
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// PostToolUse:gate 提取或 tool 行。载荷词表两家:Claude/ZCode 系(`tool_name`
/// + `tool_input`/`tool_response`)与 Cursor 系(`afterShellExecution` 顶层
///   `command`+`output`,W9),后者归一成 bash 视图走同一条路。
fn on_post_tool_use(payload: &Map<String, Value>, host: Option<&str>) {
    let dir = events_dir(payload);
    let session = session_id_of(payload);
    // Cursor 宿主:`afterShellExecution` 无 tool_name,顶层 command 即 shell
    // 命令、output 即合并输出;合成 stdout 视图,退出码证据缺失恒 None(不臆造)。
    if let Some((command, output)) = cursor_shell_payload(payload) {
        let view = json!({ "stdout": output });
        bash_receipt(&dir, &command, Some(&view), host, session);
        return;
    }
    let Some(tool) = payload
        .get("tool_name")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    else {
        return; // 无工具名:跳过
    };
    let tool = tool.to_ascii_lowercase();
    let input = payload.get("tool_input").and_then(Value::as_object);
    let response = payload.get("tool_response");

    if tool == "bash" {
        let command = input
            .and_then(|i| i.get("command"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        bash_receipt(&dir, command, response, host, session);
        return;
    }
    let summary = input
        .and_then(|i| {
            ["description", "file_path", "command", "pattern"]
                .iter()
                .find_map(|key| {
                    i.get(*key)
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                })
        })
        .unwrap_or_default();
    append_tool_event(&dir, &tool, response, summary, host);
}

/// Cursor `afterShellExecution` 载荷识别(W9):CLI 事件名归一为 posttooluse 后,
/// 载荷 `hook_event_name` 保留宿主原名,据此 + 顶层非空 `command` 判定——不靠
/// "顶层恰好有 command"的宽松形状,避免与未来其他宿主的同名词段误撞。
fn cursor_shell_payload(payload: &Map<String, Value>) -> Option<(String, String)> {
    if payload.get("hook_event_name").and_then(Value::as_str) != Some("afterShellExecution") {
        return None;
    }
    let command = payload.get("command").and_then(Value::as_str)?;
    if command.trim().is_empty() {
        return None;
    }
    let output = payload
        .get("output")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Some((command.to_owned(), output.to_owned()))
}

/// bash 工具回执统一入口:命中验证门 → gate running 行 + 暂存槽位(同临界区
/// 配对);否则 tool 行(摘要把命令本身交回)。退出码/摘要证据统一由 `response`
/// 视图提取,Cursor 合成视图缺失退出码即自然落 None。gate 词表用户表先行
/// (W10-003,config.json 静默现读,损坏/越界空表自然回落内置)。
fn bash_receipt(
    dir: &Path,
    command: &str,
    response: Option<&Value>,
    host: Option<&str>,
    session: Option<&str>,
) {
    let custom = load_custom_gates(dir);
    if let Some(gate) = gate_name_with(command, &custom) {
        // running 行与暂存同临界区:保证 Stop 折叠读到的暂存与 running 行配对
        with_lock(dir, || {
            append_line(
                dir,
                &stamp(
                    json!({
                        "ts": ts_now(),
                        "kind": "gate",
                        "gate": gate.as_ref(),
                        "state": "running"
                    }),
                    host,
                ),
            );
            append_pending_slot(
                dir,
                &json!({
                    "gate": gate.as_ref(),
                    "exit": exit_code(response),
                    "detail": summary_line(response),
                }),
                session,
            );
        });
        return;
    }
    append_tool_event(dir, "bash", response, command, host);
}

/// PostToolUseFailure(zcode/cursor 宿主,W9-003):工具失败事件与在途 gate
/// 配对,把失败从 `exit unknown` 升级为真证据。命令可归因出验证门时,顶替
/// 该 gate 的既有暂存槽(位置不变);无在途槽(宿主失败路径未先发
/// posttooluse)则补 running+暂存原子对,保证 Stop 折叠有源。命令不可得
/// (如 cursor error 载荷无 command 字段)或非验证门:零写入——宁缺毋造。
fn on_post_tool_use_failure(payload: &Map<String, Value>, host: Option<&str>) {
    let dir = events_dir(payload);
    let session = session_id_of(payload);
    let custom = load_custom_gates(&dir);
    let Some(command) = failure_command(payload) else {
        return;
    };
    let Some(gate) = gate_name_with(&command, &custom) else {
        return;
    };
    let response = payload.get("tool_response");
    // 失败证据链(W11-002 claude 实测补链):显式 response 码优先;其次 claude
    // 形制顶层 `error` 串头的 `Exit code N`(该宿主失败侧无 tool_response,真码
    // 在 error 串头);中断 130;全缺落 1——失败事件在场即非零证据,绝不落 0
    let error_text = payload
        .get("error")
        .or_else(|| payload.get("error_message"))
        .and_then(Value::as_str);
    let exit = exit_code(response)
        .or_else(|| error_text.and_then(exit_code_of_error_head))
        .unwrap_or_else(|| {
            if payload.get("is_interrupt").and_then(Value::as_bool) == Some(true) {
                130
            } else {
                1
            }
        });
    let mut detail = summary_line(response);
    if detail.is_empty()
        && let Some(msg) = error_text
    {
        // 多行 error(claude 把合并输出整串放这里)取末非空行,承 summary_line 语义
        let last = msg.lines().map(str::trim).rfind(|l| !l.is_empty());
        detail = clip(last.unwrap_or_default());
    }
    with_lock(&dir, || {
        if supersede_pending_slot(&dir, gate.as_ref(), exit, &detail, session) {
            return; // 既有槽已换上新证据,running 行在案
        }
        append_line(
            &dir,
            &stamp(
                json!({
                    "ts": ts_now(),
                    "kind": "gate",
                    "gate": gate.as_ref(),
                    "state": "running"
                }),
                host,
            ),
        );
        append_pending_slot(
            &dir,
            &json!({ "gate": gate.as_ref(), "exit": exit, "detail": detail }),
            session,
        );
    });
}

/// 失败载荷的命令归因(W9-003):claude/zcode 词表走 `tool_input.command`,
/// cursor 词表走顶层 `command`。皆无或空白 → 不可归因。
fn failure_command(payload: &Map<String, Value>) -> Option<String> {
    for key in ["tool_input", "command"] {
        let value = match key {
            "tool_input" => payload
                .get("tool_input")
                .and_then(|i| i.get("command"))
                .and_then(Value::as_str),
            _ => payload.get("command").and_then(Value::as_str),
        };
        if let Some(c) = value.filter(|c| !c.trim().is_empty()) {
            return Some(c.to_owned());
        }
    }
    None
}

/// 顶替在途暂存槽(W9-003):同 gate 且**同会话池**(W11-001,见
/// [`slot_in_session`])的槽换上新证据(位置不变)并写回,返回 true;无可配
/// 槽返回 false(调用方补原子对)。锁内调用。
fn supersede_pending_slot(
    dir: &Path,
    gate: &str,
    exit: i64,
    detail: &str,
    session: Option<&str>,
) -> bool {
    let existing = fs::read_to_string(dir.join(PENDING_NAME))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    let mut slots = pending_slots(existing);
    let mut hit = false;
    for slot in &mut slots {
        if slot.get("gate").and_then(Value::as_str) == Some(gate) && slot_in_session(slot, session)
        {
            slot["exit"] = json!(exit);
            slot["detail"] = json!(clip(detail));
            hit = true;
        }
    }
    if !hit {
        return false;
    }
    // 写不回:按无顶替处理,调用方补原子对(旧槽随读随弃)
    write_pending_slots(dir, &slots)
}

/// PreToolUse:子代理派发工具(`Task`/`Agent` 新旧名)→ `agent` dispatched 行,
/// "在跑 agent"画面闭环(W3-003)。who 取 `tool_input` 的 `agentType`/`subagent_type`/
/// `name`,皆无回退 `agent`;task 取 `description` 截 80,缺省整字段省略。其余工具
/// 零写入静默(matcher 只由宿主兜着,此处防御 matcher 之外直调也不产事件)。
/// 复用既有文件锁/追加/降级铁律,零新依赖。
fn on_pre_tool_use(payload: &Map<String, Value>, host: Option<&str>) {
    let tool = payload
        .get("tool_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_ascii_lowercase);
    if !matches!(tool.as_deref(), Some("task" | "agent")) {
        return; // 非 agent 工具:零写入静默
    }
    let input = payload.get("tool_input").and_then(Value::as_object);
    let who = input
        .and_then(|i| {
            ["agentType", "subagent_type", "name"]
                .iter()
                .find_map(|key| {
                    i.get(*key)
                        .and_then(Value::as_str)
                        .filter(|w| !w.is_empty())
                })
        })
        .unwrap_or("agent");
    let mut event = json!({
        "ts": ts_now(),
        "kind": "agent",
        "event": "dispatched",
        "who": clip(who),
    });
    if let Some(task) = input
        .and_then(|i| i.get("description"))
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    {
        // 注:events.rs 重放以 `who` 为 agent 事件主键,`task` 只是可选注记
        event["task"] = json!(clip(task));
    }
    let dir = events_dir(payload);
    with_lock(&dir, || append_line(&dir, &stamp(event, host)));
}

/// `tool` 事件:phase=end + exit + 一行摘要(截 80)。
fn append_tool_event(
    dir: &Path,
    tool: &str,
    response: Option<&Value>,
    summary: &str,
    host: Option<&str>,
) {
    with_lock(dir, || {
        append_line(
            dir,
            &stamp(
                json!({
                    "ts": ts_now(),
                    "kind": "tool",
                    "tool": tool,
                    "phase": "end",
                    "exit": exit_code(response),
                    "summary": clip(summary),
                }),
                host,
            ),
        );
    });
}

/// Stop:把暂存的**本会话池**在途 gate 逐槽折叠为各自 passed/failed(W11-001:
/// 载荷自报 `session_id` 的槽只由同会话的 Stop 折叠,治并发会话互折退出证据;
/// 无 `session_id` 的槽=default 池,任意 Stop 可折,行为与昔日一致);他会话
/// 槽原位写回暂存,由其归属会话的 Stop 折叠。暂存无论解析成败都消费删除
/// (写回失败按降级铁律整体消费——折叠仍只做一次)。槽缺 `gate` 字段跳过,
/// 不产生幽灵事件。
fn on_stop(payload: &Map<String, Value>, host: Option<&str>) {
    let dir = events_dir(payload);
    let session = session_id_of(payload);
    with_lock(&dir, || {
        let pending = fs::read_to_string(dir.join(PENDING_NAME))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok());
        let _ = fs::remove_file(dir.join(PENDING_NAME));
        let (folding, foreign): (Vec<Value>, Vec<Value>) = pending_slots(pending)
            .into_iter()
            .partition(|slot| slot_in_session(slot, session));
        if !foreign.is_empty() {
            write_pending_slots(&dir, &foreign); // 他会话在途槽原位保留
        }
        if folding.is_empty() {
            return; // 本池无在途 gate / 暂存损坏:不产生幽灵事件
        }
        for slot in folding {
            let gate = slot.get("gate").and_then(Value::as_str).unwrap_or_default();
            if gate.is_empty() {
                continue;
            }
            let exit = slot.get("exit").and_then(Value::as_i64);
            let detail = clip(
                slot.get("detail")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );
            // 折叠语义(W4-001 D1):显式 0 才 passed;非零(含 interrupted 130)
            // failed;不可知(null)同样 failed——宁可误报失败,不虚报通过。
            // 尾注 `(exit unknown)` 只加在不可知路径,区分"真失败"与"证据缺失"。
            let (state, detail) = match exit {
                Some(0) => ("passed", detail),
                Some(_) => ("failed", detail),
                None => (
                    "failed",
                    if detail.is_empty() {
                        String::from("(exit unknown)")
                    } else {
                        format!("{detail} (exit unknown)")
                    },
                ),
            };
            append_line(
                &dir,
                &stamp(
                    json!({
                        "ts": ts_now(),
                        "kind": "gate",
                        "gate": gate,
                        "state": state,
                        "exit": exit,
                        "detail": detail,
                    }),
                    host,
                ),
            );
        }
    });
}

/// `SubagentStop`:`agent` completed 行;载荷带 `agent_name`/`subagent_type`/`who`
/// 则透传(第三键为 Cursor 宿主词表,W9;前键缺席时回退,老宿主不受影响)。
fn on_subagent_stop(payload: &Map<String, Value>, host: Option<&str>) {
    let who = ["agent_name", "subagent_type", "who"]
        .iter()
        .find_map(|key| {
            payload
                .get(*key)
                .and_then(Value::as_str)
                .filter(|w| !w.is_empty())
        });
    let dir = events_dir(payload);
    let mut event = json!({ "ts": ts_now(), "kind": "agent", "event": "completed" });
    if let Some(who) = who {
        event["who"] = json!(clip(who));
    }
    // 注:events.rs 重放以 `who` 为 agent 事件主键,`task` 只是可选注记;
    // 宿主 SubagentStop 载荷天然无 task,本行只发 who 即满足重放契约。
    with_lock(&dir, || append_line(&dir, &stamp(event, host)));
}

/// SubagentStart(codex 等原生事件,W7-001):`agent` dispatched 行。who 取
/// `agent_type`/`agent_name`/`subagent_type`/`who` 首个非空,缺省 `agent`;
/// task 字段不在该载荷契约内,不臆造。
fn on_agent_dispatched(payload: &Map<String, Value>, host: Option<&str>) {
    let who = ["agent_type", "agent_name", "subagent_type", "who"]
        .iter()
        .find_map(|key| {
            payload
                .get(*key)
                .and_then(Value::as_str)
                .filter(|w| !w.is_empty())
        })
        .unwrap_or("agent");
    let event = json!({
        "ts": ts_now(),
        "kind": "agent",
        "event": "dispatched",
        "who": clip(who),
    });
    let dir = events_dir(payload);
    with_lock(&dir, || append_line(&dir, &stamp(event, host)));
}

/// 临界区包装:拿到 `.lock`(`create_new` 自旋,陈锁自愈,超时退化)后执行 `f`,
/// 退出即删锁(仅在自己真持有才删)。持有方崩溃残留的陈锁由获取方摘除自愈;
/// 自愈后仍拿不到时按降级铁律直接写(单行小写入近似原子,常态零丢失由锁保证)。
fn with_lock(dir: &Path, f: impl FnOnce()) {
    if fs::create_dir_all(dir).is_err() {
        return; // 目录建不出来:无写目标,静默
    }
    let lock_path = dir.join(LOCK_NAME);
    let held = acquire_lock(&lock_path);
    f();
    if held {
        let _ = fs::remove_file(&lock_path);
    }
}

/// 锁获取:先到先得(`create_new`);被占则自旋等待。锁文件 mtime 超过
/// `LOCK_STALE` 判为陈锁(持有方已崩溃,不会再有人删它)→ 摘除后重抢:
/// 等待前先查一次(陈锁不白等),自旋超时后再兜底一次(等待期间诞生的锁
/// 也可能已超龄)。摘除与获取同走 `create_new` 原子语义——摘后他人先抢到
/// 则按正常占用继续等/退化,可接受。最终拿不到 → false(退化直接写)。
fn acquire_lock(lock_path: &Path) -> bool {
    if is_stale_lock(lock_path) {
        let _ = fs::remove_file(lock_path);
    }
    if spin_for_lock(lock_path) {
        return true;
    }
    if is_stale_lock(lock_path) {
        let _ = fs::remove_file(lock_path);
        return try_create_lock(lock_path);
    }
    false
}

/// 自旋抢锁:`create_new` 循环,`LOCK_SLEEP` 步进,`LOCK_MAX_WAIT` 上限;
/// 超时/异常(目录消失等)→ false。
fn spin_for_lock(lock_path: &Path) -> bool {
    let deadline = Instant::now() + LOCK_MAX_WAIT;
    loop {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock_path)
        {
            Ok(_) => return true,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                if Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(LOCK_SLEEP);
            }
            Err(_) => return false,
        }
    }
}

/// 单次 `create_new` 抢锁(摘后重抢亦走此原子判定,避免"查后建"竞态)。
fn try_create_lock(lock_path: &Path) -> bool {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)
        .is_ok()
}

/// 陈锁判定:锁文件 mtime 距今超过 `LOCK_STALE`。metadata/mtime 取不到
/// (恰被他人摘除)或 mtime 在未来(时钟偏差)→ 非陈锁,照常等待。
fn is_stale_lock(lock_path: &Path) -> bool {
    fs::metadata(lock_path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|mtime| SystemTime::now().duration_since(mtime).ok())
        .is_some_and(|age| age >= LOCK_STALE)
}

/// 单行追加:轮转检查后,整行(含换行)拼好一次 `write_all`,锁内调用保证
/// 并发零丢失。
fn append_line(dir: &Path, event: &Value) {
    rotate_if_large(dir);
    let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(EVENTS_NAME))
    else {
        return; // 打不开目标文件:静默
    };
    let mut line = event.to_string();
    line.push('\n');
    let _ = file.write_all(line.as_bytes());
    let _ = file.flush();
}

/// 轮转(W2-008):events.jsonl 严格超过 [`ROTATE_BYTES`] 时滚动为
/// `events.jsonl.1`(单代保留:先摘旧 .1 再 rename,Windows 侧 rename 不
/// 覆盖既有目标)。任何失败静默——轮转缺失只损失历史留存,不阻塞本次追加。
/// 锁内调用,无并发轮转竞态。
fn rotate_if_large(dir: &Path) {
    let path = dir.join(EVENTS_NAME);
    let Ok(meta) = fs::metadata(&path) else {
        return; // 尚无文件:无需轮转
    };
    if meta.len() <= ROTATE_BYTES {
        return;
    }
    let rotated = dir.join(EVENTS_ROTATED_NAME);
    let _ = fs::remove_file(&rotated);
    let _ = fs::rename(&path, &rotated);
}

/// 暂存归一为槽位数组(W2-008 多槽位):数组原样;旧单对象格式向后兼容,
/// 包一层成单槽;其余(损坏/缺失)视为空——折叠不产生幽灵事件,写入侧
/// 则从当前槽重建。
fn pending_slots(pending: Option<Value>) -> Vec<Value> {
    match pending {
        Some(Value::Array(items)) => items,
        Some(obj @ Value::Object(_)) => vec![obj],
        _ => Vec::new(),
    }
}

/// 池配对谓词(W11-001):槽位无 `session` 字段(遗留格式与 default 池同形)
/// 归任何会话可配——跨会话 default 互折保持昔日行为;有字段则须同会话,
/// 并发会话互不折叠/互不顶替。空白字段视同无字段(防手写脏档成孤儿槽)。
fn slot_in_session(slot: &Value, session: Option<&str>) -> bool {
    match slot
        .get("session")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => true,
        Some(own) => Some(own) == session,
    }
}

/// 槽位数组整体写回暂存(W11-001:Stop 为他会话保留在途槽;顶替写回同法)。
/// 失败静默——损失的是交接暂存,不损 events.jsonl(降级铁律)。锁内调用。
fn write_pending_slots(dir: &Path, slots: &[Value]) -> bool {
    let Ok(mut file) = File::create(dir.join(PENDING_NAME)) else {
        return false;
    };
    let _ = file.write_all(serde_json::to_string(slots).unwrap_or_default().as_bytes());
    true
}

/// gate 交接暂存(多槽位):读入既有暂存归一成数组后追加一槽,同刻多个
/// 在途 gate 各占一槽,Stop 时逐槽折叠各自终态。载荷自报 `session_id` 时
/// 槽位记录归属(同会话 Stop 才折叠);未自报不落字段(default 池,与遗留
/// 格式同形,零迁移)。读-并-写同在锁内临界区;暂存失败只损失折叠,不损
/// events.jsonl。
fn append_pending_slot(dir: &Path, slot: &Value, session: Option<&str>) {
    let existing = fs::read_to_string(dir.join(PENDING_NAME))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    let mut slots = pending_slots(existing);
    let mut slot = slot.clone();
    if let Some(session) = session {
        slot["session"] = json!(session);
    }
    slots.push(slot);
    write_pending_slots(dir, &slots);
}

/// 用户自定义 gate(W10-003):`.agentdash/config.json` 声明的验证门词表条目。
/// `pub(crate)` 字段供 W10-003 性质测试构造(模块挂载后同 crate 可见)。
pub(crate) struct CustomGate {
    pub(crate) name: String,
    pub(crate) words: Vec<String>,
}

/// 读取用户自定义 gate 表(W10-003):文件缺失/损坏/超限/任一形状错 → 空表
/// (即仅内置),零事件零输出(降级铁律);配置按 hook 进程现读,改完即生效,
/// 无需缓存。表上限 32 条、词 1..=8、名截 40,均先于匹配 enforce(有界工作量)。
fn load_custom_gates(dir: &Path) -> Vec<CustomGate> {
    let path = dir.join(CONFIG_NAME);
    if !fs::metadata(&path).is_ok_and(|meta| meta.len() <= GATE_CONFIG_MAX_BYTES) {
        return Vec::new(); // 缺失(含目录不存在)或超大:视同无表/形状错
    }
    parse_custom_gates(&fs::read_to_string(&path).unwrap_or_default()).unwrap_or_default()
}

/// 解析用户 gate 表文本:`gates` 数组,每条 `{"name","words"}`。name 限
/// `[a-z0-9-_]+` 且截 40 字符(长度是归一化,字符集才是裁断);words 1..=8
/// 个非空词;至多 32 条。任一越界 → [`None`] → 调用方整表回退内置。
/// `pub(crate)` 供 W10-003 性质测试(解析面不 panic + 截断不变式)。
#[must_use]
pub(crate) fn parse_custom_gates(text: &str) -> Option<Vec<CustomGate>> {
    let root = serde_json::from_str::<Value>(text).ok()?;
    let entries = root.get("gates")?.as_array()?;
    if entries.len() > GATE_MAX_ENTRIES {
        return None; // 越界:整表回退
    }
    let mut gates = Vec::with_capacity(entries.len());
    for entry in entries {
        let name = entry.get("name").and_then(Value::as_str)?;
        if name.is_empty()
            || !name
                .chars()
                .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-' | '_'))
        {
            return None;
        }
        let words_json = entry.get("words").and_then(Value::as_array)?;
        if words_json.is_empty() || words_json.len() > GATE_MAX_WORDS {
            return None;
        }
        let mut words = Vec::with_capacity(words_json.len());
        for word in words_json {
            words.push(word.as_str().filter(|w| !w.is_empty())?.to_owned());
        }
        gates.push(CustomGate {
            name: name.chars().take(GATE_MAX_NAME_CHARS).collect(),
            words,
        });
    }
    Some(gates)
}

/// gate 匹配全入口(W10-003):用户表先行(first-match-wins 语义下用户优先,
/// 同名即覆盖内置),未命中回落内置词表 [`gate_name`]。返回名借用自用户表
/// 条目或静态内置名。自定义 words 走同一词序列机([`match_word_seq`]),不引入
/// 第二匹配器。
#[must_use]
pub(crate) fn gate_name_with<'a>(command: &str, custom: &'a [CustomGate]) -> Option<Cow<'a, str>> {
    for gate in custom {
        let words: Vec<&str> = gate.words.iter().map(String::as_str).collect();
        if match_word_seq(command, &words) {
            return Some(Cow::Borrowed(&gate.name));
        }
    }
    gate_name(command).map(Cow::Borrowed)
}

/// 内置验证门词表查询(六门):连续词序列 + 词边界(承 Python 版
/// `\bcargo\s+test\b` 的相邻语义):目标序列的词必须在命令里**连续**出现,
/// 词间只容空白或一枚 `&&`/`;` 分隔符,不得跨任意中间 token——`npm run test`
/// 不匹配 npm-test、`go build ./... && test` 不匹配 go-test,而
/// `cargo build && cargo test`(后段连续)与 `cd x && cargo clippy -- -D warnings`
/// 匹配;按序首中即返。W10-003 起为内置表包装(hook 链路走
/// [`gate_name_with`] 用户优先);`pub(crate)` 供 W4-005/W10-003 性质测试直连。
pub(crate) fn gate_name(command: &str) -> Option<&'static str> {
    const PATTERNS: [&[&str]; 6] = [
        &["cargo", "test"],
        &["cargo", "clippy"],
        &["cargo", "fmt"],
        &["go", "test"],
        &["npm", "test"],
        &["gh", "pr", "checks"],
    ];
    const NAMES: [&str; 6] = [
        "cargo-test",
        "cargo-clippy",
        "cargo-fmt",
        "go-test",
        "npm-test",
        "gh-pr-checks",
    ];
    PATTERNS
        .iter()
        .zip(NAMES)
        .find_map(|(words, name)| match_word_seq(command, words).then_some(name))
}

/// 连续词序列匹配:遍历首词的每个词边界完整出现,自该处确定性延链;任一
/// occurrence 延链成功即真,全部失败即假(首词多 occurrence 时回溯重试,
/// 承正则交替语义,如 `cargo; cargo test` 命中后段)。
fn match_word_seq(haystack: &str, words: &[&str]) -> bool {
    let Some(first) = words.first() else {
        return true; // 仅 words 为空时可达;本模块恒传非空词序列
    };
    let bytes = haystack.as_bytes();
    let mut pos = 0;
    while let Some(at) = find_word(bytes, first, pos) {
        if chain_matches(bytes, words, at) {
            return true;
        }
        pos = at + 1;
    }
    false
}

/// 自 `start` 起按序匹配 `words`(首词起于 `start` 且词边界完整,调用方保证):
/// 后续词必须恰起于前一词之后的分隔符收口处——只越过空白/`&&`/`;`,不跨任何
/// 中间 token;每词词尾词边界完整(`cargo testing` 不匹配 cargo-test)。
fn chain_matches(bytes: &[u8], words: &[&str], start: usize) -> bool {
    let mut cursor = start;
    for (idx, word) in words.iter().enumerate() {
        if idx > 0 && !bytes[cursor..].starts_with(word.as_bytes()) {
            return false; // 分隔符后第一个词字符处不是本词:序列不连续
        }
        cursor += word.len();
        if cursor < bytes.len() && is_word_byte(bytes[cursor]) {
            return false; // 词尾须词边界完整
        }
        if idx + 1 == words.len() {
            return true;
        }
        let sep = separator_len(&bytes[cursor..]);
        if sep == 0 {
            return false; // 词间只容分隔符:粘连或隔其他 token 均不连续
        }
        cursor += sep;
    }
    true
}

/// 词间分隔符长度:一段空白;或其前后可再围空白的一枚 `&&`/`;`。其余任何
/// 字节(`|`、单词、路径…)都不构成分隔。
fn separator_len(bytes: &[u8]) -> usize {
    let lead = bytes.iter().take_while(|b| is_ws_byte(**b)).count();
    let rest = &bytes[lead..];
    let op = if rest.starts_with(b"&&") {
        2
    } else if rest.first() == Some(&b';') {
        1
    } else {
        return lead; // 纯空白(可为 0):后续词须紧跟其收口处
    };
    lead + op + rest[op..].iter().take_while(|b| is_ws_byte(**b)).count()
}

/// 从 `from` 字节起找下一个词边界完整的 `word`。词字符按字节判:ASCII 字母数字、
/// `_`、以及 ≥0x80(UTF-8 连续字节)——与 Python `\w` 的 unicode 语义在 CJK 相邻
/// 场景等价(`仪表盘cargo` 不产生伪边界,不匹配)。
fn find_word(haystack: &[u8], word: &str, from: usize) -> Option<usize> {
    let needle = word.as_bytes();
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    (from..=haystack.len() - needle.len()).find(|start| {
        &haystack[*start..*start + needle.len()] == needle
            && (*start == 0 || !is_word_byte(haystack[*start - 1]))
            && (*start + needle.len() == haystack.len()
                || !is_word_byte(haystack[*start + needle.len()]))
    })
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

fn is_ws_byte(b: u8) -> bool {
    b.is_ascii_whitespace() || b == 0x0B // \v:Python \s 亦匹配
}

/// 退出码证据:字符串 response 走 dsh-shell 渲染契约(见 [`split_string_response`]);
/// 对象 response 读 Bash 的 `status`/`exit_code`/`exit`;`interrupted` → 130;`is_error` → 1。
/// 无任何证据(response 缺失/无退出码字段)→ [`None`]——落盘为 `null`,
/// 不臆造 0;Stop 折叠按失败处理(W4-001 D1,不虚报)。
fn exit_code(response: Option<&Value>) -> Option<i64> {
    if let Some(text) = response.and_then(Value::as_str) {
        let (_, status) = split_string_response(text);
        // 无尾标记 = 干净退出 0——dsh-shell 官方契约,非臆造
        return Some(status.unwrap_or(0));
    }
    let obj = response.and_then(Value::as_object)?;
    if obj.get("interrupted").and_then(Value::as_bool) == Some(true) {
        return Some(130);
    }
    for key in ["status", "exit_code", "exit"] {
        if let Some(code) = obj.get(key).and_then(Value::as_i64) {
            return Some(code);
        }
    }
    (obj.get("is_error").and_then(Value::as_bool) == Some(true)).then_some(1)
}

/// 一行摘要:字符串 response 取去尾标记后的正文末行;对象 response 取
/// stdout(空则 stderr)最后一条非空行,截 `MAX_SUMMARY`。
fn summary_line(response: Option<&Value>) -> String {
    if let Some(text) = response.and_then(Value::as_str) {
        let (body, _) = split_string_response(text);
        let last = body.lines().map(str::trim).rfind(|l| !l.is_empty());
        return clip(last.unwrap_or_default());
    }
    let Some(obj) = response.and_then(Value::as_object) else {
        return String::new();
    };
    for key in ["stdout", "stderr"] {
        let last = obj
            .get(key)
            .and_then(Value::as_str)
            .and_then(|text| text.lines().map(str::trim).rfind(|l| !l.is_empty()));
        if let Some(last) = last {
            return clip(last);
        }
    }
    String::new()
}

/// claude 形制失败事件的退出码线索(W11-002 真机捕获):失败侧载荷**无
/// `tool_response`**,真码在顶层 `error` 串头 —— `Exit code 101` 或
/// `Error: Exit code 101` 前缀,后随合并输出。解析出**非零**码才认:失败事件
/// 在场即非零证据,串头 `Exit code 0`/非数字码视同无码,走调用方默认链(绝不落 0)。
fn exit_code_of_error_head(text: &str) -> Option<i64> {
    let head = text.trim_start();
    let head = head.strip_prefix("Error: ").unwrap_or(head).trim_start();
    let rest = head.strip_prefix("Exit code ")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let code = digits.parse::<i64>().ok()?;
    (code > 0).then_some(code)
}

/// 字符串 response 拆解(W9-003 实测 + dsh-shell 渲染契约):DSH 桥的 bash
/// 回执是纯文本,尾部标记 `[exit code: N]`(非零失败)或 `[killed by signal:
/// X]`(信号杀死)由 dsh-tool-bash 渲染器追加;**两者皆无 = 干净退出 0**
/// (dsh-shell `parseExitStatus` 官方语义)。返回 (去标记正文, 终态:
/// Some(N)/Some(130)/None=干净 0)。正文中段的同形文本不受影响(锚定 \n 前缀
/// + 整串尾)。
fn split_string_response(text: &str) -> (&str, Option<i64>) {
    let trimmed = text.trim_end_matches('\n');
    if let Some(idx) = trimmed.rfind("\n[killed by signal: ") {
        return (&trimmed[..idx], Some(130));
    }
    if let Some(idx) = trimmed.rfind("\n[exit code: ") {
        let digits = &trimmed[idx + "\n[exit code: ".len()..];
        if let Some(n) = digits.strip_suffix(']').and_then(|d| d.parse::<i64>().ok()) {
            return (&trimmed[..idx], Some(n));
        }
    }
    // 标记独占全串的退化形(正文为空):"\n[exit code: N]" 去掉首 \n 后即头锚
    if let Some(rest) = trimmed.strip_prefix("[exit code: ")
        && let Some(n) = rest.strip_suffix(']').and_then(|d| d.parse::<i64>().ok())
    {
        return ("", Some(n));
    }
    (trimmed, None)
}

/// 按字符数截断(非字节;承 Python 切片语义)。
fn clip(s: &str) -> String {
    s.chars().take(MAX_SUMMARY).collect()
}

/// 本地 ISO8601 秒级 ts(带时区偏移,如 `2026-09-13T21:00:00+08:00`;spec §4.2)。
/// W3-004 起 impl 上收 `model::ts_now`(与 `generated_at` 同源同格式,发现 9)。
fn ts_now() -> String {
    crate::model::ts_now()
}
