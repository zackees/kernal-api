#![feature(rustc_private)]

extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_middle;
extern crate rustc_span;

use rustc_errors::DiagDecorator;
use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::{DefId, LocalDefId, LOCAL_CRATE};
use rustc_hir::{AmbigArg, Expr, ExprKind, ImplItem, Item, ItemKind, TraitItem, Ty, TyKind};
use rustc_lint::{LateContext, LateLintPass, LintContext};
use rustc_middle::ty;
use rustc_span::{FileName, RemapPathScopeComponents, Span};
use std::collections::BTreeMap;
use std::path::Path;

dylint_linting::declare_late_lint! {
    /// ### What it does
    ///
    /// Rejects client references to implementation crates owned by
    /// `kernal-api`. Clients use facade-owned types and operations instead,
    /// which prevents dependency/version drift and backend vocabulary from
    /// escaping into application interfaces.
    ///
    /// Inside `kernal-api` itself the rule is the complementary one: a backend
    /// may be used privately, but naming one in a public type position --
    /// enum variant payload, public field, function parameter or return type,
    /// alias, or bound -- puts backend vocabulary back into the application
    /// interface just as surely as a `pub use` does.
    pub KERNAL_API_BOUNDARY,
    Deny,
    "require systems and async APIs owned by kernal-api to pass through its facades"
}

const OWNED_IMPLEMENTATION_CRATES: &[&str] = &[
    "addr2line",
    "blake3",
    "console_api",
    "console_subscriber",
    "crash_handler",
    "framehop",
    "globset",
    "interprocess",
    "jwalk",
    "memmap2",
    "mimalloc_pprof",
    "notify",
    "pdb_addr2line",
    "portable_pty",
    "reflink_copy",
    "running_process",
    "sysinfo",
    "tokio",
];

impl<'tcx> LateLintPass<'tcx> for KernalApiBoundary {
    fn check_crate(&mut self, cx: &LateContext<'tcx>) {
        if current_package_is_facade_owner() {
            return;
        }
        for dependency in direct_owned_dependencies() {
            cx.opt_span_lint(
                KERNAL_API_BOUNDARY,
                None::<Span>,
                DiagDecorator(move |diag| {
                    diag.primary_message(format!(
                        "direct dependency on `{dependency}` duplicates a kernal-api-owned implementation; depend on kernal-api and use its facade"
                    ));
                }),
            );
        }
    }

    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        if is_facade_owner(cx) || is_boundary_source(cx, expr.span) {
            return;
        }
        let def_id = match expr.kind {
            ExprKind::MethodCall(..) => cx.typeck_results().type_dependent_def_id(expr.hir_id),
            ExprKind::Path(qpath) => match cx.qpath_res(&qpath, expr.hir_id) {
                Res::Def(_, def_id) => Some(def_id),
                _ => None,
            },
            _ => None,
        };
        if let Some(def_id) = def_id {
            check_def(cx, expr.span, def_id);
        }
    }

    fn check_ty(&mut self, cx: &LateContext<'tcx>, ty: &'tcx Ty<'tcx, AmbigArg>) {
        if is_facade_owner(cx) || is_boundary_source(cx, ty.span) {
            return;
        }
        let TyKind::Path(qpath) = ty.kind else {
            return;
        };
        if let Res::Def(_, def_id) = cx.qpath_res(&qpath, ty.hir_id) {
            check_def(cx, ty.span, def_id);
        }
    }

    fn check_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx Item<'tcx>) {
        if is_facade_owner(cx) {
            check_public_signature(cx, item.owner_id.def_id);
            return;
        }
        if is_boundary_source(cx, item.span) {
            return;
        }
        let ItemKind::Use(path, _) = item.kind else {
            return;
        };
        for resolution in path.res.into_iter().flatten() {
            if let Res::Def(_, def_id) = resolution {
                check_def(cx, item.span, def_id);
            }
        }
    }

    fn check_impl_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx ImplItem<'tcx>) {
        if !is_facade_owner(cx) {
            return;
        }
        // A trait implementation restates a signature the trait already fixed,
        // so the private adapters that convert into a backend type are not the
        // facade choosing to speak backend vocabulary. Inherent methods are.
        if matches!(
            cx.tcx.def_kind(cx.tcx.local_parent(item.owner_id.def_id)),
            DefKind::Impl { of_trait: true }
        ) {
            return;
        }
        check_public_signature(cx, item.owner_id.def_id);
    }

    fn check_trait_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx TraitItem<'tcx>) {
        if is_facade_owner(cx) {
            check_public_signature(cx, item.owner_id.def_id);
        }
    }
}

fn current_package_is_facade_owner() -> bool {
    package_is_facade_owner(std::env::var("CARGO_PKG_NAME").ok().as_deref())
}

fn package_is_facade_owner(package_name: Option<&str>) -> bool {
    package_name == Some("kernal-api")
}

/// The facade owner is exempt from the client rule and subject to the public
/// signature rule instead. Cargo's package environment identifies it during an
/// ordinary `cargo dylint` run; the crate name identifies it in this lint's
/// own UI fixtures, where the package environment belongs to the lint.
fn is_facade_owner(cx: &LateContext<'_>) -> bool {
    current_package_is_facade_owner() || cx.tcx.crate_name(LOCAL_CRATE).as_str() == "kernal_api"
}

/// Reject every owned backend crate reachable from a public type position of
/// `def_id`. This is the half of the boundary that a `pub use` grep and a
/// client-side reference check both miss: a backend named in the payload of a
/// public enum variant, a public field, a signature, an alias, or a bound is
/// backend vocabulary a client must speak, whether or not it is re-exported.
fn check_public_signature(cx: &LateContext<'_>, def_id: LocalDefId) {
    if !cx.effective_visibilities.is_exported(def_id) {
        return;
    }
    let tcx = cx.tcx;
    let span = tcx.def_span(def_id);
    let mut leaks = OwnedTypeLeaks::default();
    let has_predicates = match tcx.def_kind(def_id) {
        DefKind::Struct | DefKind::Enum | DefKind::Union => {
            for variant in tcx.adt_def(def_id).variants() {
                for field in &variant.fields {
                    // Enum variant fields inherit the enum's visibility; a
                    // private or `pub(crate)` struct field names nothing a
                    // client can reach.
                    if !tcx.visibility(field.did).is_public() {
                        continue;
                    }
                    leaks.record_ty(
                        cx,
                        tcx.def_span(field.did),
                        tcx.type_of(field.did)
                            .instantiate_identity()
                            .skip_normalization(),
                    );
                }
            }
            true
        }
        DefKind::Fn | DefKind::AssocFn => {
            let signature = tcx.fn_sig(def_id).instantiate_identity();
            for input_or_output in signature.skip_binder().inputs_and_output {
                leaks.record_ty(cx, span, input_or_output);
            }
            true
        }
        DefKind::TyAlias
        | DefKind::Const { .. }
        | DefKind::AssocConst { .. }
        | DefKind::Static { .. } => {
            leaks.record_ty(
                cx,
                span,
                tcx.type_of(def_id)
                    .instantiate_identity()
                    .skip_normalization(),
            );
            true
        }
        DefKind::Trait => true,
        _ => false,
    };
    if has_predicates {
        for (clause, clause_span) in tcx.predicates_of(def_id).predicates {
            if let ty::ClauseKind::Trait(predicate) = clause.kind().skip_binder() {
                leaks.record_def(cx, *clause_span, predicate.trait_ref.def_id);
                for argument in predicate.trait_ref.args {
                    if let Some(argument) = argument.as_type() {
                        leaks.record_ty(cx, *clause_span, argument);
                    }
                }
            }
        }
    }
    leaks.report(cx);
}

/// One diagnostic per owned crate per item, at the first position that named
/// it, so a wide signature does not bury the boundary violation in repeats.
#[derive(Default)]
struct OwnedTypeLeaks {
    found: BTreeMap<String, Span>,
}

impl OwnedTypeLeaks {
    fn record_ty<'tcx>(&mut self, cx: &LateContext<'tcx>, span: Span, ty: ty::Ty<'tcx>) {
        for argument in ty.walk() {
            let Some(argument) = argument.as_type() else {
                continue;
            };
            let def_id = match argument.kind() {
                ty::Adt(definition, _) => definition.did(),
                ty::FnDef(def_id, _) | ty::Closure(def_id, _) | ty::Foreign(def_id) => *def_id,
                ty::Alias(alias) => alias.kind.def_id(),
                _ => continue,
            };
            self.record_def(cx, span, def_id);
        }
    }

    fn record_def(&mut self, cx: &LateContext<'_>, span: Span, def_id: DefId) {
        if let Some(crate_name) = owned_crate_name(cx, def_id) {
            self.found.entry(crate_name).or_insert(span);
        }
    }

    fn report(self, cx: &LateContext<'_>) {
        for (crate_name, span) in self.found {
            cx.opt_span_lint(
                KERNAL_API_BOUNDARY,
                Some(span),
                DiagDecorator(move |diag| {
                    diag.primary_message(format!(
                        "`{crate_name}` appears in a public kernal-api type position; a client that names this type speaks backend vocabulary -- expose a facade-owned type instead"
                    ));
                }),
            );
        }
    }
}

/// The owned implementation crate `def_id` belongs to, if any.
fn owned_crate_name(cx: &LateContext<'_>, def_id: DefId) -> Option<String> {
    if def_id.is_local() {
        return None;
    }
    let crate_name = cx.tcx.crate_name(def_id.krate).as_str().to_string();
    OWNED_IMPLEMENTATION_CRATES
        .contains(&crate_name.as_str())
        .then_some(crate_name)
}

/// Read the current client package manifest as well as its resolved HIR uses.
fn direct_owned_dependencies() -> Vec<String> {
    let Some(manifest_dir) = std::env::var_os("CARGO_MANIFEST_DIR") else {
        return Vec::new();
    };
    let Ok(source) = std::fs::read_to_string(Path::new(&manifest_dir).join("Cargo.toml")) else {
        return Vec::new();
    };
    direct_owned_dependencies_in(&source)
}

fn direct_owned_dependencies_in(source: &str) -> Vec<String> {
    let Ok(manifest) = toml::from_str::<toml::Value>(source) else {
        return Vec::new();
    };
    if manifest
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        == Some("kernal-api")
    {
        return Vec::new();
    }

    let mut found = Vec::new();
    collect_owned_dependency_tables(&manifest, &mut found);
    found.sort();
    found.dedup();
    found
}

fn collect_owned_dependency_tables(value: &toml::Value, found: &mut Vec<String>) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, value) in table {
        if matches!(
            key.as_str(),
            "dependencies" | "dev-dependencies" | "build-dependencies"
        ) {
            if let Some(dependencies) = value.as_table() {
                for (alias, declaration) in dependencies {
                    let package = declaration
                        .get("package")
                        .and_then(toml::Value::as_str)
                        .unwrap_or(alias);
                    let normalized = package.replace('-', "_");
                    if OWNED_IMPLEMENTATION_CRATES.contains(&normalized.as_str()) {
                        found.push(package.to_owned());
                    }
                }
            }
        } else {
            collect_owned_dependency_tables(value, found);
        }
    }
}

fn is_boundary_source(cx: &LateContext<'_>, span: Span) -> bool {
    let filename = match cx.sess().source_map().span_to_filename(span) {
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
    };
    filename.replace('\\', "/").contains("/kernal-api/src/")
}

fn check_def(cx: &LateContext<'_>, span: Span, def_id: DefId) {
    let Some(crate_name) = owned_crate_name(cx, def_id) else {
        return;
    };
    cx.opt_span_lint(
        KERNAL_API_BOUNDARY,
        Some(span),
        DiagDecorator(move |diag| {
            diag.primary_message(format!(
                "direct `{crate_name}` use bypasses the kernal-api facade; use a kernal_api-owned type or operation"
            ));
        }),
    );
}

#[test]
fn ui() {
    dylint_testing::ui_test(env!("CARGO_PKG_NAME"), "ui");
}

#[test]
fn the_owned_crate_set_is_sorted_and_unique() {
    assert!(OWNED_IMPLEMENTATION_CRATES
        .windows(2)
        .all(|pair| pair[0] < pair[1]));
}

#[test]
fn manifest_scan_finds_normal_aliased_target_and_test_dependencies() {
    let manifest = r#"
        [package]
        name = "client"

        [dependencies]
        tokio = "1"
        async_backend = { package = "console-subscriber", version = "0.5" }
        process_backend = { package = "running-process", version = "4" }

        [target.'cfg(windows)'.dev-dependencies]
        portable-pty = "0.9"
    "#;
    toml::from_str::<toml::Value>(manifest)
        .expect("the client manifest fixture must be valid TOML");
    assert_eq!(
        direct_owned_dependencies_in(manifest),
        [
            "console-subscriber",
            "portable-pty",
            "running-process",
            "tokio"
        ]
    );
}

#[test]
fn manifest_scan_exempts_the_facade_owner() {
    assert!(direct_owned_dependencies_in(
        r#"[package]
name = "kernal-api"
[dependencies]
tokio = "1"
"#
    )
    .is_empty());
}

#[test]
fn package_identity_exempts_only_the_facade_owner() {
    assert!(package_is_facade_owner(Some("kernal-api")));
    assert!(!package_is_facade_owner(Some("kernal_api_boundary")));
    assert!(!package_is_facade_owner(None));
}
