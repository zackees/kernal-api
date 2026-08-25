#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_errors;
extern crate rustc_span;

use std::collections::HashSet;

use rustc_errors::DiagDecorator;
use rustc_lint::{EarlyContext, EarlyLintPass, LintContext};
use rustc_span::{FileName, RemapPathScopeComponents, Span};

#[derive(Default)]
struct KernalApiPlatformBoundary {
    scanned_files: HashSet<String>,
}

dylint_linting::impl_pre_expansion_lint! {
    /// Rejects host selection and native OS APIs in client source.
    ///
    /// Inspection happens before macro/cfg expansion, so Windows-only code is
    /// still checked on Linux CI. The sole host selector and native APIs live
    /// in `kernal-api`'s private platform implementation trees.
    pub KERNAL_API_PLATFORM_BOUNDARY,
    Deny,
    "keep client host-platform selection inside kernal-api",
    KernalApiPlatformBoundary::default()
}

const SELECTORS: [&str; 10] = [
    "windows",
    "unix",
    "target_os",
    "target_family",
    "target_arch",
    "target_abi",
    "target_env",
    "target_vendor",
    "target_endian",
    "target_pointer_width",
];

impl EarlyLintPass for KernalApiPlatformBoundary {
    fn check_item(&mut self, cx: &EarlyContext<'_>, item: &rustc_ast::ast::Item) {
        if current_package_is_facade_owner() {
            return;
        }
        let current_file = source_filename(cx, item.span);
        if !in_scope(&current_file) || !self.scanned_files.insert(current_file.clone()) {
            return;
        }
        let source = std::fs::read_to_string(&current_file)
            .or_else(|_| cx.sess().source_map().span_to_snippet(item.span));
        if let Ok(source) = source {
            for invocation in platform_cfg_invocations(&source) {
                emit(cx, item.span, format!("host cfg `{invocation}`"));
            }
            for reference in native_platform_references(&source) {
                emit(cx, item.span, format!("native API `{reference}`"));
            }
            for reference in concrete_tree_references(&source) {
                emit(
                    cx,
                    item.span,
                    format!("private platform tree `{reference}`"),
                );
            }
        }
    }
}

fn current_package_is_facade_owner() -> bool {
    package_is_facade_owner(std::env::var("CARGO_PKG_NAME").ok().as_deref())
}

fn package_is_facade_owner(package_name: Option<&str>) -> bool {
    package_name == Some("kernal-api")
}

fn emit(cx: &EarlyContext<'_>, span: Span, detail: String) {
    cx.opt_span_lint(
        KERNAL_API_PLATFORM_BOUNDARY,
        Some(span),
        DiagDecorator(move |diag| {
            diag.primary_message(format!(
                "host-platform selection outside the kernal-api boundary: {detail}; use a platform-neutral kernal_api facade operation"
            ));
        }),
    );
}

fn in_scope(filename: &str) -> bool {
    let normalized = filename.replace('\\', "/");
    if normalized.ends_with("ui/allowed_boundary.rs") {
        return false;
    }
    if normalized.starts_with("ui/") || normalized.contains("/ui/") {
        return true;
    }
    if normalized.contains("/kernal-api/src/") {
        return false;
    }
    normalized.ends_with(".rs")
        && !normalized.contains("/target/")
        && !normalized.contains("/.cargo/registry/")
}

fn platform_cfg_invocations(source: &str) -> Vec<String> {
    let code = code_without_comments_or_strings(source);
    let compact: String = code
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    let mut invocations = Vec::new();
    for start in ["#[cfg(", "#[cfg_attr(", "#![cfg(", "#![cfg_attr(", "cfg!("] {
        for (offset, _) in compact.match_indices(start) {
            let Some(clause) = balanced_invocation(&compact, offset) else {
                continue;
            };
            if SELECTORS.iter().any(|selector| clause.contains(selector)) {
                invocations.push(clause.trim_start_matches("#[").to_owned());
            }
        }
    }
    invocations
}

fn concrete_tree_references(source: &str) -> Vec<String> {
    let code = code_without_comments_or_strings(source);
    let mut references = Vec::new();
    for name in [
        "platform_imp",
        "platform_win",
        "platform_linux",
        "platform_macos",
    ] {
        for (offset, _) in code.match_indices(name) {
            let before = code[..offset].chars().next_back();
            let after = code[offset + name.len()..].chars().next();
            let boundary = |value: Option<char>| {
                value.is_none_or(|value| !(value.is_alphanumeric() || value == '_'))
            };
            if boundary(before) && boundary(after) {
                references.push(name.to_owned());
            }
        }
    }
    references
}

fn native_platform_references(source: &str) -> Vec<String> {
    let code = code_without_comments_or_strings(source);
    [
        "std::os::windows",
        "std::os::unix",
        "std::os::linux",
        "std::os::macos",
        "windows_sys",
        "windows::Win32",
        "libc::",
        "tokio::net::windows",
        "tokio::net::Unix",
        "interprocess::os::windows",
        "interprocess::os::unix",
    ]
    .into_iter()
    .filter(|marker| code.contains(marker))
    .map(str::to_owned)
    .collect()
}

fn balanced_invocation(source: &str, offset: usize) -> Option<&str> {
    let mut depth = 0_u32;
    let mut saw_open = false;
    for (relative, character) in source[offset..].char_indices() {
        match character {
            '(' => {
                saw_open = true;
                depth += 1;
            }
            ')' if saw_open => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[offset..offset + relative + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

fn code_without_comments_or_strings(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut output = String::with_capacity(source.len());
    let mut index = 0;
    while index < bytes.len() {
        if let Some((prefix_len, hashes)) = raw_string_prefix(&bytes[index..]) {
            let start = index;
            index += prefix_len;
            while index < bytes.len() {
                let suffix = &bytes[index + 1..];
                if bytes[index] == b'"'
                    && suffix.len() >= hashes
                    && suffix[..hashes].iter().all(|byte| *byte == b'#')
                {
                    index += 1 + hashes;
                    break;
                }
                index += 1;
            }
            mask_range(&mut output, bytes, start, index);
        } else if bytes[index..].starts_with(b"//") {
            while index < bytes.len() && bytes[index] != b'\n' {
                output.push(' ');
                index += 1;
            }
        } else if bytes[index..].starts_with(b"/*") {
            let mut depth = 1_u32;
            output.push_str("  ");
            index += 2;
            while index < bytes.len() && depth > 0 {
                if bytes[index..].starts_with(b"/*") {
                    depth += 1;
                    output.push_str("  ");
                    index += 2;
                } else if bytes[index..].starts_with(b"*/") {
                    depth -= 1;
                    output.push_str("  ");
                    index += 2;
                } else {
                    output.push(if bytes[index] == b'\n' { '\n' } else { ' ' });
                    index += 1;
                }
            }
        } else if bytes[index] == b'"' {
            output.push(' ');
            index += 1;
            while index < bytes.len() {
                let byte = bytes[index];
                output.push(if byte == b'\n' { '\n' } else { ' ' });
                index += 1;
                if byte == b'\\' && index < bytes.len() {
                    output.push(' ');
                    index += 1;
                } else if byte == b'"' {
                    break;
                }
            }
        } else {
            output.push(bytes[index] as char);
            index += 1;
        }
    }
    output
}

fn raw_string_prefix(source: &[u8]) -> Option<(usize, usize)> {
    let mut index = usize::from(source.starts_with(b"br"));
    if source.get(index) != Some(&b'r') {
        return None;
    }
    index += 1;
    let hashes_start = index;
    while source.get(index) == Some(&b'#') {
        index += 1;
    }
    (source.get(index) == Some(&b'"')).then_some((index + 1, index - hashes_start))
}

fn mask_range(output: &mut String, source: &[u8], start: usize, end: usize) {
    for byte in &source[start..end] {
        output.push(if *byte == b'\n' { '\n' } else { ' ' });
    }
}

fn source_filename(cx: &EarlyContext<'_>, span: Span) -> String {
    match cx.sess().source_map().span_to_filename(span) {
        FileName::Real(real_filename) => real_filename
            .local_path()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                real_filename
                    .path(RemapPathScopeComponents::DIAGNOSTICS)
                    .to_string_lossy()
                    .into_owned()
            }),
        filename => filename
            .display(RemapPathScopeComponents::DIAGNOSTICS)
            .to_string(),
    }
}

#[test]
fn ui() {
    dylint_testing::ui_test(env!("CARGO_PKG_NAME"), "ui");
}

#[test]
fn source_detector_ignores_comments_strings_and_feature_cfgs() {
    assert!(
        platform_cfg_invocations(r####"let _ = "#[cfg(windows)]"; /* cfg!(unix) */"####).is_empty()
    );
    assert!(platform_cfg_invocations("#[cfg(feature = \"x\")] fn f() {}").is_empty());
    assert_eq!(
        platform_cfg_invocations("if cfg!(windows) {}"),
        ["cfg!(windows)"]
    );
}

#[test]
fn native_and_private_references_are_detected() {
    assert_eq!(
        native_platform_references("use std::os::unix::fs::PermissionsExt; libc::getpid();"),
        ["std::os::unix", "libc::"]
    );
    assert_eq!(
        concrete_tree_references("crate::platform_imp::x(); platform_win::y();"),
        ["platform_imp", "platform_win"]
    );
}

#[test]
fn all_client_target_kinds_are_in_scope() {
    for path in [
        "src/lib.rs",
        "tests/host.rs",
        "examples/host.rs",
        "benches/host.rs",
    ] {
        assert!(in_scope(path), "{path}");
    }
}

#[test]
fn package_identity_exempts_only_the_facade_owner() {
    assert!(package_is_facade_owner(Some("kernal-api")));
    assert!(!package_is_facade_owner(Some(
        "kernal_api_platform_boundary"
    )));
    assert!(!package_is_facade_owner(None));
}
