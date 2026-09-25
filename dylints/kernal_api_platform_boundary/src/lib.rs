#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_errors;
extern crate rustc_span;

mod baseline;
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
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
            .ok()
            .map(|dir| absolute(Path::new(&dir)));
        let owner_library = is_owner_library(
            package.as_deref(),
            manifest_dir.as_deref(),
            &root,
            cx.sess().opts.crate_types.iter().any(|ty| {
                matches!(
                    ty,
                    rustc_session::config::CrateType::Rlib
                        | rustc_session::config::CrateType::Dylib
                        | rustc_session::config::CrateType::Cdylib
                        | rustc_session::config::CrateType::StaticLib
                )
            }) || cx.sess().is_test_crate(),
        );
        // Guest source compiled natively by one of the owner's tests keeps its
        // guest classification, which only the owner library's selector gives it.
        let known_guest = match (package.as_deref(), &manifest_dir) {
            (Some(FACADE_OWNER), Some(dir)) if !owner_library => {
                scan::scan_crate_with(
                    &dir.join("src/lib.rs"),
                    true,
                    &Default::default(),
                    &scan::read_file,
                )
                .1
            }
            _ => Default::default(),
        };
        let (violations, _) =
            scan::scan_crate_with(&root, owner_library, &known_guest, &scan::read_file);
        // Existing debt recorded in the migration baseline is reported by the
        // repository gate, which also rejects stale entries; only hits beyond
        // it fail here. Outside this repository nothing is baselined.
        let baselined = baseline::parse(baseline::BASELINE).expect("valid baseline");
        let keyed = match baseline::repository_root_of(&root) {
            Some(repository) => baseline::keyed(&violations, &repository),
            None => violations.iter().map(|v| (v, Default::default())).collect(),
        };
        for (violation, key) in keyed {
            if baselined.contains(&key) {
                continue;
            }
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
fn is_owner_library(
    package: Option<&str>,
    manifest_dir: Option<&Path>,
    root: &Path,
    library: bool,
) -> bool {
    let (Some(package), Some(manifest_dir)) = (package, manifest_dir) else {
        return false;
    };
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let manifest_dir = manifest_dir
        .canonicalize()
        .unwrap_or_else(|_| manifest_dir.to_path_buf());
    package == FACADE_OWNER
        && library
        && root
            .strip_prefix(&manifest_dir)
            .is_ok_and(|rel| rel == Path::new("src/lib.rs"))
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn real_path(cx: &EarlyContext<'_>, span: Span) -> Option<PathBuf> {
    match cx.sess().source_map().span_to_filename(span) {
        FileName::Real(real) => real.local_path().map(Path::to_path_buf).or_else(|| {
            Some(
                real.path(RemapPathScopeComponents::DIAGNOSTICS)
                    .to_path_buf(),
            )
        }),
        _ => None,
    }
}

/// Map a scanner hit to a span, loading cfg-elided files into the source map
/// so the diagnostic points at the offending construct.
fn violation_span(cx: &EarlyContext<'_>, violation: &scan::Violation) -> Option<Span> {
    let source_map = cx.sess().source_map();
    // Reuse the compiler's copy of a file it already loaded, whatever path
    // spelling it was loaded under; a second copy under the absolute path
    // would make the diagnostic name a different file than the compiler does.
    let wanted = violation
        .file
        .canonicalize()
        .unwrap_or_else(|_| violation.file.clone());
    let loaded = source_map
        .files()
        .iter()
        .find(|file| match &file.name {
            FileName::Real(real) => real
                .local_path()
                .and_then(|path| path.canonicalize().ok())
                .is_some_and(|path| path == wanted),
            _ => false,
        })
        .cloned();
    let file = match loaded {
        Some(file) => file,
        // A cfg-elided file: spell it relative to the working directory, as
        // the compiler spells the files it was handed.
        None => {
            let relative = std::env::current_dir()
                .ok()
                .and_then(|cwd| cwd.canonicalize().ok())
                .and_then(|cwd| wanted.strip_prefix(cwd).ok().map(Path::to_path_buf));
            source_map
                .load_file(relative.as_deref().unwrap_or(&violation.file))
                .ok()?
        }
    };
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
    assert!(is_owner_library(
        Some("kernal-api"),
        Some(&dir),
        &root,
        true
    ));
    assert!(!is_owner_library(
        Some("kernal-api"),
        Some(&dir),
        &root,
        false
    ));
    assert!(!is_owner_library(Some("client"), Some(&dir), &root, true));
    assert!(!is_owner_library(
        Some("kernal-api"),
        Some(&dir),
        &dir.join("tests/x/main.rs"),
        true
    ));
    assert!(!is_owner_library(None, Some(&dir), &root, true));
}

/// Native packages whose every Cargo target must scan clean, relative to the
/// repository root: the facade owner's workspace, the client fixtures that
/// consume it, and the native build tools.
#[cfg(test)]
const NATIVE_MANIFESTS: [&str; 6] = [
    "Cargo.toml",
    "tests/build-resources-consumer/Cargo.toml",
    "tests/daemon-registration-consumer/Cargo.toml",
    "tests/daemon-registration-v2-consumer/Cargo.toml",
    "tools/wasm-abi-generator/Cargo.toml",
    "benchmarks/wasm-sketch/component-tools/Cargo.toml",
];

/// Wasm guest packages, generated guest ABI bindings included. They never
/// build for a native host, so they are scanned under the guest rule: guest
/// predicates and wasm imports are allowed, native host selection is not.
/// Their files keep that classification when a native tool `include!`s them.
#[cfg(test)]
const GUEST_MANIFESTS: [&str; 5] = [
    "src/wasm/generated/v1/guest/Cargo.toml",
    "guests/threaded-smoke/Cargo.toml",
    "examples/wasm-tauri-screenshot/guest/Cargo.toml",
    "benchmarks/wasm-sketch/compiler-guest/Cargo.toml",
    "benchmarks/wasm-sketch/component-guest/Cargo.toml",
];

/// The lint crates themselves: their UI fixtures violate the boundary on
/// purpose. The list is exact; an unclassified manifest fails
/// `every_manifest_is_classified`.
#[cfg(test)]
const LINT_MANIFESTS: [&str; 2] = [
    "dylints/kernal_api_boundary/Cargo.toml",
    "dylints/kernal_api_platform_boundary/Cargo.toml",
];

#[cfg(test)]
fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

/// `(package name, target root, is library)` for every target of a manifest.
#[cfg(test)]
fn manifest_targets(manifest: &Path) -> Vec<(String, PathBuf, bool)> {
    let output =
        std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
            .args([
                "metadata",
                "--format-version",
                "1",
                "--no-deps",
                "--offline",
                "--manifest-path",
            ])
            .arg(manifest)
            .output()
            .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "{}: {}",
        manifest.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("metadata json");
    let mut targets = Vec::new();
    for package in metadata["packages"].as_array().expect("packages") {
        let name = package["name"].as_str().unwrap_or_default().to_owned();
        for target in package["targets"].as_array().expect("targets") {
            let root = PathBuf::from(target["src_path"].as_str().expect("src_path"));
            let library = target["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "lib"));
            targets.push((name.clone(), root, library));
        }
    }
    targets
}

#[test]
fn every_manifest_is_classified() {
    let output = std::process::Command::new("git")
        .args(["ls-files", "*Cargo.toml"])
        .current_dir(repository_root())
        .output()
        .expect("run git ls-files");
    assert!(output.status.success());
    let mut tracked: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    tracked.sort();
    let mut classified: Vec<String> = NATIVE_MANIFESTS
        .iter()
        .chain(&GUEST_MANIFESTS)
        .chain(&LINT_MANIFESTS)
        .map(|path| (*path).to_owned())
        .collect();
    classified.sort();
    assert_eq!(
        tracked, classified,
        "classify each manifest as native, guest or lint"
    );
}

/// The repository acceptance gate: every Cargo target of every native and
/// guest package, discovered through `cargo metadata`, scans clean. It needs
/// no product compilation, so a violation in a module no host compiles (a
/// Windows-only file on Linux, say) fails within seconds.
#[test]
fn repository_source_is_inside_the_boundary() {
    let repository = repository_root();
    let read = &scan::read_file;
    let mut violations = Vec::new();
    let mut roots = 0;

    // The owner library's root guest arm and the guest packages define the
    // guest files; everything that reaches them later keeps that class.
    let (owner_violations, mut known_guest) = scan::scan_crate_with(
        &repository.join("src/lib.rs"),
        true,
        &Default::default(),
        read,
    );
    assert!(
        !known_guest.is_empty(),
        "the owner library declares a guest tree"
    );
    violations.extend(owner_violations);
    for manifest in GUEST_MANIFESTS {
        for (_, root, _) in manifest_targets(&repository.join(manifest)) {
            roots += 1;
            let guest_root = std::iter::once(root.clone()).collect();
            let (found, files) = scan::scan_crate_with(&root, false, &guest_root, read);
            violations.extend(found);
            known_guest.extend(files);
        }
    }
    for manifest in NATIVE_MANIFESTS {
        for (name, root, library) in manifest_targets(&repository.join(manifest)) {
            let owner = is_owner_library(Some(&name), Some(&repository), &root, library);
            roots += 1;
            violations.extend(scan::scan_crate_with(&root, owner, &known_guest, read).0);
        }
    }
    violations.sort();
    violations.dedup();
    assert!(roots > 10, "discovered only {roots} target roots");
    // Until the migration finishes, the debt must match the exact baseline:
    // no new hits, no stale entries.
    let keys: Vec<baseline::Key> = baseline::keyed(&violations, &repository)
        .into_iter()
        .map(|(_, key)| key)
        .collect();
    if let Err(error) = baseline::check(&keys, baseline::BASELINE) {
        panic!("platform boundary baseline mismatch:\n{error}");
    }
}
