//! W4-005:手搓解析器的性质测试(proptest,dev-only)。
//!
//! 覆盖面(spec D4):
//! - `utc_timestamp` ∘ `rfc3339_to_secs` 往返一致(Hinnant 民法 + 格式化 + 解析);
//! - 任意畸形输入解析**不 panic**(垃圾必 `None` 或可解析,绝不崩);
//! - `±HH:MM` 本地偏移按绝对时刻折算正确;
//! - `parse_fix_round` 残余 note 性质(前缀恰抑,残余 `trim` 后原样保留);
//! - `gate_name` 词边界不变式(空白/`&&` 间隔可匹配,粘连/插词必拒);
//! - `gate_name_with` 用户自定义表同词序列机 + 用户优先(W10-003);
//! - `parse_custom_gates` 任意输入不 panic、合法长名截 40(W10-003);
//! - `civil_from_days` / `days_from_civil` 互逆(Windows CI 专属,互逆对
//!   只在该目标编译)。
//!
//! agentdash 是纯二进制 crate:按 `#[path]` 在 crate 根挂载模块树(同
//! `tests/merge.rs` 约定);挂载源中本测试未触达的 pub 项属死代码,文件级放行。

#![allow(dead_code)]

#[path = "../src/contract.rs"]
mod contract;
#[path = "../src/events.rs"]
mod events;
#[path = "../src/hook.rs"]
mod hook;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/sources/mod.rs"]
mod sources;

use proptest::prelude::*;

use model::rfc3339_to_secs;

/// 年 9999 上界的纪元秒( civil 算法常规域,与内核判定一致)。
const MAX_SECS: u64 = 253_402_300_799;

proptest! {
    #![proptest_config(ProptestConfig {
        // W6-004:失败案例落 crate 根 proptest-regressions/(可提交回放)
        failure_persistence: Some(Box::new(
            proptest::test_runner::FileFailurePersistence::Direct("proptest-regressions"),
        )),
        ..ProptestConfig::with_cases(256)
    })]

    /// P1:自有输出往返——`utc_timestamp(secs)` 重解析必须还原 `secs`。
    #[test]
    fn utc_timestamp_roundtrips(secs in 0u64..=MAX_SECS) {
        let stamp = model::utc_timestamp(secs);
        prop_assert_eq!(rfc3339_to_secs(&stamp), Some(secs), "stamp={}", stamp);
    }

    /// P2:畸形输入不 panic——任意垃圾串解析要么 `None` 要么有值,绝不崩
    /// (手搓字节索引是全仓最脆面,该性质钉住"崩溃不可能")。
    #[test]
    fn rfc3339_never_panics_on_garbage(
        head in "[0-9]{0,5}[-:T].{0,25}",
        tail in prop::option::of("(Z|[+-][0-9]{0,3}:?[0-9]{0,3})"),
    ) {
        let mut text = head;
        if let Some(t) = tail {
            text.push_str(&t);
        }
        let _ = rfc3339_to_secs(&text);
    }

    /// P3:`Z` 换 `+HH:00` 偏移后,绝对时刻应少整 off 小时(本地墙钟 → 纪元秒
    /// 的折算方向);下界 100_000 保证减去 23h 不下溢。
    #[test]
    fn offset_form_folds_to_instant(
        secs in 100_000u64..=MAX_SECS,
        off in 0u64..=23,
    ) {
        let utc = model::utc_timestamp(secs);
        let local = format!("{}+{off:02}:00", &utc[..utc.len() - 1]);
        prop_assert_eq!(rfc3339_to_secs(&local), Some(secs - off * 3_600), "local={}", local);
    }

    /// P4:合法 `fix round D/M <残余>` 前缀必解析,D/M 保真,残余 `trim` 后
    /// 原样保留(全角空格 U+3000 会触发归一化,策略中替换掉以聚焦本性质)。
    #[test]
    fn fix_round_residual_preserved(
        d in 0u32..1_000,
        m in 1u32..1_000,
        rest in ".*",
    ) {
        let rest = rest.replace('\u{3000}', " ");
        let note = format!("fix round {d}/{m} {rest}");
        let parsed = model::parse_fix_round(&note);
        prop_assert!(parsed.is_some(), "合法前缀必解析: note={:?}", note);
        let ((rd, rm), residual) = parsed.unwrap();
        prop_assert_eq!((rd, rm), (d, m));
        prop_assert_eq!(residual, rest.trim());
    }

    /// P5a:六门词序列在任意纯空白间隔与 `&&` 链后段均可命中(空白形态不变式)。
    #[test]
    fn gate_matches_across_blank_and_separator(ws in "[ \t\n]{1,3}") {
        let cases = [
            ("cargo test", "cargo-test"),
            ("cargo clippy", "cargo-clippy"),
            ("cargo fmt", "cargo-fmt"),
            ("go test", "go-test"),
            ("npm test", "npm-test"),
            ("gh pr checks", "gh-pr-checks"),
        ];
        for (cmd, name) in cases {
            let spaced = cmd.split(' ').collect::<Vec<_>>().join(&ws);
            prop_assert_eq!(hook::gate_name(&spaced), Some(name), "纯空白间隔: {}", spaced);
            let chained = format!("echo{ws}&&{ws}{spaced}");
            prop_assert_eq!(hook::gate_name(&chained), Some(name), "&& 链后段: {}", chained);
        }
    }

    /// P5b:粘连(`cargoxtest`)与插词(`cargo x test`)必拒——词边界完整性。
    /// g1 排除独立词 `go`:`cargo go test` 文本上确含合法 `go test` 门(承
    /// Python 参考的文本匹配语义,非缺陷,真值由下方点断言钉住);该例由
    /// CI macOS 种子首抓,本机 256 案例未及(regex 策略下 `go` 低频)。
    #[test]
    fn gate_rejects_glued_or_interrupted_words(
        g1 in "[a-z]{1,3}".prop_filter("exclude standalone `go`", |w| *w != "go"),
        g2 in "[a-z]{1,3}",
    ) {
        prop_assert_eq!(hook::gate_name(&format!("cargo{g1}test")), None);
        prop_assert_eq!(hook::gate_name(&format!("cargo {g1} test")), None);
        prop_assert_eq!(hook::gate_name(&format!("go {g1}build{g2} test")), None);
    }

    /// P7(W10-003):自定义 gate 表走**同一词序列机**——词边界语义与表来源
    /// 无关(空白/`&&` 间隔可命中,粘连必拒),用户表先行(命中即用用户名),
    /// 未命中回落内置。junk 同 P5b 排除 `go`。
    #[test]
    fn custom_table_word_sequence_invariants(
        ws in "[ \t\n]{1,3}",
        junk in "[a-z]{1,3}".prop_filter("exclude standalone `go`", |w| *w != "go"),
    ) {
        let custom = vec![hook::CustomGate {
            name: "ut".to_owned(),
            words: vec!["pytest".to_owned()],
        }];
        let spaced = hook::gate_name_with(&format!("pytest{ws}-q"), &custom);
        prop_assert_eq!(spaced.as_deref(), Some("ut"), "纯空白间隔: 用户词表命中");
        let chained_cmd = format!("echo{ws}&&{ws}pytest{ws}-q");
        let chained = hook::gate_name_with(&chained_cmd, &custom);
        prop_assert_eq!(chained.as_deref(), Some("ut"), "&& 链后段: 用户词表命中");
        let glued_cmd = format!("pytest{junk}q");
        let glued = hook::gate_name_with(&glued_cmd, &custom);
        prop_assert_eq!(glued.as_deref(), None, "粘连必拒(与内置同规)");
        let fallback = hook::gate_name_with("cargo test", &custom);
        prop_assert_eq!(
            fallback.as_deref(),
            Some("cargo-test"),
            "用户表未命中回落内置"
        );
        // 用户优先:同名覆盖——用户 words 先查,内置表无 `cargo check` 门
        let over = vec![hook::CustomGate {
            name: "cargo-test".to_owned(),
            words: vec!["cargo".to_owned(), "check".to_owned()],
        }];
        let overridden = hook::gate_name_with("cargo check", &over);
        prop_assert_eq!(overridden.as_deref(), Some("cargo-test"));
    }

    /// P8(W10-003):config.json 解析面对任意输入不 panic(损坏/形状错 →
    /// `None` → 整表回退,降级铁律的结构面);`(?s)` 让 `.` 覆盖换行。
    #[test]
    fn custom_config_parse_never_panics(
        text in proptest::string::string_regex("(?s).{0,200}").unwrap(),
    ) {
        let _ = hook::parse_custom_gates(&text);
    }

    /// P9(W10-003):合法字符集的长名**截 40 归一**(非整表回退)——名长
    /// 是长度问题、字符集才是裁断;词表 1..=8 界内时解析必成功。
    #[test]
    fn custom_gate_long_name_clipped_not_rejected(name in "[a-z0-9_-]{41,80}") {
        let text = format!(r#"{{"gates":[{{"name":"{name}","words":["w"]}}]}}"#);
        let gates = hook::parse_custom_gates(&text);
        prop_assert!(gates.is_some(), "合法形状必解析: name={name}");
        let parsed = gates.unwrap();
        prop_assert_eq!(parsed.len(), 1);
        let clipped = &name[..40];
        prop_assert_eq!(&parsed[0].name, clipped, "名截 40(ASCII 单字节)");
        prop_assert_eq!(&parsed[0].words, &["w".to_owned()][..]);
    }
}

#[test]
fn gate_textual_match_cargo_go_test_is_go_test() {
    // W4-005 CI 发现的词边界真值钉版:门匹配是文本级的——`cargo go test`
    // 中段独立词 `go` 与尾词 `test` 构成合法 `go-test` 门(生产语义正确,
    // 首版 P5b 过宽断言误判为必拒)。
    assert_eq!(hook::gate_name("cargo go test"), Some("go-test"));
}

// Windows 专属:民法互逆对只有该目标编译(days_from_civil 为 cfg(windows))。
#[cfg(windows)]
mod windows_only {
    use proptest::prelude::*;

    use super::model;

    proptest! {
        #![proptest_config(ProptestConfig {
        // W6-004:失败案例落 crate 根 proptest-regressions/(可提交回放)
        failure_persistence: Some(Box::new(
            proptest::test_runner::FileFailurePersistence::Direct("proptest-regressions"),
        )),
        ..ProptestConfig::with_cases(256)
    })]

        /// P6:`days_from_civil(civil_from_days(d)) == d`(9999 年域内互逆)。
        #[test]
        fn civil_days_roundtrip(days in 0i64..=2_932_896) {
            let (y, m, d) = model::civil_from_days(days);
            prop_assert_eq!(model::days_from_civil(y, m, d), days);
        }
    }
}
