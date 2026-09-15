//! W1-002 契约层集成测试(spec §4.1):合法全量 / 损坏→Corrupt / 最小核 / 富态降级+警告,外加 schema 一致性。
//!
//! 本 crate 一期为二进制(无 lib 目标),按 `#[path]` 直接纳入被测模块。

#[path = "../src/contract.rs"]
mod contract;

use contract::{ContractError, TaskState, parse_ledger};

/// spec §4.1 的合法全量样例(含未知字段、整数任务引用、六种状态)。
const FULL: &str = r#"{
  "$schema": "agentdash.tasklog.v1",
  "wave": "W23",
  "title": "波次标题",
  "profile": "sdd",
  "lanes":  [{"name": "A-impl", "tasks": [1, 2]}],
  "tasks": {
    "1": {"label": "动词短语", "state": "active", "note": "fix round 2/5"},
    "2": {"label": "待办", "state": "pending"},
    "3": {"label": "复核中", "state": "review"},
    "4": {"label": "返修", "state": "fix-round", "note": "round 2"},
    "5": {"label": "完成", "state": "done"},
    "6": {"label": "阻塞", "state": "blocked"}
  },
  "barriers": [{"id": "B1", "after": [1], "unlocks": [2]}],
  "future_extension": {"anything": [1, true, null]}
}"#;

fn assert_corrupt(text: &str) {
    match parse_ledger(text) {
        Err(ContractError::Corrupt(_)) => {}
        other => panic!("expected `ContractError::Corrupt`, got {other:?}"),
    }
}

#[test]
fn full_legal_ledger_parses_all_fields() {
    let ledger = parse_ledger(FULL).expect("legal ledger must parse");
    assert_eq!(ledger.wave.as_deref(), Some("W23"));
    assert_eq!(ledger.title, "波次标题");
    assert_eq!(ledger.profile.as_deref(), Some("sdd"));
    assert!(
        ledger.warnings.is_empty(),
        "legal ledger must not warn: {:?}",
        ledger.warnings
    );

    assert_eq!(ledger.lanes.len(), 1);
    assert_eq!(ledger.lanes[0].name, "A-impl");
    assert_eq!(ledger.lanes[0].tasks, vec!["1", "2"]);

    assert_eq!(ledger.tasks.len(), 6);
    assert_eq!(ledger.tasks["1"].label, "动词短语");
    assert_eq!(ledger.tasks["1"].state, TaskState::Active);
    assert_eq!(ledger.tasks["1"].note.as_deref(), Some("fix round 2/5"));
    assert_eq!(ledger.tasks["2"].state, TaskState::Pending);
    assert_eq!(ledger.tasks["2"].note, None);
    assert_eq!(ledger.tasks["3"].state, TaskState::Review);
    assert_eq!(ledger.tasks["4"].state, TaskState::FixRound);
    assert_eq!(ledger.tasks["5"].state, TaskState::Done);
    assert_eq!(ledger.tasks["6"].state, TaskState::Blocked);

    assert_eq!(ledger.barriers.len(), 1);
    assert_eq!(ledger.barriers[0].id, "B1");
    assert_eq!(ledger.barriers[0].after, vec!["1"]);
    assert_eq!(ledger.barriers[0].unlocks, vec!["2"]);
}

#[test]
fn corrupt_inputs_report_corrupt() {
    assert_corrupt("");
    assert_corrupt("{ not json");
    assert_corrupt("{\"title\": 5}"); // 必填字段类型不符
    assert_corrupt("{\"tasks\": {}}"); // 缺必填 title
    assert_corrupt("{\"title\":\"x\",\"tasks\":{\"1\":{\"label\":\"y\",\"state\":\"running\"}}}"); // 未知状态串
    assert_corrupt("{\"title\":\"x\"}{\"title\":\"y\"}"); // 尾随垃圾

    let err = parse_ledger("{ not json").expect_err("must be corrupt");
    assert!(
        err.to_string().contains("corrupt ledger.json"),
        "display: {err}"
    );
}

#[test]
fn minimal_core_with_empty_tasks_is_legal() {
    let ledger =
        parse_ledger(r#"{"title": "最小核", "tasks": {}}"#).expect("minimal core must parse");
    assert_eq!(ledger.title, "最小核");
    assert_eq!(ledger.wave, None);
    assert_eq!(ledger.profile, None);
    assert!(ledger.tasks.is_empty());
    assert!(ledger.lanes.is_empty());
    assert!(ledger.barriers.is_empty());
    assert!(ledger.warnings.is_empty());

    let explicit = parse_ledger(r#"{"$schema": "agentdash.tasklog.v1", "title": "x"}"#)
        .expect("known $schema must parse");
    assert!(explicit.tasks.is_empty());
    assert!(explicit.warnings.is_empty());
}

#[test]
fn rich_states_without_profile_degrade_to_active_with_warnings() {
    let text = r#"{
        "title": "无 profile 富态",
        "tasks": {
            "1": {"label": "复核中", "state": "review", "note": "round 2/5"},
            "2": {"label": "返修", "state": "fix-round"}
        }
    }"#;
    let ledger = parse_ledger(text).expect("degradation is not an error");
    assert_eq!(ledger.profile, None);
    assert_eq!(ledger.tasks["1"].state, TaskState::Active);
    assert_eq!(ledger.tasks["2"].state, TaskState::Active);
    assert_eq!(ledger.tasks["1"].label, "复核中");
    assert_eq!(
        ledger.tasks["1"].note.as_deref(),
        Some("round 2/5"),
        "降级不改其他字段"
    );
    assert_eq!(ledger.warnings.len(), 2, "warnings: {:?}", ledger.warnings);
    assert!(
        ledger.warnings[0].starts_with("task 1"),
        "warnings sorted by task id: {:?}",
        ledger.warnings
    );
    assert!(ledger.warnings.iter().all(|w| w.contains("active")));
}

#[test]
fn rich_states_with_declared_profile_stay_rich() {
    let text = r#"{"profile": "sdd", "title": "sdd 富态", "tasks": {
        "1": {"label": "复核中", "state": "review"},
        "2": {"label": "返修", "state": "fix-round"}
    }}"#;
    let ledger = parse_ledger(text).expect("declared rich states are legal");
    assert_eq!(ledger.tasks["1"].state, TaskState::Review);
    assert_eq!(ledger.tasks["2"].state, TaskState::FixRound);
    assert!(ledger.warnings.is_empty());
}

#[test]
fn unknown_schema_id_warns_but_parses() {
    let ledger = parse_ledger(r#"{"$schema": "other.tool.v9", "title": "异构工具"}"#)
        .expect("unknown $schema degrades, not corrupt");
    assert_eq!(ledger.title, "异构工具");
    assert_eq!(ledger.warnings.len(), 1);
    assert!(ledger.warnings[0].contains("other.tool.v9"));
    assert!(ledger.warnings[0].contains("agentdash.tasklog.v1"));
}

#[test]
fn task_state_as_str_matches_serde_roundtrip() {
    let all = [
        TaskState::Pending,
        TaskState::Active,
        TaskState::Review,
        TaskState::FixRound,
        TaskState::Done,
        TaskState::Blocked,
    ];
    for state in all {
        let json = serde_json::to_string(&state).expect("serialize");
        assert_eq!(json, format!("\"{}\"", state.as_str()));
        let back: TaskState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, state);
    }
}

// ---------- W3-004 可选 `milestones`(声明式里程碑,加法扩展) ----------

/// W3-004:`milestones` 合法形态逐字段解析(id/title/tasks;整数引用统一收
/// 字符串);字段缺省时为空(v1 合成兜底在模型层,契约层只管形状)。
#[test]
fn milestones_field_parses_and_defaults_to_empty() {
    let text = r#"{
        "title": "声明式里程碑",
        "milestones": [
            {"id": "M1", "title": "契约扩展", "tasks": ["01", 2]},
            {"id": "M2", "title": "渲染接线"}
        ]
    }"#;
    let ledger = parse_ledger(text).expect("milestones 是加法字段,合法");
    assert_eq!(ledger.milestones.len(), 2);
    assert_eq!(ledger.milestones[0].id, "M1");
    assert_eq!(ledger.milestones[0].title, "契约扩展");
    assert_eq!(
        ledger.milestones[0].tasks,
        vec!["01", "2"],
        "任务引用沿用 lanes 先例:整数统一收字符串"
    );
    assert_eq!(ledger.milestones[1].id, "M2", "tasks 缺省为空组,不要求非空");
    assert!(ledger.milestones[1].tasks.is_empty());
    assert!(
        ledger.warnings.is_empty(),
        "合法 milestones 不得告警: {:?}",
        ledger.warnings
    );

    let bare = parse_ledger(r#"{"title": "无声明"}"#).expect("缺省 milestones 合法");
    assert!(
        bare.milestones.is_empty(),
        "缺省 = 空表(模型层据此走 v1 合成)"
    );
    assert!(bare.warnings.is_empty());
}

/// W3-004 降级铁律(契约侧):`milestones` 结构损坏(非数组 / 缺必填 /
/// 引用类型非法)→ 整账**不 corrupt**,降级为空表 + 恰一条点名警告,其余
/// 字段照常解析——模型层据此走单里程碑合成兜底(与 lanes/tasks 损坏即
/// `Corrupt` 的口径不同:milestones 是可选增强,坏了自己退,不拖垮台账)。
#[test]
fn malformed_milestones_degrade_to_warning_not_corrupt() {
    let cases = [
        r#"{"title":"x","milestones":"nope"}"#,              // 非数组
        r#"{"title":"x","milestones":[{"title":"缺 id"}]}"#, // 缺必填 id
        r#"{"title":"x","milestones":[{"id":"M1","tasks":"01"}]}"#, // tasks 非数组
        r#"{"title":"x","milestones":[{"id":"M1","tasks":[true]}]}"#, // 引用类型非法
    ];
    for text in cases {
        let ledger = parse_ledger(text).expect("milestones 损坏降级,不判 corrupt");
        assert!(
            ledger.milestones.is_empty(),
            "损坏 milestones 降级为空表: {text}"
        );
        assert_eq!(
            ledger.warnings.len(),
            1,
            "恰一条警告: {text} → {:?}",
            ledger.warnings
        );
        assert!(
            ledger.warnings[0].contains("milestones"),
            "警告点名 milestones 字段: {:?}",
            ledger.warnings
        );
        assert_eq!(ledger.title, "x", "降级只折损 milestones,其余字段照常");
    }

    // `null` 视同缺省(宽容口径),不告警
    let null = parse_ledger(r#"{"title":"x","milestones":null}"#).expect("null = 缺省");
    assert!(null.milestones.is_empty() && null.warnings.is_empty());
}

fn schema_text() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/schema/agentdash.tasklog.v1.json"
    );
    std::fs::read_to_string(path).expect("schema file readable at CARGO_MANIFEST_DIR/schema/")
}

/// 跟随本地 `$ref`(`#/...`,最深 8 层)取到实际节点。
fn resolve<'a>(
    schema: &'a serde_json::Value,
    node: &'a serde_json::Value,
) -> &'a serde_json::Value {
    let mut current = node;
    for _ in 0..8 {
        let Some(reference) = current.get("$ref").and_then(|v| v.as_str()) else {
            return current;
        };
        assert!(reference.starts_with("#/"), "local ref only: {reference}");
        current = schema;
        for part in reference.trim_start_matches("#/").split('/') {
            current = &current[part];
        }
        assert!(!current.is_null(), "dangling ref `{reference}`");
    }
    panic!("$ref chain deeper than 8: {node}");
}

#[test]
fn schema_file_agrees_with_parser() {
    let schema: serde_json::Value =
        serde_json::from_str(&schema_text()).expect("schema is valid JSON");

    assert_eq!(schema["$id"], "agentdash.tasklog.v1");
    assert_eq!(
        schema["properties"]["$schema"]["const"],
        "agentdash.tasklog.v1"
    );
    assert!(
        schema["required"]
            .as_array()
            .expect("required list")
            .iter()
            .any(|v| v == "title"),
        "schema must require `title` (parser rejects its absence)"
    );

    let tasks_def = resolve(&schema, &schema["properties"]["tasks"]);
    let state_enum: Vec<String> = tasks_def["additionalProperties"]["properties"]["state"]["enum"]
        .as_array()
        .expect("state enum")
        .iter()
        .map(|v| v.as_str().expect("string state").to_string())
        .collect();
    let mut from_schema = state_enum.clone();
    from_schema.sort();
    let mut from_parser: Vec<String> = [
        TaskState::Pending,
        TaskState::Active,
        TaskState::Review,
        TaskState::FixRound,
        TaskState::Done,
        TaskState::Blocked,
    ]
    .iter()
    .map(|s| s.as_str().to_string())
    .collect();
    from_parser.sort();
    assert_eq!(
        from_schema, from_parser,
        "schema enum must equal parser states"
    );

    for state in &state_enum {
        let doc = format!(
            r#"{{"title":"t","profile":"sdd","tasks":{{"1":{{"label":"L","state":"{state}"}}}}}}"#
        );
        let ledger = parse_ledger(&doc).expect("every schema state must parse");
        assert_eq!(ledger.tasks["1"].state.as_str(), state.as_str());
        assert!(ledger.warnings.is_empty());
    }

    // W3-004:schema 与解析器同步携带可选 `milestones`(id/title 必填,tasks
    // 沿用 taskIds 引用形态),且 `milestones` 不得进入 required(旧文件照验通过)
    let milestones_def = resolve(&schema, &schema["properties"]["milestones"]);
    let required: Vec<String> = milestones_def["items"]["required"]
        .as_array()
        .expect("milestone required list")
        .iter()
        .map(|v| v.as_str().expect("string key").to_string())
        .collect();
    assert!(
        required.contains(&"id".to_owned()) && required.contains(&"title".to_owned()),
        "milestone 条目必填 id/title: {required:?}"
    );
    assert!(
        !schema["required"]
            .as_array()
            .expect("required list")
            .iter()
            .any(|v| v == "milestones"),
        "milestones 是可选字段,不得入 required"
    );
}

#[test]
fn schema_example_parses_clean() {
    let schema: serde_json::Value =
        serde_json::from_str(&schema_text()).expect("schema is valid JSON");
    let text = serde_json::to_string(&schema["examples"][0]).expect("example serializable");
    let ledger = parse_ledger(&text).expect("schema example must parse");
    assert_eq!(ledger.title, "波次标题");
    assert!(
        ledger.warnings.is_empty(),
        "warnings: {:?}",
        ledger.warnings
    );
}

// ------------------------------------------------------------ done_at(W5-001)

#[test]
fn done_at_passthrough_and_absent() {
    let ledger = parse_ledger(
        r#"{"title":"t","tasks":{
            "1":{"label":"a","state":"done","done_at":"2026-09-15T10:00:00+08:00"},
            "2":{"label":"b","state":"pending"}
        }}"#,
    )
    .expect("合法台账");
    assert_eq!(
        ledger.tasks["1"].done_at.as_deref(),
        Some("2026-09-15T10:00:00+08:00"),
        "done_at 原样穿透"
    );
    assert_eq!(ledger.tasks["2"].done_at, None, "缺省为 None(加法兼容)");
}
