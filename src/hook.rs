//! 集成包钩子入口:`agentdash hook <event>`(spec §6 修订:二进制直调,零 Python 前置)。
//!
//! 读 stdin 全量 JSON 载荷,按事件追加一行(spec §4.2)到 `<cwd>/.agentdash/events.jsonl`:
//! - posttooluse:bash 命中验证门(cargo test/clippy/fmt、go test、npm test、gh pr checks)
//!   → `gate` running 行 + 退出码/一行摘要暂存 `pending_gate.json` **槽位数组**
//!   (同刻多个在途 gate 各占一槽,Stop 折叠须经落盘交接;旧单对象格式读入兼容);
//!   否则 `tool` phase=end + exit + summary
//! - stop:把在途 gate 逐槽折叠为各自 passed/failed(exit + detail 摘要行),消费后删除暂存;
//!   退出码不可知(暂存 exit 为 `null`)记 `failed` + detail 尾注 `(exit unknown)`——
//!   不虚报通过(W4-001 D1)
//! - pretooluse:子代理派发工具(`Task`/`Agent`)→ `agent` dispatched(who=
//!   `agentType`/`subagent_type`/`name`,缺省 `agent`;task=`description` 截 80,
//!   缺省省略)
//! - subagentstop:`agent` completed(载荷带 `agent_name`/`who` 则透传)
//!
//! events.jsonl 轮转(W2-008):追加前检查文件大小,超过 5MB 滚动为
//! `events.jsonl.1`(覆盖旧 .1)再新建,防单文件无限增长。
//!
//! 铁律:自身任何失败(损坏/空 stdin、非对象载荷、IO 错误)一律静默退出 0,
//! 绝不向宿主报错阻塞会话。并发追加经 `.agentdash/.lock` 文件锁自旋
//! (`create_new` 循环,上限重试后按降级铁律退化直接写)保证零丢失;
//! 持有方崩溃残留的超龄陈锁(mtime 超阈值)由后续获取方摘除自愈。

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

/// PostToolUse:gate 提取或 tool 行。
fn on_post_tool_use(payload: &Map<String, Value>, host: Option<&str>) {
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
    let dir = events_dir(payload);

    if tool == "bash" {
        let command = input
            .and_then(|i| i.get("command"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if let Some(gate) = gate_name(command) {
            // running 行与暂存同临界区:保证 Stop 折叠读到的暂存与 running 行配对
            with_lock(&dir, || {
                append_line(
                    &dir,
                    &stamp(
                        json!({ "ts": ts_now(), "kind": "gate", "gate": gate, "state": "running" }),
                        host,
                    ),
                );
                append_pending_slot(
                    &dir,
                    &json!({
                        "gate": gate,
                        "exit": exit_code(response),
                        "detail": summary_line(response),
                    }),
                );
            });
            return;
        }
        append_tool_event(&dir, &tool, response, command, host);
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

/// Stop:把暂存的每个在途 gate 逐槽折叠为各自 passed/failed;暂存无论解析
/// 成败都消费删除(折叠只做一次)。槽缺 `gate` 字段跳过,不产生幽灵事件。
fn on_stop(payload: &Map<String, Value>, host: Option<&str>) {
    let dir = events_dir(payload);
    with_lock(&dir, || {
        let pending = fs::read_to_string(dir.join(PENDING_NAME))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok());
        let _ = fs::remove_file(dir.join(PENDING_NAME));
        let slots = pending_slots(pending);
        if slots.is_empty() {
            return; // 无在途 gate / 暂存损坏:不产生幽灵事件
        }
        for slot in slots {
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

/// `SubagentStop`:`agent` completed 行;载荷带 `agent_name`/`who` 则透传。
fn on_subagent_stop(payload: &Map<String, Value>, host: Option<&str>) {
    let who = ["agent_name", "who"].iter().find_map(|key| {
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

/// gate 交接暂存(多槽位):读入既有暂存归一成数组后追加一槽,同刻多个
/// 在途 gate 各占一槽,Stop 时逐槽折叠各自终态。读-并-写同在锁内临界区;
/// 暂存失败只损失折叠,不损 events.jsonl。
fn append_pending_slot(dir: &Path, slot: &Value) {
    let path = dir.join(PENDING_NAME);
    let existing = fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    let mut slots = pending_slots(existing);
    slots.push(slot.clone());
    let Ok(mut file) = File::create(&path) else {
        return;
    };
    let _ = file.write_all(serde_json::to_string(&slots).unwrap_or_default().as_bytes());
}

/// 验证门命令匹配:连续词序列 + 词边界(承 Python 版 `\bcargo\s+test\b` 的
/// 相邻语义):目标序列的词必须在命令里**连续**出现,词间只容空白或一枚
/// `&&`/`;` 分隔符,不得跨任意中间 token——`npm run test` 不匹配 npm-test、
/// `go build ./... && test` 不匹配 go-test,而 `cargo build && cargo test`
/// (后段连续)与 `cd x && cargo clippy -- -D warnings` 匹配;按序首中即返。
/// `pub(crate)` 供 W4-005 性质测试(词边界不变式需直连纯函数)。
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

/// 退出码证据:Bash 的 `status`/`exit_code`/`exit`;`interrupted` → 130;`is_error` → 1。
/// 无任何证据(response 缺失/非对象/无退出码字段)→ [`None`]——落盘为 `null`,
/// 不臆造 0;Stop 折叠按失败处理(W4-001 D1,不虚报)。
fn exit_code(response: Option<&Value>) -> Option<i64> {
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

/// 一行摘要:stdout(空则 stderr)最后一条非空行,截 `MAX_SUMMARY`。
fn summary_line(response: Option<&Value>) -> String {
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

/// 按字符数截断(非字节;承 Python 切片语义)。
fn clip(s: &str) -> String {
    s.chars().take(MAX_SUMMARY).collect()
}

/// 本地 ISO8601 秒级 ts(带时区偏移,如 `2026-09-13T21:00:00+08:00`;spec §4.2)。
/// W3-004 起 impl 上收 `model::ts_now`(与 `generated_at` 同源同格式,发现 9)。
fn ts_now() -> String {
    crate::model::ts_now()
}
