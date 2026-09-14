//! 写回核心(W2-006 T6):把 TUI 的 `d`/`b`/`m` 操作落回
//! `<repo>/.agentdash/ledger.json`——投影层唯一的显式写副作用。
//!
//! - [`apply`]:读台账 → 改指定任务的 `state`/`note` → 原子写回(tmp +
//!   rename);JSON 以 `serde_json::Value` 直改,未知字段与未触及任务原样保留;
//! - 并发:沿用 hook 的 `.agentdash/.lock` 文件锁模式(`create_new` 自旋、
//!   陈锁自愈、超时退化)——与事件追加共用同一把锁,写回与 hook 跨进程串行;
//! - 幂等即报错:目标态已达成(已是 done/blocked、备注原样、清空空备注)→
//!   `Err`,"没有改动"必须显式呈现给状态行,不静默;
//! - 原子性:先写 `ledger.json.tmp`,回读校验 JSON 合法后才 rename 顶替正身
//!   ——写中途崩溃最多留 tmp 残骸,ledger.json 只有旧/新两态,绝无半文件。
//!
//! 自包含设计:不引 crate 其他模块(不依赖 `crate::` 路径),因为本文件被
//! `tui.rs` 以相对 `#[path]` 挂为子模块(见 tui.rs 挂载注释),同时
//! `tests/writeback.rs` 也独立挂载本文件跑 apply 与锁测试。

use std::fs::{self, OpenOptions};
use std::io;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use serde_json::{Map, Value};

/// 锁与事件追加共用 `.agentdash/.lock`(hook 同名,跨进程互斥)。
const LOCK_NAME: &str = ".lock";
/// 锁自旋上限(承 hook:超时按降级铁律直接写,绝不让 TUI 长等待)。
const LOCK_MAX_WAIT: Duration = Duration::from_secs(2);
/// 锁自旋步进(承 hook)。
const LOCK_SLEEP: Duration = Duration::from_millis(2);
/// 陈锁判定阈值(承 hook:持有方崩溃残留可摘除自愈)。
const LOCK_STALE: Duration = Duration::from_secs(10);

/// 写回动作(TUI `d`/`b`/`m` → ledger.json 的最小改面)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// 标记完成(`d`):`state → "done"`。
    MarkDone,
    /// 标记阻塞(`b`):`state → "blocked"`。
    MarkBlocked,
    /// 写备注(`m` 行编辑,⏎ 提交):`note ← 文本`;空文本 = 清除备注。
    SetNote(String),
}

impl Action {
    /// 成功消息与目标态共用的落点标签(done/blocked/note)。
    fn label(&self) -> &'static str {
        match self {
            Self::MarkDone => "done",
            Self::MarkBlocked => "blocked",
            Self::SetNote(_) => "note",
        }
    }
}

/// 对 `<repo>/.agentdash/ledger.json` 执行写回动作。
///
/// 成功返回可显示消息(如 `已写回 T2:done`);失败返回可显示原因(无台账 /
/// 台账损坏 / 任务不在台账 / 已是目标态 / IO 错误),调用方原样上状态行。
///
/// # Errors
/// 台账缺失或损坏、任务不在台账、目标态已达成(幂等命中,含备注原样)与
/// 读写 IO 失败;幂等命中也走 `Err`——与"已是目标→Err"的约定一致。
pub fn apply(repo: &Path, task_id: &str, action: Action) -> Result<String, String> {
    let dir = repo.join(".agentdash");
    let path = dir.join("ledger.json");
    if !path.is_file() {
        return Err(format!("无台账:{} 不存在", path.display()));
    }
    with_lock(&dir, || {
        let label = action.label();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Err(format!("无台账:{} 不存在", path.display()));
            }
            Err(err) => return Err(format!("读台账失败:{}:{err}", path.display())),
        };
        let mut ledger: Value =
            serde_json::from_str(&text).map_err(|err| format!("台账损坏:{err}"))?;
        let Some(obj) = ledger.as_object_mut() else {
            return Err(String::from("台账损坏:顶层不是 JSON 对象"));
        };
        mutate(obj, task_id, action)?;
        let pretty =
            serde_json::to_string_pretty(&ledger).map_err(|err| format!("台账序列化失败:{err}"))?;
        atomic_write(&path, &format!("{pretty}\n"))?;
        Ok(format!("已写回 {task_id}:{label}"))
    })
    .unwrap_or_else(|| Err(format!("写回失败:{} 目录不可用", dir.display())))
}

/// 修改 `tasks.<task_id>` 的 state/note(锁内调用):无 tasks 表、无该任务、
/// 任务规格非对象或已是目标态 → `Err`,台账内存副本保持未改(不落盘)。
fn mutate(obj: &mut Map<String, Value>, task_id: &str, action: Action) -> Result<(), String> {
    let label = action.label();
    let Some(tasks) = obj.get_mut("tasks").and_then(Value::as_object_mut) else {
        return Err(format!("任务 {task_id} 不在台账(tasks 表缺失或非对象)"));
    };
    let Some(spec) = tasks.get_mut(task_id).and_then(Value::as_object_mut) else {
        return Err(format!("任务 {task_id} 不在台账"));
    };
    match action {
        Action::MarkDone | Action::MarkBlocked => {
            if spec.get("state").and_then(Value::as_str) == Some(label) {
                return Err(format!("任务 {task_id} 已是 {label}(未改动)"));
            }
            spec.insert(String::from("state"), Value::String(String::from(label)));
        }
        Action::SetNote(text) => set_note(spec, task_id, text)?,
    }
    Ok(())
}

/// 备注写入:原样备注 → Err;空文本 = 清除 note 键(本就无备注/为 null 则
/// 视为未改动);其余整体替换。注意此处的 `text` 已由 tui 层 trim。
fn set_note(spec: &mut Map<String, Value>, task_id: &str, text: String) -> Result<(), String> {
    if spec.get("note").and_then(Value::as_str) == Some(text.as_str()) {
        return Err(format!("任务 {task_id} 备注未变化"));
    }
    if text.is_empty() {
        match spec.get("note") {
            None | Some(Value::Null) => return Err(format!("任务 {task_id} 备注未变化")),
            Some(_) => {
                spec.remove("note");
            }
        }
    } else {
        spec.insert(String::from("note"), Value::String(text));
    }
    Ok(())
}

/// 原子写回:先落 `ledger.json.tmp` 并回读校验 JSON 合法,再 rename 顶替正身;
/// 校验不过或 rename 失败都尽力摘除 tmp——写中途崩溃最坏留下 tmp 残骸,
/// ledger.json 只有旧/新两态,绝无半文件。
fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, content).map_err(|err| format!("写回失败:{}:{err}", tmp.display()))?;
    let valid = fs::read_to_string(&tmp)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some();
    if !valid {
        let _ = fs::remove_file(&tmp);
        return Err(format!(
            "写回校验失败:{} 内容不合法,已放弃顶替正身",
            tmp.display()
        ));
    }
    if fs::rename(&tmp, path).is_err() {
        // 兜底(个别平台/占用场景 rename 不覆盖既有目标):先摘旧再顶,窗口极小
        let _ = fs::remove_file(path);
        fs::rename(&tmp, path).map_err(|err| format!("写回失败:{}:{err}", path.display()))?;
    }
    Ok(())
}

/// 临界区包装(承 hook `with_lock` 同一模式,泛化出返回值):拿到 `.lock` 后
/// 执行 `f`,退出即删锁(仅在自己真持有才删);目录建不出 → `None`。拿不到
/// 锁超时退化仍执行 `f`(降级铁律:单次原子替换,丢改风险远小于让用户干等)。
fn with_lock<T>(dir: &Path, f: impl FnOnce() -> T) -> Option<T> {
    if fs::create_dir_all(dir).is_err() {
        return None;
    }
    let lock_path = dir.join(LOCK_NAME);
    let held = acquire_lock(&lock_path);
    let out = f();
    if held {
        let _ = fs::remove_file(&lock_path);
    }
    Some(out)
}

/// 锁获取(承 hook):先到先得;被占则自旋等待。锁文件 mtime 超过
/// [`LOCK_STALE`] 判为陈锁 → 摘除后重抢(等待前查一次,自旋超时后再兜底
/// 一次)。摘除与获取同走 `create_new` 原子语义。最终拿不到 → false(退化
/// 直接写)。
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

/// 自旋抢锁:`create_new` 循环,[`LOCK_SLEEP`] 步进,[`LOCK_MAX_WAIT`] 上限;
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
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
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

/// 陈锁判定:锁文件 mtime 距今超过 [`LOCK_STALE`]。metadata/mtime 取不到
/// (恰被他人摘除)或 mtime 在未来(时钟偏差)→ 非陈锁,照常等待。
fn is_stale_lock(lock_path: &Path) -> bool {
    fs::metadata(lock_path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|mtime| SystemTime::now().duration_since(mtime).ok())
        .is_some_and(|age| age >= LOCK_STALE)
}
