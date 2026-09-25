#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_errors;
extern crate rustc_span;

mod scan;

use std::path::{Path, PathBuf};

use rustc_errors::DiagDecorator;
use rustc_lint::{EarlyContext, EarlyLintPass, LintContext};
use rustc_span::{BytePos, FileName, RemapPathScopeComponents, Span};

dylint_linting::declare_pre_expansion_lint! {
    /// Rejects host selection and native OS APIs outside the one structured
    /// host selector and the private concrete platform trees it selects.
    ///
    /// Inspection parses every module's source before macro or `cfg`
    /// expansion and follows out-of-line modules whatever `cfg` guards them,
    /// so Windows- and macOS-only source is checked on a Linux host. The
    /// facade owner is not exempt: only the modules declared by the exactly
    /// shaped `cfg_select!` at the root of its library are native locations.
    pub KERNAL_API_PLATFORM_BOUNDARY,
    Deny,
    "keep host-platform selection inside kernal-api's private platform trees"
}

/// The package whose library root may hold the one host selector.
const FACADE_OWNER: &str = "kernal-api";

impl EarlyLintPass for KernalApiPlatformBoundary {
    fn check_crate(&mut self, cx: &EarlyContext<'_>, krate: &rustc_ast::ast::Crate) {
        let Some(root) = real_path(cx, krate.spans.inner_span) else {
            return;
        };
        let root = absolute(&root);
        let package = std::env::var("CARGO_PKG_NAME").ok();
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok().map(|dir| absolute(Path::new(&dir)));
        let owner_library = is_owner_library(
            package.as_deref(),
            manifest_dir.as_deref(),
            &root,
            cx.sess().opts.crate_types.iter().any(|ty| {
                matches!(ty, rustc_session::config::CrateType::Rlib | rustc_session::config::CrateType::Dylib | rustc_session::config::CrateType::Cdylib | rustc_session::config::CrateType::StaticLib)
            }) || cx.sess().is_test_crate(),
        );
        // Guest source compiled natively by one of the owner's tests keeps its
        // guest classification, which only the owner library's selector gives it.
        let known_guest = match (package.as_deref(), &manifest_dir) {
            (Some(FACADE_OWNER), Some(dir)) if !owner_library => {
                scan::scan_crate_with(&dir.join("src/lib.rs"), true, &Default::default(), &scan::read_file).1
            }
            _ => Default::default(),
        };
        let (violations, _) = scan::scan_crate_with(&root, owner_library, &known_guest, &scan::read_file);
        for violation in violations {
            let span = violation_span(cx, &violation).unwrap_or(krate.spans.inner_span);
            let detail = format!("{} `{}`", violation.kind, violation.construct);
            cx.opt_span_lint(
                KERNAL_API_PLATFORM_BOUNDARY,
                Some(span),
                DiagDecorator(move |diag| {
                    diag.primary_message(format!(
                        "host-platform selection outside the kernal-api boundary: {detail}; use a platform-neutral facade operation"
                    ));
                }),
            );
        }
    }
}

/// Only the owner's `src/lib.rs` library root (built as a library or as its
/// unit-test harness) may declare the host selector. Classification is by
/// path relative to the manifest, so the checkout location does not matter.
fn is_owner_library(package: Option<&str>, manifest_dir: Option<&Path>, root: &Path, library: bool) -> bool {
    let (Some(package), Some(manifest_dir)) = (package, manifest_dir) else {
        return false;
    };
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let manifest_dir = manifest_dir
        .canonicalize()
        .unwrap_or_else(|_| manifest_dir.to_path_buf());
    package == FACADE_OWNER && library && root.strip_prefix(&manifest_dir).is_ok_and(|rel| rel == Path::new("src/lib.rs"))
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn real_path(cx: &EarlyContext<'_>, span: Span) -> Option<PathBuf> {
    match cx.sess().source_map().span_to_filename(span) {
        FileName::Real(real) => real
            .local_path()
            .map(Path::to_path_buf)
            .or_else(|| Some(real.path(RemapPathScopeComponents::DIAGNOSTICS).to_path_buf())),
        _ => None,
    }
}

/// Map a scanner hit to a span, loading cfg-elided files into the source map
/// so the diagnostic points at the offending construct.
fn violation_span(cx: &EarlyContext<'_>, violation: &scan::Violation) -> Option<Span> {
    let file = cx.sess().source_map().load_file(&violation.file).ok()?;
    let len = (file.end_position() - file.start_pos).0 as usize;
    let start = violation.start.min(len);
    let end = violation.end.clamp(start, len);
    Some(Span::with_root_ctxt(
        file.start_pos + BytePos(start as u32),
        file.start_pos + BytePos(end as u32),
    ))
}

#[test]
fn ui() {
    dylint_testing::ui_test(env!("CARGO_PKG_NAME"), "ui");
}

#[test]
fn only_the_owner_library_root_may_select() {
    let dir = std::env::temp_dir();
    let root = dir.join("src/lib.rs");
    assert!(is_owner_library(Some("kernal-api"), Some(&dir), &root, true));
    assert!(!is_owner_library(Some("kernal-api"), Some(&dir), &root, false));
    assert!(!is_owner_library(Some("client"), Some(&dir), &root, true));
    assert!(!is_owner_library(Some("kernal-api"), Some(&dir), &dir.join("tests/x/main.rs"), true));
    assert!(!is_owner_library(None, Some(&dir), &root, true));
}

/// The repository acceptance gate: every Cargo target of every package in the
/// kernal-api workspace, discovered through `cargo metadata`, scans clean.
/// This needs no compilation of the product, so it catches a violation in a
/// module no host compiles (a Windows-only file on Linux, say) within seconds.
#[test]
fn repository_source_is_inside_the_boundary() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let output = std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["metadata", "--format-version", "1", "--no-deps", "--manifest-path"])
        .arg(&manifest)
        .output()
        .expect("run cargo metadata");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).expect("metadata json");
    let owner_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/lib.rs");
    let owner_root = std::path::absolute(owner_root).expect("absolute");
    let (_, known_guest) = scan::scan_crate_with(&owner_root, true, &Default::default(), &scan::read_file);
    assert!(!known_guest.is_empty(), "the owner library declares a guest tree");
    let mut violations = Vec::new();
    let mut roots = 0;
    for package in metadata["packages"].as_array().expect("packages") {
        let name = package["name"].as_str().unwrap_or_default();
        for target in package["targets"].as_array().expect("targets") {
            let root = PathBuf::from(target["src_path"].as_str().expect("src_path"));
            let library = target["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "lib"));
            let owner = name == FACADE_OWNER && library && root.ends_with("src/lib.rs");
            roots += 1;
            violations.extend(scan::scan_crate_with(&root, owner, &known_guest, &scan::read_file).0);
        }
    }
    violations.sort();
    violations.dedup();
    assert!(roots > 10, "discovered only {roots} target roots");
    let report: Vec<String> = violations
        .iter()
        .map(|v| format!("{}:{}:{}: {} `{}`", v.file.display(), v.line, v.column, v.kind, v.construct))
        .collect();
    assert!(report.is_empty(), "{} boundary violations:\n{}", report.len(), report.join("\n"));
}
