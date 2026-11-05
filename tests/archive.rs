//! W15-05 CLI `archive` 子命令与 `--wave <id>` 历史波次透视端到端测试(子进程回放真实二进制)。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

const EXE: &str = env!("CARGO_BIN_EXE_agentdash");

fn run(args: &[&str]) -> Output {
    Command::new(EXE)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn agentdash")
}

fn run_en(args: &[&str]) -> Output {
    Command::new(EXE)
        .args(args)
        .env("AGENTDASH_LANG", "en")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn agentdash en")
}

struct TempRepo(PathBuf);

impl TempRepo {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "agentdash-cli-archive-{tag}-{}-{serial}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".agentdash")).expect("create .agentdash");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn path_str(&self) -> String {
        self.0.to_str().expect("utf8 path").to_owned()
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn cli_archive_seal_list_strict_force_prune_and_exit_codes() {
    let repo = TempRepo::new("lifecycle");
    let root = repo.path_str();
    let dash = repo.path().join(".agentdash");

    // 1. 空仓库 --list 恒退 0(双语)
    let empty_zh = run(&["archive", "--list", &root]);
    assert_eq!(empty_zh.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&empty_zh.stdout).contains("archive: 暂无归档波次"),
        "中文空列表提示: {}",
        String::from_utf8_lossy(&empty_zh.stdout)
    );
    let empty_en = run_en(&["archive", "list", &root]);
    assert_eq!(empty_en.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&empty_en.stdout).contains("archive: no archived waves"),
        "英文空列表提示: {}",
        String::from_utf8_lossy(&empty_en.stdout)
    );

    // 2. 无台账封存 -> 退 1
    let missing = run(&["archive", &root]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("未找到台账"));

    // 3. 用法错 -> 退 2(非法波次 ID、互斥旗标、多余路径、未知旗标)
    for bad_args in [
        &["archive", "--wave", "../escape", &root][..],
        &["archive", "--wave=", &root][..],
        &["archive", "--list", "--force", &root][..],
        &["archive", "--prune-events", "--keep-events", &root][..],
        &["archive", "--bogus", &root][..],
        &["archive", &root, &root][..],
    ] {
        let out = run(bad_args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "bad_args={bad_args:?} 应退 2, stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // 4. 台账无 wave 字段且未传 --wave -> 退 1;传入 --wave W1 则成功
    fs::write(
        dash.join("ledger.json"),
        r#"{
          "$schema": "agentdash.tasklog.v1",
          "title": "Wave One Title",
          "tasks": {
            "W1-01": {
              "label": "first task",
              "state": "done",
              "started_at": "2026-10-01T08:00:00Z",
              "done_at": "2026-10-01T08:05:00Z"
            }
          }
        }"#,
    )
    .expect("write W1 ledger without wave");
    fs::write(
        dash.join("events.jsonl"),
        "{\"kind\":\"gate\",\"gate\":\"cargo-test\",\"state\":\"passed\",\"exit\":0,\"detail\":\"ok\",\"host\":\"claude\",\"ts\":\"2026-10-01T08:05:10Z\"}\n",
    )
    .expect("write W1 events");

    let no_id = run_en(&["archive", &root]);
    assert_eq!(no_id.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&no_id.stderr).contains("requires a wave ID"));

    let sealed = run(&["archive", "--wave", "W1", "--strict", &root]);
    assert_eq!(
        sealed.status.code(),
        Some(0),
        "strict seal W1 failed: {}",
        String::from_utf8_lossy(&sealed.stderr)
    );
    assert!(
        String::from_utf8_lossy(&sealed.stdout).contains("已归档 W1 -> .agentdash/waves/W1"),
        "中文归档回执: {}",
        String::from_utf8_lossy(&sealed.stdout)
    );

    // 5. 重复封存无 --force -> 退 1;加 --force -> 退 0
    let dup = run(&["archive", "--wave=W1", &root]);
    assert_eq!(dup.status.code(), Some(1));
    let forced = run_en(&["archive", "--wave=W1", "--force", "--prune-events", &root]);
    assert_eq!(forced.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&forced.stdout).contains("archived W1 -> .agentdash/waves/W1"));

    // 6. --list 与 --list --json 输出校验
    let list_out = run(&["archive", "-l", &root]);
    assert_eq!(list_out.status.code(), Some(0));
    let list_text = String::from_utf8_lossy(&list_out.stdout);
    assert!(
        list_text.contains("W1")
            && list_text.contains("strict✓")
            && list_text.contains("Wave One Title")
    );

    let json_out = run(&["archive", "--list", "--json", &root]);
    assert_eq!(json_out.status.code(), Some(0));
    let parsed: serde_json::Value =
        serde_json::from_slice(&json_out.stdout).expect("valid index json");
    assert_eq!(parsed["$schema"], "agentdash.waves.index.v1");
    assert_eq!(parsed["latest"], "W1");
}

#[test]
fn cli_historical_wave_flag_across_render_oneline_and_stats() {
    let repo = TempRepo::new("history-views");
    let root = repo.path_str();
    let dash = repo.path().join(".agentdash");

    // 1. 封存 W1
    fs::write(
        dash.join("ledger.json"),
        r#"{
          "$schema": "agentdash.tasklog.v1",
          "wave": "W1",
          "title": "历史第一波",
          "lanes": [{"name": "L1", "tasks": ["W1-01"]}],
          "tasks": {
            "W1-01": {
              "label": "建立归档协议",
              "state": "done",
              "started_at": "2026-10-10T09:00:00+08:00",
              "done_at": "2026-10-10T09:10:00+08:00"
            }
          }
        }"#,
    )
    .expect("write W1");
    fs::write(
        dash.join("events.jsonl"),
        "{\"kind\":\"gate\",\"gate\":\"cargo-test\",\"state\":\"passed\",\"exit\":0,\"detail\":\"w1 green\",\"host\":\"claude\",\"ts\":\"2026-10-10T09:10:05+08:00\"}\n",
    )
    .expect("write W1 events");

    let s1 = run(&["archive", "--strict", "--prune-events", &root]);
    assert_eq!(s1.status.code(), Some(0));

    // 2. 原地覆写活跃 ledger.json 为进行中的 W2
    fs::write(
        dash.join("ledger.json"),
        r#"{
          "$schema": "agentdash.tasklog.v1",
          "wave": "W2",
          "title": "当前活跃第二波",
          "tasks": {
            "W2-01": {"label": "新任务进行中", "state": "active"}
          }
        }"#,
    )
    .expect("write active W2");

    // 3. 删除 waves/W1/events.jsonl 模拟从 Git 新克隆后通过 --wave W1 查看全套历史视图
    fs::remove_file(dash.join("waves").join("W1").join("events.jsonl"))
        .expect("simulate gitignored wave events");

    let oneline_w1 = run(&["oneline", "--wave", "W1", &root]);
    assert_eq!(oneline_w1.status.code(), Some(0));
    let ol_text = String::from_utf8_lossy(&oneline_w1.stdout);
    assert!(
        ol_text.contains("✓1▶0·0") && !ol_text.contains("W2"),
        "oneline --wave W1 应显示历史 W1(✓1▶0·0)而非活跃 W2: {ol_text}"
    );

    let panel_w1 = run(&["render", "panel", "--wave=W1", &root]);
    assert_eq!(panel_w1.status.code(), Some(0));
    let p_text = String::from_utf8_lossy(&panel_w1.stdout);
    assert!(p_text.contains("历史第一波") && p_text.contains("建立归档协议"));
    assert!(!p_text.contains("当前活跃第二波"));

    let svg_w1 = run(&["render", "graph", "--format", "svg", "--wave", "W1", &root]);
    assert_eq!(svg_w1.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&svg_w1.stdout).contains("<svg"));

    let digest_w1 = run(&["render", "digest", "--strict", "--wave", "W1", &root]);
    assert_eq!(
        digest_w1.status.code(),
        Some(0),
        "新克隆无 events.jsonl 时凭 meta.json 回填后 digest --strict 仍应退 0:\n{}",
        String::from_utf8_lossy(&digest_w1.stdout)
    );

    let stats_w1 = run(&["stats", "--wave", "W1", &root]);
    assert_eq!(stats_w1.status.code(), Some(0));
    let st_text = String::from_utf8_lossy(&stats_w1.stdout);
    assert!(
        st_text.contains("claude") && st_text.contains("cargo-test") && st_text.contains("10m")
    );

    // 4. 不存在的历史波次 -> 退 1;watch 传 --wave -> 退 2
    let nope = run(&["render", "panel", "--wave", "W999", &root]);
    assert_eq!(nope.status.code(), Some(1));
    let watch_wave = run(&["watch", "--wave", "W1", &root]);
    assert_eq!(watch_wave.status.code(), Some(2));

    // 5. 再归档一个 W2,测试 `render panel <root>/.agentdash/waves/*` 过滤 index.json 且无伪告警
    fs::write(
        dash.join("ledger.json"),
        r#"{
          "$schema": "agentdash.tasklog.v1",
          "wave": "W2",
          "title": "历史第二波",
          "tasks": {
            "W2-01": {"label": "新任务已完成", "state": "done"}
          }
        }"#,
    )
    .expect("write done W2");
    assert_eq!(run(&["archive", "--strict", &root]).status.code(), Some(0));

    let glob_pat = format!("{}/.agentdash/waves/*", root.replace('\\', "/"));
    let multi_waves = run(&["render", "panel", &glob_pat]);
    assert_eq!(multi_waves.status.code(), Some(0));
    let mw_stdout = String::from_utf8_lossy(&multi_waves.stdout);
    assert!(
        mw_stdout.contains("历史第一波") && mw_stdout.contains("历史第二波"),
        "waves/* 应展开全部归档波次:\n{mw_stdout}"
    );
    assert!(
        !mw_stdout.contains("no data sources found"),
        "waves/* 必须排除 index.json,不得渲染伪空态警告:\n{mw_stdout}"
    );

    // 6. `archive --json list` 旗标前置顺序兼容
    let json_list = run(&["archive", "--json", "list", &root]);
    assert_eq!(json_list.status.code(), Some(0));
}
