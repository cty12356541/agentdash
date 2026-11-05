//! W15-04 波次归档内核集成测试:
//! - `validate_wave_id` 安全校验(路径穿越、首尾点、Windows 保留设备名、超长拦截);
//! - 双代事件(`events.jsonl.1` + `events.jsonl`)合并去重、同秒保序与 `done_at` 后置验证门切片;
//! - 连续波次水线切分与 `--force` 重封存不自我投毒(`WritePolicy::ForceOverwrite`);
//! - 原子封存产出 6 份产物(`ledger.json`/`events.jsonl`/`digest.txt`/`stats.txt`/`graph.svg`/`meta.json`)
//!   + `waves/index.json`、FNV-1a64 校验、`--strict` 预检拦截与损坏索引自愈。

#![allow(dead_code)]

#[path = "../src/archive.rs"]
mod archive;
#[path = "../src/contract.rs"]
mod contract;
#[path = "../src/events.rs"]
mod events;
#[path = "../src/hook.rs"]
mod hook;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/render/mod.rs"]
mod render;
#[path = "../src/sources/mod.rs"]
mod sources;
#[path = "../src/stats.rs"]
mod stats;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use archive::{
    ArchiveOptions, EventRetention, StrictPolicy, WaveError, WritePolicy, archive_wave, list_waves,
    natural_wave_cmp, resolve_wave_dir, validate_wave_id,
};

struct TempRepo(PathBuf);

impl TempRepo {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "agentdash-archive-unit-{tag}-{}-{serial}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".agentdash")).expect("create .agentdash");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn dash(&self) -> PathBuf {
        self.0.join(".agentdash")
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn validate_wave_id_rejects_traversal_dots_and_windows_devices() {
    for valid in ["W1", "W14", "W-PI09", "M6", "v0.13.0-rc_1", "2026-11-05"] {
        assert_eq!(
            validate_wave_id(valid).expect("valid wave id"),
            valid,
            "合法波次 ID 应通过: {valid}"
        );
    }
    assert_eq!(validate_wave_id("  W15  ").expect("trimmed wave id"), "W15");

    let long_65 = "W".repeat(65);
    for invalid in [
        "",
        "   ",
        ".",
        "..",
        "../W1",
        "W1/../W2",
        "W1/sub",
        "W1\\sub",
        ".hidden",
        "-flag",
        "_leading",
        "trailing.",
        "W1..W2",
        "W1:colon",
        "W1 space",
        "波次一",
        "CON",
        "con",
        "NUL",
        "aux.v1",
        "prn",
        "COM1",
        "com9.txt",
        "LPT1",
        "lpt9.bak",
        long_65.as_str(),
    ] {
        assert!(
            validate_wave_id(invalid).is_err(),
            "非法波次 ID 必须拒绝: {invalid:?}"
        );
    }
}

#[test]
fn natural_wave_cmp_sorts_numerically_across_prefixes() {
    let mut ids = vec![
        "W14".to_owned(),
        "W2".to_owned(),
        "W10".to_owned(),
        "W1".to_owned(),
        "W-PI10".to_owned(),
        "W-PI06".to_owned(),
        "M2".to_owned(),
    ];
    ids.sort_by(|a, b| natural_wave_cmp(a, b));
    assert_eq!(
        ids,
        vec!["M2", "W-PI06", "W-PI10", "W1", "W2", "W10", "W14"]
    );

    // 零填充数字段与后缀的严格全序传递性回归("W001" < "W01a" < "W1b",无环)
    assert_eq!(
        natural_wave_cmp("W001", "W1b"),
        std::cmp::Ordering::Less,
        "无后缀的 W001 必须排在有后缀的 W1b 之前"
    );
    assert_eq!(natural_wave_cmp("W001", "W01a"), std::cmp::Ordering::Less);
    assert_eq!(natural_wave_cmp("W01a", "W1b"), std::cmp::Ordering::Less);
}

#[test]
fn event_slicing_merges_rotated_gen_preserves_same_second_order_and_post_done_gate() {
    let repo = TempRepo::new("slice-rot");
    let dash = repo.dash();

    // W14 台账:任务完成于 12:00:00,但最终验证门在 12:00:05 通过(符合先盖 done_at 再跑门禁的收口铁律)
    fs::write(
        dash.join("ledger.json"),
        r#"{
          "$schema": "agentdash.tasklog.v1",
          "wave": "W14",
          "profile": "sdd",
          "title": "W14 轮转切片测试",
          "tasks": {
            "W14-01": {
              "label": "核心实现",
              "state": "done",
              "started_at": "2026-10-28T11:50:00+08:00",
              "done_at": "2026-10-28T12:00:00+08:00"
            }
          }
        }"#,
    )
    .expect("write ledger");

    // events.jsonl.1 含早于窗口的旧波次事件(10:00:00)、窗口前派发但窗口内完成的子代理、以及接缝重复行
    let seam_line = r#"{"kind":"tool","tool":"bash","phase":"end","exit":0,"summary":"seam","host":"claude","ts":"2026-10-28T11:55:00+08:00"}"#;
    fs::write(
        dash.join("events.jsonl.1"),
        format!(
            "{}\n{}\n{seam_line}\n",
            r#"{"kind":"tool","tool":"bash","phase":"end","exit":0,"summary":"old-wave","host":"claude","ts":"2026-10-28T10:00:00+08:00"}"#,
            r#"{"kind":"agent","event":"dispatched","who":"lane-a","task":"pre-window dispatch","host":"claude","ts":"2026-10-28T11:40:00+08:00"}"#,
        ),
    )
    .expect("write rotated events");

    // events.jsonl 含接缝重复行、窗口内实名 completed、同秒 gate running -> passed,以及 pending_gate.json 已决槽
    fs::write(
        dash.join("events.jsonl"),
        format!(
            "{seam_line}\n{}\n{}\n",
            r#"{"kind":"agent","event":"completed","who":"lane-a","host":"claude","ts":"2026-10-28T11:56:00+08:00"}"#,
            r#"{"kind":"gate","gate":"cargo-clippy","state":"running","host":"claude","ts":"2026-10-28T12:00:05+08:00"}"#,
        ),
    )
    .expect("write live events");

    // 本回合尚未触发 Stop 的已决暂存槽(exit=0):archive 应先冲刷折叠为 passed
    fs::write(
        dash.join("pending_gate.json"),
        r#"[{"gate":"cargo-clippy","exit":0,"detail":"0 warnings","session":"s1"}]"#,
    )
    .expect("write pending gate");

    let receipt = archive_wave(
        repo.path(),
        &ArchiveOptions {
            wave_override: None,
            strict: StrictPolicy::Enforce,
            write_policy: WritePolicy::CreateNew,
            retention: EventRetention::Keep,
            infer: true,
        },
    )
    .expect("strict archive should pass");

    assert_eq!(receipt.wave, "W14");
    assert_eq!(receipt.tasks_done, 1);
    assert_eq!(receipt.tasks_total, 1);
    assert!(
        !dash.join("pending_gate.json").exists(),
        "已决暂存槽应被冲刷消费"
    );

    let wave_dir = dash.join("waves").join("W14");
    for file in [
        "ledger.json",
        "events.jsonl",
        "digest.txt",
        "stats.txt",
        "graph.svg",
        "meta.json",
    ] {
        assert!(wave_dir.join(file).is_file(), "归档产物 `{file}` 必须生成");
    }
    let sliced = fs::read_to_string(wave_dir.join("events.jsonl")).expect("read sliced events");
    assert!(
        !sliced.contains("old-wave"),
        "早于窗口的旧波次 tool 事件应被切掉:\n{sliced}"
    );
    assert!(
        sliced.contains("pre-window dispatch"),
        "跨窗口边界未闭合的 dispatched 必须前插保留以配对窗口内 completed:\n{sliced}"
    );
    assert_eq!(
        sliced.matches("\"summary\":\"seam\"").count(),
        1,
        ".1 与当前文件接缝处的完全相同行应去重为 1 条:\n{sliced}"
    );

    let digest = fs::read_to_string(wave_dir.join("digest.txt")).expect("read digest");
    assert!(
        !digest.contains(" ?"),
        "done_at 之后的通过门被完整保留,不得出现存疑 `done ?`:\n{digest}"
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn sequential_waves_and_force_reseal_do_not_self_poison_watermark() {
    let repo = TempRepo::new("seq-force");
    let dash = repo.dash();

    // 1. 封存 v1.0 无 started_at 的 W1
    fs::write(
        dash.join("ledger.json"),
        r#"{
          "$schema": "agentdash.tasklog.v1",
          "wave": "W1",
          "title": "Wave One",
          "tasks": {
            "T1": {"label": "w1 task", "state": "done", "done_at": "2026-10-01T10:00:00Z"}
          }
        }"#,
    )
    .expect("write W1 ledger");
    fs::write(
        dash.join("events.jsonl"),
        concat!(
            r#"{"kind":"gate","gate":"cargo-test","state":"passed","exit":0,"detail":"w1 ok","host":"claude","ts":"2026-10-01T10:00:05Z"}"#,
            "\n",
        ),
    )
    .expect("write W1 events");

    archive_wave(
        repo.path(),
        &ArchiveOptions {
            wave_override: None,
            strict: StrictPolicy::Enforce,
            write_policy: WritePolicy::CreateNew,
            retention: EventRetention::Keep,
            infer: true,
        },
    )
    .expect("archive W1");

    // 无 --force 重封存同名 W1 必须报 AlreadyExists
    let dup = archive_wave(
        repo.path(),
        &ArchiveOptions {
            wave_override: None,
            strict: StrictPolicy::Permissive,
            write_policy: WritePolicy::CreateNew,
            retention: EventRetention::Keep,
            infer: true,
        },
    );
    assert!(matches!(dup, Err(WaveError::AlreadyExists(_))));

    // 2. 切入 W2(同样无 started_at),追加 W2 事件并开启 Prune
    fs::write(
        dash.join("ledger.json"),
        r#"{
          "$schema": "agentdash.tasklog.v1",
          "wave": "W2",
          "title": "Wave Two",
          "tasks": {
            "T2": {"label": "w2 task", "state": "done", "done_at": "2026-11-05T10:00:00Z"}
          }
        }"#,
    )
    .expect("write W2 ledger");
    fs::write(
        dash.join("events.jsonl"),
        concat!(
            r#"{"kind":"gate","gate":"cargo-test","state":"passed","exit":0,"detail":"w1 ok","host":"claude","ts":"2026-10-01T10:00:05Z"}"#,
            "\n",
            r#"{"kind":"gate","gate":"cargo-test","state":"passed","exit":0,"detail":"w2 ok","host":"agy","ts":"2026-11-05T10:00:05Z"}"#,
            "\n",
        ),
    )
    .expect("write W1+W2 events");

    archive_wave(
        repo.path(),
        &ArchiveOptions {
            wave_override: None,
            strict: StrictPolicy::Enforce,
            write_policy: WritePolicy::CreateNew,
            retention: EventRetention::Prune,
            infer: true,
        },
    )
    .expect("archive W2 with prune");

    let w2_events =
        fs::read_to_string(dash.join("waves").join("W2").join("events.jsonl")).expect("read W2");
    assert!(!w2_events.contains("w1 ok"), "W2 切片应按水线排除 W1 事件");
    assert!(w2_events.contains("w2 ok"), "W2 切片应保留 W2 事件");

    // 3. 在 Prune 之后用 ForceOverwrite 重封存 W2:不得因自身水线或活跃流已清空而丢失已归档的 w2 ok 事件
    let resealed = archive_wave(
        repo.path(),
        &ArchiveOptions {
            wave_override: None,
            strict: StrictPolicy::Enforce,
            write_policy: WritePolicy::ForceOverwrite,
            retention: EventRetention::Keep,
            infer: true,
        },
    )
    .expect("force reseal W2 after prune");
    assert_eq!(resealed.events_count, 1);
    let w2_after =
        fs::read_to_string(dash.join("waves").join("W2").join("events.jsonl")).expect("read W2");
    assert!(w2_after.contains("w2 ok"));

    // 4. 损坏 index.json 后调用 list_waves 自动扫描 waves/*/meta.json 自愈重建
    fs::write(dash.join("waves").join("index.json"), "{corrupt").expect("corrupt index");
    let listed = list_waves(repo.path());
    assert_eq!(listed.waves.len(), 2);
    assert_eq!(listed.waves[0].wave, "W1");
    assert_eq!(listed.waves[1].wave, "W2");

    // 5. resolve_wave_dir 支持大小写不敏感匹配("w2" -> "W2")
    let resolved = resolve_wave_dir(repo.path(), "w2").expect("resolve lowercase w2");
    assert!(resolved.ends_with("W2"));
}

#[test]
fn strict_archive_rejects_incomplete_tasks_or_failed_gates_with_zero_residue() {
    let repo = TempRepo::new("strict-reject");
    let dash = repo.dash();
    fs::write(
        dash.join("ledger.json"),
        r#"{
          "$schema": "agentdash.tasklog.v1",
          "wave": "W99",
          "title": "Incomplete Wave",
          "tasks": {
            "T1": {"label": "done task", "state": "done"},
            "T2": {"label": "still active", "state": "active"}
          }
        }"#,
    )
    .expect("write incomplete ledger");

    let res = archive_wave(
        repo.path(),
        &ArchiveOptions {
            wave_override: None,
            strict: StrictPolicy::Enforce,
            write_policy: WritePolicy::CreateNew,
            retention: EventRetention::Keep,
            infer: true,
        },
    );
    assert!(matches!(res, Err(WaveError::StrictFailed(_))));
    assert!(
        !dash.join("waves").join("W99").exists(),
        "--strict 失败不得留下半成品波次目录"
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn review_regressions_same_second_events_host_flush_and_out_of_order_reseal() {
    let repo = TempRepo::new("review-reg");
    let dash = repo.dash();

    // 1. 同秒两条完全相同的合法事件不得被误去重,且冲刷 pending_gate 保留 host(并支持 agy 延迟 transcript 恢复)
    let tp = repo.path().join("agy_tp.jsonl");
    fs::write(
        &tp,
        r#"{"step_index":7,"type":"GENERIC","content":"Command exited with code 0.\n\nCommand output:\nagygate ok\n"}"#,
    )
    .expect("write agy transcript");

    fs::write(
        dash.join("ledger.json"),
        r#"{
          "$schema": "agentdash.tasklog.v1",
          "wave": "W1",
          "title": "Wave 1",
          "tasks": {
            "W1-01": {
              "label": "task 1",
              "state": "done",
              "started_at": "2026-10-01T10:00:00Z",
              "done_at": "2026-10-01T10:05:00Z"
            }
          }
        }"#,
    )
    .expect("write W1");
    let dup_tool = r#"{"kind":"tool","tool":"read","phase":"end","exit":0,"summary":"Cargo.toml","host":"agy","ts":"2026-10-01T10:01:00Z"}"#;
    fs::write(
        dash.join("events.jsonl"),
        format!(
            "{dup_tool}\n{dup_tool}\n{}\n",
            r#"{"kind":"gate","gate":"cargo-test","state":"running","host":"agy","session_id":"s-agy","ts":"2026-10-01T10:05:01Z"}"#
        ),
    )
    .expect("write W1 events");
    fs::write(
        dash.join("pending_gate.json"),
        serde_json::json!([{
            "gate": "cargo-test",
            "exit": null,
            "detail": "",
            "host": "agy",
            "session": "s-agy",
            "transcript": tp.to_string_lossy(),
            "step_idx": 7
        }])
        .to_string(),
    )
    .expect("write deferred agy slot");

    let r1 = archive_wave(
        repo.path(),
        &ArchiveOptions {
            wave_override: None,
            strict: StrictPolicy::Enforce,
            write_policy: WritePolicy::CreateNew,
            retention: EventRetention::Keep,
            infer: true,
        },
    )
    .expect("W1 strict archive should resolve deferred agy transcript and pass");
    assert_eq!(
        r1.events_count, 4,
        "两条同秒 read + running + 冲刷 passed 共 4 条事件,不得误删相邻同秒行"
    );
    let meta1: archive::ArchiveMeta = serde_json::from_str(
        &fs::read_to_string(dash.join("waves").join("W1").join("meta.json")).expect("read W1 meta"),
    )
    .expect("parse W1 meta");
    assert_eq!(
        meta1.hosts.get("agy"),
        Some(&4),
        "冲刷的 pending_gate 必须归属 host=agy 而非 unknown: {:?}",
        meta1.hosts
    );
    assert!(!meta1.hosts.contains_key("unknown"));

    // 2. 封存未来的 W2(11:00Z),随后切回 W1 执行 ForceOverwrite 重封存:W2 的未来水线不得污染 W1!
    fs::write(
        dash.join("ledger.json"),
        r#"{
          "$schema": "agentdash.tasklog.v1",
          "wave": "W2",
          "title": "Wave 2",
          "tasks": {
            "W2-01": {
              "label": "task 2",
              "state": "done",
              "started_at": "2026-10-01T11:00:00Z",
              "done_at": "2026-10-01T11:05:00Z"
            }
          }
        }"#,
    )
    .expect("write W2");
    fs::write(
        dash.join("events.jsonl"),
        "{\"kind\":\"gate\",\"gate\":\"cargo-test\",\"state\":\"passed\",\"exit\":0,\"detail\":\"w2\",\"host\":\"claude\",\"ts\":\"2026-10-01T11:05:05Z\"}\n",
    )
    .expect("write W2 events");
    archive_wave(
        repo.path(),
        &ArchiveOptions {
            wave_override: None,
            strict: StrictPolicy::Enforce,
            write_policy: WritePolicy::CreateNew,
            retention: EventRetention::Prune,
            infer: true,
        },
    )
    .expect("archive W2");

    // 切回 W1(无 started_at 的 v1.0 形态)重封存:不得被 W2(11:05Z)水线吞掉 10:05Z 事件
    fs::write(
        dash.join("ledger.json"),
        r#"{
          "$schema": "agentdash.tasklog.v1",
          "wave": "W1",
          "title": "Wave 1 v1.0 reseal",
          "tasks": {
            "W1-01": {
              "label": "task 1",
              "state": "done",
              "done_at": "2026-10-01T10:05:00Z"
            }
          }
        }"#,
    )
    .expect("write W1 v1.0");
    let r1_reseal = archive_wave(
        repo.path(),
        &ArchiveOptions {
            wave_override: None,
            strict: StrictPolicy::Enforce,
            write_policy: WritePolicy::ForceOverwrite,
            retention: EventRetention::Keep,
            infer: true,
        },
    )
    .expect("W1 force reseal after W2 exists");
    assert_eq!(r1_reseal.events_count, 4);
}
