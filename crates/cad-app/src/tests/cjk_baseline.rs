//! The ADR-named CJK-baseline honesty test (ADR 0015 decision B, §acceptance).
//!
//! `cad_app_adds_no_new_cjk_literals_outside_the_recorded_baseline` scans the
//! shipped `crates/cad-app/src` Rust sources (test code excluded) for CJK string
//! literals and compares them against the recorded [`CJK_BASELINE`] below.
//!
//! The ADR fence is "the Rust side must not add new CJK literals": user-visible
//! chrome copy belongs in `crates/cad-ui-core/i18n/{zh-CN,en}.json` (the shell
//! substitutes the catalog label). The pre-existing `label()` / `status_line()` /
//! diagnostic fallbacks are a **recorded deviation**, so the guard's job is to
//! stop *silent* growth, not to forbid the recorded set: a new literal fails until
//! it is added to the baseline as a deliberate, reviewed change (and the honest
//! fix is almost always to route new copy through the i18n catalog instead).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The recorded CJK string literals of the shipped `crates/cad-app/src` sources.
///
/// **Deliberately frozen.** Update this list only as a deliberate change, and
/// prefer routing new user-visible copy through the i18n catalog. The point is to
/// make any growth a conscious edit instead of an unnoticed one.
const CJK_BASELINE: &[&str] = &[
    "EXTEND：目标已延伸至最近的边界（LINE 对 LINE/LWPOLYLINE 子集）",
    "OFFSET：圆偏移，新半径 {distance:.4}",
    "OFFSET：直线沿拾取侧偏移 {distance:.4}",
    "TRIM：保留拾取点一侧（LINE 对 LINE 子集）",
    "TRIM：目标完全位于边界内，整段删除",
    "{}：已选 {} 点",
    "{}：已选 {} 点，可确认",
    "{}：已选 {} 点，还需 {} 点",
    "{}：请先选择对象",
    "从剪贴板粘贴 {} 个实体（原坐标）",
    "修剪",
    "修剪：已选目标与边界，可确认",
    "修剪：请选择要修剪的目标",
    "修剪：请选择边界",
    "偏移",
    "加载中",
    "单位未知",
    "后端 {}（实际运行）",
    "后端 {}（请求 {preference:?}，实际运行 {}）",
    "后端偏好：{choice:?}（渲染会话由宿主重建）",
    "后端失败（请求 {preference:?}）：{}",
    "图元 {}，单位 {:?}",
    "圆",
    "圆弧",
    "复制",
    "多段线",
    "子图元 {} 按整实体{action}（子图元级编辑未支持）",
    "子图元 {} 按整实体删除（子图元级编辑未支持）",
    "子图元 {} 按整实体复制（子图元级编辑未支持）",
    "子图元 {} 按整实体移动（子图元级编辑未支持）",
    "实体 {} 的按对象颜色/线型/线宽覆盖未复制，副本继承图层样式",
    "实例路径引用的实体 {} {action}其块定义，影响所有实例",
    "实例路径引用的实体 {} 删除其块定义，影响所有实例",
    "实例路径引用的实体 {} 复制其块定义，影响所有实例",
    "实例路径引用的实体 {} 已按整实体捕获（只读，不影响实例）",
    "实例路径引用的实体 {} 移动其块定义，影响所有实例",
    "就绪",
    "已切换到三维视图（透视）",
    "已剪切 {count} 个实体（已删除并移入剪贴板）",
    "已回到二维俯视图",
    "已复制 {count} 个实体到剪贴板",
    "平移",
    "延伸",
    "延伸：已选目标与边界，可确认",
    "延伸：请选择要延伸的目标",
    "延伸：请选择边界",
    "强制后端不可用：{reason}",
    "强制后端不可用；可回退到 Auto 或改用另一后端",
    "打开失败",
    "折线长度",
    "旋转",
    "无可用的 WebGPU/WebGL2",
    "无外部资源引用",
    "无工具",
    "椭圆",
    "模式：{mode:?}",
    "正多边形",
    "没有可用的 WebGPU/WebGL2 设备；请更新浏览器或返回 Auto",
    "活动图层：{}",
    "测量",
    "浏览",
    "直线",
    "矩形",
    "移动",
    "空图纸",
    "缩放",
    "范围 {:.3},{:.3} → {:.3},{:.3}",
    "角度",
    "设备初始化失败：{reason}",
    "设备初始化失败；请重试或切换后端",
    "资源引用 {}：{keys}",
    "距离",
    "轨道旋转 yaw={yaw:.4} pitch={pitch:.4}",
    "选择",
    "镜像",
    "面积",
];

/// Whether `ch` is a CJK ideograph, extension-A ideograph, CJK punctuation or a
/// fullwidth form — the same ranges `scripts/check-i18n.py` uses.
fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x3000..=0x303f | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xff00..=0xffef)
}

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Whether `path` is unit-test code (skipped, mirroring `check-i18n.py`): a
/// `tests.rs` file or anything under a `tests/` directory relative to `src`.
fn is_test_source(path: &Path, src_root: &Path) -> bool {
    let is_tests_rs = path.file_name().is_some_and(|name| name == "tests.rs");
    let under_tests = path
        .strip_prefix(src_root)
        .unwrap_or(path)
        .components()
        .any(|component| component.as_os_str() == "tests");
    is_tests_rs || under_tests
}

/// The shipped (non-test) part of a source file: everything before the trailing
/// `#[cfg(test)]` module, which is a fixture rather than chrome.
fn shipped_source(text: &str) -> &str {
    match text.find("#[cfg(test)]") {
        Some(index) => &text[..index],
        None => text,
    }
}

/// Every CJK-bearing double-quoted string literal in `text`, honouring backslash
/// escapes and skipping `//` and `/* */` comments. A lexical walk (like the
/// `check-i18n.py` React scan), deliberately conservative rather than a full
/// parser; raw/byte strings are not special-cased (there are none with CJK here).
fn cjk_literals(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if ch == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
            continue;
        }
        if ch == '"' {
            i += 1;
            let mut literal = String::new();
            while i < chars.len() {
                let current = chars[i];
                if current == '\\' {
                    // Keep the escape so `\n`/`\"` stay distinct from their raw
                    // characters; a CJK character is never escaped.
                    literal.push('\\');
                    if let Some(next) = chars.get(i + 1) {
                        literal.push(*next);
                    }
                    i += 2;
                    continue;
                }
                if current == '"' {
                    i += 1;
                    break;
                }
                literal.push(current);
                i += 1;
            }
            if literal.chars().any(is_cjk) {
                out.push(literal);
            }
            continue;
        }
        i += 1;
    }
    out
}

/// The sorted, deduplicated CJK literals of every shipped source file.
fn shipped_cjk_literals() -> BTreeSet<String> {
    let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src_root, &mut files);
    let mut found = BTreeSet::new();
    for path in files {
        if is_test_source(&path, &src_root) {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read source");
        for literal in cjk_literals(shipped_source(&text)) {
            found.insert(literal);
        }
    }
    found
}

#[test]
fn cad_app_adds_no_new_cjk_literals_outside_the_recorded_baseline() {
    let found = shipped_cjk_literals();
    let baseline: BTreeSet<String> = CJK_BASELINE.iter().map(|s| (*s).to_string()).collect();

    let added: Vec<&String> = found.difference(&baseline).collect();
    let removed: Vec<&String> = baseline.difference(&found).collect();

    if !added.is_empty() {
        for literal in &added {
            eprintln!("NEW_CJK_LITERAL\t{literal:?}");
        }
    }
    if !removed.is_empty() {
        for literal in &removed {
            eprintln!("STALE_CJK_BASELINE\t{literal:?}");
        }
    }

    assert!(
        added.is_empty(),
        "crates/cad-app added {} new CJK string literal(s) not in the recorded baseline: {added:?}\n\
         User-visible copy belongs in crates/cad-ui-core/i18n/{{zh-CN,en}}.json (the shell \
         substitutes the catalog label). If a new CJK literal is genuinely intended, update \
         CJK_BASELINE in this file *deliberately*; do not grow it silently.",
        added.len()
    );
    assert!(
        removed.is_empty(),
        "the CJK baseline records {} literal(s) that are no longer present: {removed:?}\n\
         Remove them from CJK_BASELINE deliberately (a stale baseline hides a real regression).",
        removed.len()
    );
}
