//! 自研 glob(W10-001 D1,零依赖):仅路径分量级 `*` 与 `?`,自字面前缀
//! `read_dir` 逐段展开。不支持 `**`(按单分量通配处理,不递归下钻)、不支
//! 持字符组;shell 无关——agentdash 自行展开,引号包裹的模式同样生效。
//!
//! 语义要点(D1):
//! - 不含通配符的参数原样透传,不触文件系统(承单 PATH 现行行为);
//! - 通配段逐段 `read_dir` 过滤,字面段直接拼接,展开结果排序确定;
//! - 模式以分隔符收尾时只收目录(承 shell `dir/*/` 惯例,仓目录场景防
//!   杂文件混入);
//! - 展开为空表交由调用方处理(逐 pattern 产 ⚠ 行,恒退 0,不崩溃)。

use std::path::PathBuf;

/// 参数是否含通配符(分量级 `*` / `?` 任一)。
pub(crate) fn has_wildcard(text: &str) -> bool {
    text.contains(['*', '?'])
}

/// 逐参展开(W10-001):含通配符的参数做分量级展开,无匹配的原串回收进
/// `nomatch`(供调用方逐 pattern 产 ⚠ 行);字面参数原样透传。返回
/// (展开后路径序列, 无匹配 pattern 串序列),各自保持参数序。
pub(crate) fn expand_args(args: &[PathBuf]) -> (Vec<PathBuf>, Vec<String>) {
    let mut repos = Vec::new();
    let mut nomatch = Vec::new();
    for arg in args {
        let pattern = arg.to_string_lossy();
        if has_wildcard(&pattern) {
            let hits = expand(&pattern);
            if hits.is_empty() {
                nomatch.push(pattern.into_owned());
            } else {
                repos.extend(hits);
            }
        } else {
            repos.push(arg.clone());
        }
    }
    (repos, nomatch)
}

/// 展开单个模式:含通配符做分量级展开(无匹配 → 空表);不含则原样透传。
pub(crate) fn expand(pattern: &str) -> Vec<PathBuf> {
    if !has_wildcard(pattern) {
        return vec![PathBuf::from(pattern)];
    }
    let comps = split_components(pattern);
    // 尾随分隔符 → 只收目录(空尾分量本身不参与匹配,效果折入此旗标)
    let dirs_only = pattern
        .chars()
        .next_back()
        .is_some_and(std::path::is_separator);
    let Some(wild_idx) = comps.iter().position(|(_, comp)| has_wildcard(comp)) else {
        return Vec::new();
    };
    // 字面前缀:首个通配分量之前的原串原样切片(盘符/根/前导分隔符零重排);
    // 通配落在首分量时基址取 cwd
    let (base_offset, _) = comps[wild_idx];
    let mut current: Vec<PathBuf> = vec![if base_offset == 0 {
        PathBuf::from(".")
    } else {
        PathBuf::from(&pattern[..base_offset])
    }];
    for &(_, comp) in &comps[wild_idx..] {
        if comp.is_empty() {
            continue; // 前导/双写/尾随分隔符产生的空分量
        }
        if has_wildcard(comp) {
            let mut next: Vec<PathBuf> = Vec::new();
            for dir in &current {
                let Ok(entries) = std::fs::read_dir(dir) else {
                    continue; // 前缀不在场:该分支零命中
                };
                for entry in entries.flatten() {
                    let file_name = entry.file_name();
                    let Some(name) = file_name.to_str() else {
                        continue; // 非 UTF-8 名不参与(不崩溃)
                    };
                    if match_component(comp, name) {
                        next.push(dir.join(entry.file_name()));
                    }
                }
            }
            current = next;
        } else {
            for dir in &mut current {
                dir.push(comp);
            }
        }
    }
    // 字面尾段可能并不存在(承 shell:整路径在场才算命中);尾随分隔符只收目录
    current.retain(|path| {
        if dirs_only {
            path.is_dir()
        } else {
            path.exists()
        }
    });
    current.sort();
    current
}

/// 按路径分隔符切分量(Windows 兼收 `\` 与 `/`),保留各分量在原串中的字
/// 节偏移,供字面前缀原样切片。
fn split_components(pattern: &str) -> Vec<(usize, &str)> {
    let mut comps: Vec<(usize, &str)> = Vec::new();
    let mut start = 0;
    for (idx, ch) in pattern.char_indices() {
        if std::path::is_separator(ch) {
            comps.push((start, &pattern[start..idx]));
            start = idx + ch.len_utf8();
        }
    }
    comps.push((start, &pattern[start..]));
    comps
}

/// 分量匹配:`*` 匹配任一(含空)字符序列、`?` 恰一字符(Unicode 标量),
/// 不跨分量;回溯实现,零正则零依赖。
fn match_component(pattern: &str, name: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let nam: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while ni < nam.len() {
        if pi < pat.len() && (pat[pi] == '?' || pat[pi] == nam[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < pat.len() && pat[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(found) = star {
            pi = found + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    pat[pi..].iter().all(|&ch| ch == '*')
}
