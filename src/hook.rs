//! 集成包钩子入口:`agentdash hook <event>`(spec §6 修订:二进制直调,零 Python 前置)。
//!
//! 读 stdin 全量 JSON 载荷,按事件追加一行(spec §4.2)到 `<cwd>/.agentdash/events.jsonl`:
//! - posttooluse:bash 命中验证门(cargo test/clippy/fmt、go test、npm test、gh pr checks)
//!   → `gate` running 行 + 退出码/一行摘要暂存 `pending_gate.json`(Stop 折叠须经落盘交接);
//!   否则 `tool` phase=end + exit + summary
//! - stop:把在途 gate 折叠为 passed/failed(exit + detail 摘要行),消费后删除暂存
//! - subagentstop:`agent` completed(载荷带 `agent_name`/`who` 则透传)
//!
//! 铁律:自身任何失败(损坏/空 stdin、非对象载荷、IO 错误)一律静默退出 0,
//! 绝不向宿主报错阻塞会话。并发追加经 `.agentdash/.lock` 文件锁自旋
//! (`create_new` 循环,上限重试后按降级铁律退化直接写)保证零丢失。

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

/// 摘要字段与 detail 的最大字符数(承 Python 参考实现语义)。
const MAX_SUMMARY: usize = 80;
/// 目录约定(spec §4):`<repo>/.agentdash/` 下的三个文件名。
const LOCK_NAME: &str = ".lock";
const PENDING_NAME: &str = "pending_gate.json";
const EVENTS_NAME: &str = "events.jsonl";
/// 锁自旋参数:上限重试后退化直接写(绝不让宿主 hook 长等待)。
const LOCK_MAX_WAIT: Duration = Duration::from_secs(2);
const LOCK_SLEEP: Duration = Duration::from_millis(2);

/// hook 子命令入口。事件名来自 CLI 参数(如 `agentdash hook posttooluse`),
/// 缺省时回退载荷 `hook_event_name`(老版本宿主防御;载荷只有 `tool_name` 时视为
/// PostToolUse)。恒退 0。
#[must_use]
pub fn run(event: Option<&str>) -> ExitCode {
    let mut bytes = Vec::new();
    // 读失败不退出:按空载荷降级,后面 JSON 解析自然跳过
    let _ = std::io::stdin().read_to_end(&mut bytes);
    // 非 UTF-8 字节按 replacement 降级(承 Python 版 stdio errors=replace),保住可解析部分
    let raw = String::from_utf8_lossy(&bytes);
    let Ok(payload) = serde_json::from_str::<Value>(&raw) else {
        return ExitCode::SUCCESS; // 损坏/空 stdin:静默
    };
    let Some(obj) = payload.as_object() else {
        return ExitCode::SUCCESS; // 非对象载荷:静默
    };
    match resolve_event(event, obj).as_deref() {
        Some("posttooluse") => on_post_tool_use(obj),
        Some("stop") => on_stop(obj),
        Some("subagentstop") => on_subagent_stop(obj),
        _ => {} // 未知事件名:静默
    }
    ExitCode::SUCCESS
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
fn on_post_tool_use(payload: &Map<String, Value>) {
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
                    &json!({ "ts": ts_now(), "kind": "gate", "gate": gate, "state": "running" }),
                );
                write_pending(
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
        append_tool_event(&dir, &tool, response, command);
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
    append_tool_event(&dir, &tool, response, summary);
}

/// `tool` 事件:phase=end + exit + 一行摘要(截 80)。
fn append_tool_event(dir: &Path, tool: &str, response: Option<&Value>, summary: &str) {
    with_lock(dir, || {
        append_line(
            dir,
            &json!({
                "ts": ts_now(),
                "kind": "tool",
                "tool": tool,
                "phase": "end",
                "exit": exit_code(response),
                "summary": clip(summary),
            }),
        );
    });
}

/// Stop:折叠在途 gate 为 passed/failed;暂存无论解析成败都消费删除(折叠只做一次)。
fn on_stop(payload: &Map<String, Value>) {
    let dir = events_dir(payload);
    with_lock(&dir, || {
        let pending = fs::read_to_string(dir.join(PENDING_NAME))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok());
        let _ = fs::remove_file(dir.join(PENDING_NAME));
        let Some(pending) = pending.filter(|p| {
            p.get("gate")
                .and_then(Value::as_str)
                .is_some_and(|g| !g.is_empty())
        }) else {
            return; // 无在途 gate / 暂存损坏:不产生幽灵事件
        };
        let gate = pending
            .get("gate")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let exit = pending.get("exit").and_then(Value::as_i64);
        let state = if exit == Some(0) { "passed" } else { "failed" };
        let detail = clip(
            pending
                .get("detail")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
        append_line(
            &dir,
            &json!({
                "ts": ts_now(),
                "kind": "gate",
                "gate": gate,
                "state": state,
                "exit": exit,
                "detail": detail,
            }),
        );
    });
}

/// `SubagentStop`:`agent` completed 行;载荷带 `agent_name`/`who` 则透传。
fn on_subagent_stop(payload: &Map<String, Value>) {
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
    // 注:events.rs 重放(W1-003)对 agent 事件要求 who+task 双字段;SubagentStop 载荷无 task,
    // 本行按车道 brief 只发 who(重放侧记一条警告行,不中断)——接口缺口已在 task-8b 报告上报。
    with_lock(&dir, || append_line(&dir, &event));
}

/// 临界区包装:拿到 `.lock`(`create_new` 自旋,超时退化)后执行 `f`,退出即删锁。
/// 锁文件可能因进程崩溃残留:超时方按降级铁律直接写(单行小写入近似原子,
/// 常态零丢失由锁保证)。
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

/// 锁获取:先到先得(`create_new`);被占则自旋等待;超时/异常 → false(退化直接写)。
fn acquire_lock(lock_path: &Path) -> bool {
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

/// 单行追加:整行(含换行)拼好后一次 `write_all`,锁内调用保证并发零丢失。
fn append_line(dir: &Path, event: &Value) {
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

/// gate 交接暂存:退出码 + 一行摘要(暂存失败只损失折叠,不损 events.jsonl)。
fn write_pending(dir: &Path, pending: &Value) {
    let Ok(mut file) = File::create(dir.join(PENDING_NAME)) else {
        return;
    };
    let _ = file.write_all(pending.to_string().as_bytes());
}

/// 验证门命令匹配:词序列 + 词边界 + 词间空白(承 Python 版 `\b` 正则语义):
/// `xcargo test` 不匹配,`cd x && cargo clippy -- -D warnings` 匹配;按序首中即返。
fn gate_name(command: &str) -> Option<&'static str> {
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

/// 词序列匹配:`\b`词`\s+`(词间一个以上空白)`\b`词…
fn match_word_seq(haystack: &str, words: &[&str]) -> bool {
    let bytes = haystack.as_bytes();
    let mut pos = 0;
    for (idx, word) in words.iter().enumerate() {
        let Some(at) = find_word(bytes, word, pos) else {
            return false;
        };
        let end = at + word.len();
        if idx + 1 == words.len() {
            return true;
        }
        let gap = bytes[end..].iter().take_while(|b| is_ws_byte(**b)).count();
        if gap == 0 {
            return false; // 词间须有空白(正则 \s+)
        }
        pos = end + gap;
    }
    true // 仅 words 为空时可达;本模块恒传非空词序列
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

/// 退出码:Bash 的 `status`/`exit_code`/`exit`;`interrupted` → 130;`is_error` → 1;不可知 → 0。
fn exit_code(response: Option<&Value>) -> i64 {
    let Some(obj) = response.and_then(Value::as_object) else {
        return 0;
    };
    if obj.get("interrupted").and_then(Value::as_bool) == Some(true) {
        return 130;
    }
    for key in ["status", "exit_code", "exit"] {
        if let Some(code) = obj.get(key).and_then(Value::as_i64) {
            return code;
        }
    }
    i64::from(obj.get("is_error").and_then(Value::as_bool) == Some(true))
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
fn ts_now() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    let offset = local_utc_offset(now);
    format_ts(now + offset, offset)
}

/// (本地纪元秒, 偏移秒) → `YYYY-MM-DDTHH:MM:SS±HH:MM`。偏移 0 输出 `+00:00`
/// (承 Python `isoformat` 语义)。
fn format_ts(local_secs: i64, offset_secs: i64) -> String {
    let days = local_secs.div_euclid(86_400);
    let day_secs = local_secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (sign, off) = if offset_secs < 0 {
        ("-", -offset_secs)
    } else {
        ("+", offset_secs)
    };
    format!(
        "{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}{sign}{oh:02}:{om:02}",
        h = day_secs / 3_600,
        m = (day_secs % 3_600) / 60,
        s = day_secs % 60,
        oh = off / 3_600,
        om = (off % 3_600) / 60,
    )
}

/// 天序数(1970-01-01 = 0)→ (年, 月, 日)。Hinnant 算法,常规日期域内无溢出。
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(windows)]
/// (年, 月, 日) → 天序数。仅 Windows 偏移差分(GetLocalTime/GetSystemTime)使用。
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400; // [0, 399]
    let mp = if month > 2 { month - 3 } else { month + 9 }; // [0, 11]
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(windows)]
/// 本地 UTC 偏移秒:kernel32 `GetLocalTime` 与 `GetSystemTime` 同瞬时差分
/// (自动含 DST;按分钟取整吸收两次取时之间的秒级间隙)。
fn local_utc_offset(_utc: i64) -> i64 {
    #[repr(C)]
    #[derive(Default)]
    struct SysTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        millis: u16,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocalTime(out: *mut SysTime);
        fn GetSystemTime(out: *mut SysTime);
    }
    let pseudo = |st: &SysTime| {
        days_from_civil(i64::from(st.year), i64::from(st.month), i64::from(st.day)) * 86_400
            + i64::from(st.hour) * 3_600
            + i64::from(st.minute) * 60
            + i64::from(st.second)
    };
    // SAFETY:两 API 均只向调用方提供的单个 SYSTEMTIME(8×u16)写入,无失败路径;
    // 指针指向本栈帧内变量,调用期间线程不被重入。
    unsafe {
        let mut local = SysTime::default();
        let mut system = SysTime::default();
        GetLocalTime(std::ptr::addr_of_mut!(local));
        GetSystemTime(std::ptr::addr_of_mut!(system));
        let drift = pseudo(&local) - pseudo(&system);
        (drift + 30).div_euclid(60) * 60
    }
}

#[cfg(all(unix, target_pointer_width = "64"))]
/// 本地 UTC 偏移秒:libc `localtime_r` 的 `tm_gmtoff`(glibc/musl/macOS 的
/// struct tm 布局一致:9×int 后按对齐排 long;LP64 下 time_t = i64)。
fn local_utc_offset(utc: i64) -> i64 {
    #[repr(C)]
    struct Tm {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        wday: i32,
        yday: i32,
        isdst: i32,
        gmtoff: i64,
    }
    unsafe extern "C" {
        fn localtime_r(time: *const i64, out: *mut Tm) -> *mut Tm;
    }
    // SAFETY:localtime_r 线程安全(结果写入调用方缓冲,不触碰静态区);Tm 的
    // repr(C) 布局与主流 libc struct tm 一致,仅读取 gmtoff 字段。
    unsafe {
        let mut tm = Tm {
            sec: 0,
            min: 0,
            hour: 0,
            mday: 0,
            mon: 0,
            year: 0,
            wday: 0,
            yday: 0,
            isdst: 0,
            gmtoff: 0,
        };
        if localtime_r(&utc, &mut tm).is_null() {
            return 0;
        }
        if tm.gmtoff.abs() >= 86_400 {
            return 0; // 离谱偏移按损坏处理,退化 UTC
        }
        tm.gmtoff
    }
}

#[cfg(all(unix, not(target_pointer_width = "64")))]
/// 非 LP64:不做 struct tm 布局假设,退化为 UTC 偏移。
fn local_utc_offset(_utc: i64) -> i64 {
    0
}

#[cfg(not(any(windows, unix)))]
/// 未知平台:退化为 UTC 偏移。
fn local_utc_offset(_utc: i64) -> i64 {
    0
}
