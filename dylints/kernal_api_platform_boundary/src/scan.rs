//! Pre-expansion source scanner for the platform boundary.
//!
//! The scanner works on token trees parsed straight from source files, not on
//! the compiler's expanded AST, so comments and string literals never match
//! and `cfg`-elided code is still inspected. Module discovery follows every
//! out-of-line `mod name;` (honouring `#[path]`) whatever `cfg` guards it, so
//! a Linux lint pass reads the Windows and macOS source too.
//!
//! Location is decided structurally. The only approved native locations are
//! the modules declared by the one host selector at the root of the facade
//! owner's library; everything reached through them is a concrete tree. All
//! other source -- the rest of the owner, its tests, binaries and examples,
//! and every client crate -- is neutral.

use std::collections::HashSet;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

use proc_macro2::{Delimiter, Group, Spacing, TokenStream, TokenTree};

/// `cfg` predicates that select a native host (or a host property).
pub const SELECTORS: [&str; 10] = [
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

/// Raw host binding crates. Naming one as a path root is a native API use.
const NATIVE_CRATES: [&str; 18] = [
    "libc",
    "mach2",
    "winapi",
    "windows",
    "windows_sys",
    "windows_core",
    "x11rb",
    "gtk",
    "gdk",
    "glib",
    "webkit2gtk",
    "objc2",
    "block2",
    "core_foundation",
    "core_graphics",
    "nix",
    "rustix",
    "dispatch2",
];

/// Guest-target predicates, valid only inside the guest tree.
const GUEST_PREDICATES: [(&str, &str); 4] = [
    ("target_family", "wasm"),
    ("target_arch", "wasm32"),
    ("target_os", "wasi"),
    ("target_os", "unknown"),
];

/// Names of the private concrete platform trees and their selected alias.
const CONCRETE_NAMES: [&str; 4] = [
    "platform_imp",
    "platform_win",
    "platform_linux",
    "platform_macos",
];

/// The exact arms of the one host selector, keyed by `target_os` value.
const HOST_ARMS: [(&str, &str); 3] = [
    ("windows", "platform_win"),
    ("linux", "platform_linux"),
    ("macos", "platform_macos"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    HostCfg,
    NativeApi,
    ConcreteTree,
    Selector,
    Unreadable,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Kind::HostCfg => "host cfg",
            Kind::NativeApi => "native API",
            Kind::ConcreteTree => "private platform tree",
            Kind::Selector => "host selector",
            Kind::Unreadable => "unparsed module source",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Violation {
    pub file: PathBuf,
    /// Byte range within `file`.
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub column: usize,
    pub kind: Kind,
    pub construct: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Zone {
    /// The owner's library root: may hold the one host selector and the
    /// crate-root `use platform_imp::...` bridge.
    OwnerRoot,
    Neutral,
    Concrete,
}

#[derive(Clone)]
struct Ctx {
    file: PathBuf,
    zone: Zone,
    /// Directory in which `mod child;` files are resolved.
    mod_dir: PathBuf,
    /// Directory against which a `#[path]` on an out-of-line module resolves.
    path_dir: PathBuf,
    /// True while every enclosing group is a `cfg_select!` arm body at the
    /// top of the root file.
    root_level: bool,
    /// Path prefix for a braced `use` group, such as `std::` in
    /// `use std::{os::unix, io};`.
    prefix: Vec<String>,
    /// Inside the wasm guest tree declared by the root guest selector, where
    /// guest-target predicates (not native host ones) are allowed.
    guest: bool,
}

pub struct Scanner<'a> {
    read: &'a dyn Fn(&Path) -> Option<String>,
    visited: HashSet<PathBuf>,
    violations: Vec<Violation>,
    known_guest: HashSet<PathBuf>,
    guest_files: HashSet<PathBuf>,
    host_selector_seen: bool,
    guest_selector_seen: bool,
}

/// Scan a crate from its root file. `owner_library` is true only for the
/// facade owner's library target: the one place a host selector is valid.
#[cfg(test)]
pub fn scan_crate(
    root: &Path,
    owner_library: bool,
    read: &dyn Fn(&Path) -> Option<String>,
) -> Vec<Violation> {
    scan_crate_with(root, owner_library, &HashSet::new(), read).0
}

/// As [`scan_crate`], treating `known_guest` files as guest source however
/// they are reached (a native test may compile the guest facade by `#[path]`).
/// Also returns the guest files this scan discovered.
pub fn scan_crate_with(
    root: &Path,
    owner_library: bool,
    known_guest: &HashSet<PathBuf>,
    read: &dyn Fn(&Path) -> Option<String>,
) -> (Vec<Violation>, HashSet<PathBuf>) {
    let mut scanner = Scanner {
        read,
        visited: HashSet::new(),
        violations: Vec::new(),
        known_guest: known_guest.clone(),
        guest_files: HashSet::new(),
        host_selector_seen: false,
        guest_selector_seen: false,
    };
    let dir = root.parent().map(Path::to_path_buf).unwrap_or_default();
    let ctx = Ctx {
        file: root.to_path_buf(),
        zone: if owner_library {
            Zone::OwnerRoot
        } else {
            Zone::Neutral
        },
        mod_dir: dir.clone(),
        path_dir: dir,
        root_level: owner_library,
        prefix: Vec::new(),
        guest: false,
    };
    scanner.scan_file(ctx, None);
    let mut violations = scanner.violations;
    violations.sort();
    violations.dedup();
    (violations, scanner.guest_files)
}

/// Read a file from disk; the production reader.
pub fn read_file(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn is_punct(tree: Option<&TokenTree>, ch: char) -> bool {
    matches!(tree, Some(TokenTree::Punct(p)) if p.as_char() == ch)
}

fn ident_is(tree: Option<&TokenTree>, name: &str) -> bool {
    matches!(tree, Some(TokenTree::Ident(i)) if i == name)
}

fn ident_name(tree: Option<&TokenTree>) -> Option<String> {
    match tree {
        Some(TokenTree::Ident(i)) => Some(i.to_string().trim_start_matches("r#").to_owned()),
        _ => None,
    }
}

/// `::` at index `i` (two joint colons).
fn is_path_sep(trees: &[TokenTree], i: usize) -> bool {
    matches!(trees.get(i), Some(TokenTree::Punct(p)) if p.as_char() == ':' && p.spacing() == Spacing::Joint)
        && is_punct(trees.get(i + 1), ':')
}

fn compact(stream: &TokenStream) -> String {
    stream
        .to_string()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// True if a `cfg` predicate token stream names a host selector.
fn names_host_selector(stream: &TokenStream, guest: bool) -> bool {
    let trees: Vec<TokenTree> = stream.clone().into_iter().collect();
    let mut i = 0;
    while i < trees.len() {
        match &trees[i] {
            TokenTree::Ident(ident) if SELECTORS.iter().any(|s| ident == s) => {
                let guest_value = is_punct(trees.get(i + 1), '=')
                    && string_literal(trees.get(i + 2)).is_some_and(|value| {
                        GUEST_PREDICATES.contains(&(ident.to_string().as_str(), value.as_str()))
                    });
                if !(guest && guest_value) {
                    return true;
                }
                i += 3;
                continue;
            }
            TokenTree::Group(group) if names_host_selector(&group.stream(), guest) => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

fn string_literal(tree: Option<&TokenTree>) -> Option<String> {
    match tree {
        Some(TokenTree::Literal(lit)) => {
            let text = lit.to_string();
            let inner = text.strip_prefix('"')?.strip_suffix('"')?;
            Some(inner.replace("\\\\", "\\"))
        }
        _ => None,
    }
}

impl Scanner<'_> {
    fn push(&mut self, ctx: &Ctx, span: proc_macro2::Span, kind: Kind, construct: String) {
        if ctx.zone == Zone::Concrete {
            return;
        }
        let start = span.start();
        let range = span.byte_range();
        self.violations.push(Violation {
            file: ctx.file.clone(),
            start: range.start,
            end: range.end,
            line: start.line,
            column: start.column + 1,
            kind,
            construct,
        });
    }

    fn scan_file(&mut self, ctx: Ctx, declared_at: Option<(&Ctx, proc_macro2::Span)>) {
        let file = normalize(&ctx.file);
        if !self.visited.insert(file.clone()) {
            return;
        }
        let Some(source) = (self.read)(&file) else {
            if let Some((parent, span)) = declared_at {
                if ctx.zone != Zone::Concrete {
                    self.push(
                        parent,
                        span,
                        Kind::Unreadable,
                        format!("{} (missing)", file.display()),
                    );
                }
            }
            return;
        };
        let source = source.strip_prefix('\u{feff}').unwrap_or(&source);
        let stream = match TokenStream::from_str(source) {
            Ok(stream) => stream,
            Err(error) => {
                if ctx.zone != Zone::Concrete {
                    if let Some((parent, span)) = declared_at {
                        self.push(
                            parent,
                            span,
                            Kind::Unreadable,
                            format!("{} ({error})", file.display()),
                        );
                    }
                }
                return;
            }
        };
        let guest = ctx.guest || self.known_guest.contains(&file);
        if guest {
            self.guest_files.insert(file.clone());
        }
        let ctx = Ctx { file, guest, ..ctx };
        self.walk(&stream, &ctx);
    }

    fn walk(&mut self, stream: &TokenStream, ctx: &Ctx) {
        let trees: Vec<TokenTree> = stream.clone().into_iter().collect();
        let mut path_attr: Option<String> = None;
        let mut i = 0;
        while i < trees.len() {
            let tree = &trees[i];
            // Attributes: `#[...]` and `#![...]`.
            if is_punct(Some(tree), '#') {
                let bang = usize::from(is_punct(trees.get(i + 1), '!'));
                if let Some(TokenTree::Group(group)) = trees.get(i + 1 + bang) {
                    if group.delimiter() == Delimiter::Bracket {
                        self.check_attribute(group, ctx);
                        if let Some(path) = path_attribute(&group.stream()) {
                            path_attr = Some(path);
                        }
                        i += 2 + bang;
                        continue;
                    }
                }
            }
            match tree {
                TokenTree::Ident(ident) => {
                    let name = ident.to_string();
                    // `cfg!(...)`
                    if name == "cfg" && is_punct(trees.get(i + 1), '!') {
                        if let Some(TokenTree::Group(group)) = trees.get(i + 2) {
                            if names_host_selector(&group.stream(), ctx.guest) {
                                self.push(
                                    ctx,
                                    ident.span(),
                                    Kind::HostCfg,
                                    format!("cfg!{}", compact(&TokenStream::from(TokenTree::Group(group.clone())))),
                                );
                            }
                            i += 3;
                            continue;
                        }
                    }
                    // `cfg_select! { ... }`
                    if name == "cfg_select" && is_punct(trees.get(i + 1), '!') {
                        if let Some(TokenTree::Group(group)) = trees.get(i + 2) {
                            self.cfg_select(ident.span(), group, ctx);
                            i += 3;
                            continue;
                        }
                    }
                    // `mod name;` / `mod name { ... }`
                    if name == "mod" {
                        if let Some(child) = ident_name(trees.get(i + 1)) {
                            if is_punct(trees.get(i + 2), ';') {
                                self.out_of_line_module(ctx, &child, path_attr.take(), ident.span(), ctx.zone);
                                i += 3;
                                continue;
                            }
                            if let Some(TokenTree::Group(body)) = trees.get(i + 2) {
                                if body.delimiter() == Delimiter::Brace {
                                    let dir = match path_attr.take() {
                                        Some(path) => ctx.mod_dir.join(path),
                                        None => ctx.mod_dir.join(&child),
                                    };
                                    let inner = Ctx {
                                        mod_dir: dir.clone(),
                                        path_dir: dir,
                                        root_level: false,
                                        prefix: Vec::new(),
                                        zone: if ctx.zone == Zone::OwnerRoot {
                                            Zone::Neutral
                                        } else {
                                            ctx.zone
                                        },
                                        ..ctx.clone()
                                    };
                                    self.walk(&body.stream(), &inner);
                                    i += 3;
                                    continue;
                                }
                            }
                        }
                    }
                    // A foreign import block, `extern "ABI" { .. }`, declares native
                    // symbols directly. (`extern "C" fn` callbacks are definitions.)
                    if name == "extern" && !ctx.guest {
                        let abi = usize::from(matches!(trees.get(i + 1), Some(TokenTree::Literal(_))));
                        if let Some(TokenTree::Group(body)) = trees.get(i + 1 + abi) {
                            if body.delimiter() == Delimiter::Brace {
                                self.push(ctx, ident.span(), Kind::NativeApi, "extern block".to_owned());
                            }
                        }
                    }
                    // `include!("file.rs")`
                    if name == "include" && is_punct(trees.get(i + 1), '!') {
                        if let Some(TokenTree::Group(group)) = trees.get(i + 2) {
                            let args: Vec<TokenTree> = group.stream().into_iter().collect();
                            if let Some(path) = string_literal(args.first()) {
                                if args.len() == 1 && path.ends_with(".rs") {
                                    let dir = ctx.file.parent().map(Path::to_path_buf).unwrap_or_default();
                                    let child = Ctx {
                                        file: dir.join(&path),
                                        root_level: false,
                                        prefix: Vec::new(),
                                        zone: if ctx.zone == Zone::OwnerRoot { Zone::Neutral } else { ctx.zone },
                                        ..ctx.clone()
                                    };
                                    let span = ident.span();
                                    self.scan_file(child, Some((ctx, span)));
                                }
                            }
                            i += 3;
                            continue;
                        }
                    }
                    // A segment after `a::` belongs to the path that started at
                    // `a`; a leading `::` starts an absolute path here.
                    let continues_path = i >= 2
                        && is_path_sep(&trees, i - 2)
                        && (matches!(trees.get(i.wrapping_sub(3)), Some(TokenTree::Ident(_)))
                            || is_punct(trees.get(i.wrapping_sub(3)), '>'));
                    if !continues_path {
                        if name != "pub" {
                            path_attr = None;
                        }
                        i = self.path(&trees, i, ctx);
                        continue;
                    }
                }
                TokenTree::Group(group) => {
                    let prefix = if group.delimiter() == Delimiter::Brace && i >= 2 && is_path_sep(&trees, i - 2) {
                        path_prefix_before(&trees, i - 2, &ctx.prefix)
                    } else {
                        Vec::new()
                    };
                    let inner = Ctx {
                        root_level: false,
                        prefix,
                        zone: if ctx.zone == Zone::OwnerRoot { Zone::Neutral } else { ctx.zone },
                        ..ctx.clone()
                    };
                    self.walk(&group.stream(), &inner);
                }
                _ => {}
            }
            if !matches!(tree, TokenTree::Punct(p) if p.as_char() == '#')
                && !ident_is(Some(tree), "pub")
                && !matches!(tree, TokenTree::Group(g) if g.delimiter() == Delimiter::Parenthesis)
            {
                path_attr = None;
            }
            i += 1;
        }
    }

    /// Inspect the path starting at `trees[i]` (an identifier not preceded by
    /// `::`). Returns the next index to visit.
    fn path(&mut self, trees: &[TokenTree], i: usize, ctx: &Ctx) -> usize {
        let mut segments: Vec<(String, proc_macro2::Span)> = Vec::new();
        let mut j = i;
        while let Some(TokenTree::Ident(ident)) = trees.get(j) {
            segments.push((ident.to_string().trim_start_matches("r#").to_owned(), ident.span()));
            if is_path_sep(trees, j + 1) && matches!(trees.get(j + 3), Some(TokenTree::Ident(_))) {
                j += 3;
            } else {
                break;
            }
        }
        let names: Vec<String> = ctx
            .prefix
            .iter()
            .cloned()
            .chain(segments.iter().map(|(name, _)| name.clone()))
            .collect();
        let continues = is_path_sep(trees, j + 1);
        let prev = i.checked_sub(1).and_then(|p| trees.get(p));
        let after_use = ident_is(prev, "use") || ident_is(prev, "crate");
        let first_span = segments[0].1;

        // Concrete tree names.
        for (name, span) in &segments {
            if CONCRETE_NAMES.contains(&name.as_str()) && !self.bridge_allowed(ctx, prev, name, &segments) {
                self.push(ctx, *span, Kind::ConcreteTree, name.clone());
            }
        }

        // A single identifier that is not used as a path is an ordinary name
        // (`let windows = ...`, `fn libc_like()`), unless imported.
        let is_path = segments.len() > 1 || continues || after_use || !ctx.prefix.is_empty();
        if is_path {
            if let Some(native) = native_path(&names) {
                self.push(ctx, first_span, Kind::NativeApi, native);
            }
        }
        // Later segments are revisited as path continuations and skipped, but
        // a trailing `cfg_select!` or `cfg!` still reaches its own handler.
        i + 1
    }

    /// `platform_imp` may be named only by the crate-root re-export bridge of
    /// the owner library: `use platform_imp::...` at the root level.
    fn bridge_allowed(&self, ctx: &Ctx, prev: Option<&TokenTree>, name: &str, segments: &[(String, proc_macro2::Span)]) -> bool {
        ctx.zone == Zone::OwnerRoot
            && ctx.root_level
            && name == "platform_imp"
            && segments.first().map(|(n, _)| n.as_str()) == Some("platform_imp")
            && ident_is(prev, "use")
    }

    fn check_attribute(&mut self, group: &Group, ctx: &Ctx) {
        let trees: Vec<TokenTree> = group.stream().into_iter().collect();
        if let Some(TokenTree::Ident(ident)) = trees.first() {
            if ident == "link" && !ctx.guest {
                self.push(ctx, ident.span(), Kind::NativeApi, "#[link]".to_owned());
            }
        }
        self.check_attribute_trees(&trees, ctx, group);
    }

    fn check_attribute_trees(&mut self, trees: &[TokenTree], ctx: &Ctx, whole: &Group) {
        for (i, tree) in trees.iter().enumerate() {
            match tree {
                TokenTree::Ident(ident) if ident == "cfg" || ident == "cfg_attr" => {
                    if let Some(TokenTree::Group(args)) = trees.get(i + 1) {
                        if args.delimiter() == Delimiter::Parenthesis && names_host_selector(&args.stream(), ctx.guest) {
                            self.push(
                                ctx,
                                ident.span(),
                                Kind::HostCfg,
                                compact(&whole.stream()),
                            );
                            return;
                        }
                    }
                }
                TokenTree::Group(inner) => {
                    let inner_trees: Vec<TokenTree> = inner.stream().into_iter().collect();
                    let before = self.violations.len();
                    self.check_attribute_trees(&inner_trees, ctx, whole);
                    if self.violations.len() != before {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    fn out_of_line_module(&mut self, ctx: &Ctx, child: &str, path: Option<String>, span: proc_macro2::Span, zone: Zone) {
        let candidates = match path {
            Some(path) => vec![ctx.path_dir.join(path)],
            None => vec![
                ctx.mod_dir.join(format!("{child}.rs")),
                ctx.mod_dir.join(child).join("mod.rs"),
            ],
        };
        let file = candidates
            .iter()
            .find(|candidate| (self.read)(&normalize(candidate)).is_some())
            .cloned()
            .unwrap_or_else(|| candidates[0].clone());
        let is_mod_rs = file
            .file_name()
            .is_some_and(|name| name == "mod.rs");
        let parent = file.parent().map(Path::to_path_buf).unwrap_or_default();
        let mod_dir = if is_mod_rs {
            parent.clone()
        } else {
            parent.join(file.file_stem().unwrap_or_default())
        };
        let child_ctx = Ctx {
            file,
            zone: if zone == Zone::OwnerRoot { Zone::Neutral } else { zone },
            mod_dir,
            path_dir: parent,
            root_level: false,
            prefix: Vec::new(),
            guest: ctx.guest,
        };
        self.scan_file(child_ctx, Some((ctx, span)));
    }

    fn cfg_select(&mut self, span: proc_macro2::Span, group: &Group, ctx: &Ctx) {
        let arms = split_arms(&group.stream());
        if ctx.zone == Zone::OwnerRoot && ctx.root_level {
            if !self.guest_selector_seen && is_guest_selector(&arms) {
                self.guest_selector_seen = true;
                // Both arms stay at the root level: the first holds the guest
                // ABI tree, the fallback the native facade root.
                for (index, (_, body)) in arms.iter().enumerate() {
                    let arm = Ctx { guest: index == 0, ..ctx.clone() };
                    self.walk(&body.stream(), &arm);
                }
                return;
            }
            if !self.host_selector_seen {
                if let Some(modules) = host_selector_modules(&arms) {
                    self.host_selector_seen = true;
                    for (module, path) in modules {
                        self.out_of_line_module(ctx, &module, path, span, Zone::Concrete);
                    }
                    return;
                }
            }
        }
        let mut reported = false;
        for (predicate, body) in &arms {
            if !reported && names_host_selector(predicate, ctx.guest) {
                reported = true;
                self.push(
                    ctx,
                    span,
                    if ctx.zone == Zone::OwnerRoot { Kind::Selector } else { Kind::HostCfg },
                    format!("cfg_select!{{{}=>..}}", compact(predicate)),
                );
            }
            let inner = Ctx {
                root_level: false,
                zone: if ctx.zone == Zone::OwnerRoot { Zone::Neutral } else { ctx.zone },
                ..ctx.clone()
            };
            self.walk(&body.stream(), &inner);
        }
    }
}

/// The value of `path = "..."` in an attribute body.
fn path_attribute(stream: &TokenStream) -> Option<String> {
    let trees: Vec<TokenTree> = stream.clone().into_iter().collect();
    if trees.len() == 3 && ident_is(trees.first(), "path") && is_punct(trees.get(1), '=') {
        return string_literal(trees.get(2));
    }
    None
}

/// Collect the path before `::{` ending at `sep` (the index of the first `:`).
fn path_prefix_before(trees: &[TokenTree], sep: usize, outer: &[String]) -> Vec<String> {
    let mut segments = Vec::new();
    let mut k = sep;
    while k >= 1 {
        match trees.get(k - 1) {
            Some(TokenTree::Ident(ident)) => {
                segments.push(ident.to_string());
                if k >= 3 && is_path_sep(trees, k - 3) {
                    k -= 3;
                } else {
                    break;
                }
            }
            _ => break,
        }
    }
    segments.reverse();
    outer.iter().cloned().chain(segments).collect()
}

fn native_path(names: &[String]) -> Option<String> {
    let first = names.first()?.as_str();
    let second = names.get(1).map(String::as_str);
    let third = names.get(2).map(String::as_str);
    if NATIVE_CRATES.contains(&first) || first.starts_with("objc2_") {
        return Some(first.to_owned());
    }
    if (first == "std" || first == "core") && second == Some("os") {
        return Some(format!("{first}::os"));
    }
    if first == "tokio" && matches!(second, Some("net" | "signal" | "process")) {
        if let Some(third) = third {
            if third == "unix" || third == "windows" || third.starts_with("Unix") {
                return Some(format!("tokio::{}::{third}", second.unwrap_or_default()));
            }
        }
    }
    if first == "interprocess" && second == Some("os") {
        return Some("interprocess::os".to_owned());
    }
    None
}

/// Split `cfg_select!` arms into `(predicate, body)` pairs.
fn split_arms(stream: &TokenStream) -> Vec<(TokenStream, Group)> {
    let mut arms = Vec::new();
    let mut predicate = Vec::new();
    let trees: Vec<TokenTree> = stream.clone().into_iter().collect();
    let mut i = 0;
    while i < trees.len() {
        if is_punct(trees.get(i), '=') && is_punct(trees.get(i + 1), '>') {
            if let Some(TokenTree::Group(body)) = trees.get(i + 2) {
                arms.push((predicate.drain(..).collect(), body.clone()));
                i += 3;
                if is_punct(trees.get(i), ',') {
                    i += 1;
                }
                continue;
            }
        }
        predicate.push(trees[i].clone());
        i += 1;
    }
    arms
}

fn is_guest_selector(arms: &[(TokenStream, Group)]) -> bool {
    arms.len() == 2
        && compact(&arms[0].0) == "target_family=\"wasm\""
        && compact(&arms[1].0) == "_"
}

/// The concrete modules declared by an exactly shaped host selector:
/// `target_os = "windows" | "linux" | "macos"`, no fallback, each arm body
/// `mod platform_x; pub(crate) use platform_x as platform_imp;` (an optional
/// `#[path]` on the module is allowed).
fn host_selector_modules(arms: &[(TokenStream, Group)]) -> Option<Vec<(String, Option<String>)>> {
    if arms.len() != HOST_ARMS.len() {
        return None;
    }
    let mut modules = Vec::new();
    let mut seen = HashSet::new();
    for (predicate, body) in arms {
        let predicate = compact(predicate);
        let (os, module) = HOST_ARMS
            .iter()
            .find(|(os, _)| predicate == format!("target_os=\"{os}\""))?;
        if !seen.insert(*os) {
            return None;
        }
        let trees: Vec<TokenTree> = body.stream().into_iter().collect();
        let mut k = 0;
        let mut path = None;
        if is_punct(trees.first(), '#') {
            if let Some(TokenTree::Group(attr)) = trees.get(1) {
                path = Some(path_attribute(&attr.stream())?);
                k = 2;
            }
        }
        let rest = compact(&trees[k..].iter().cloned().collect());
        if rest != format!("mod{module};pub(crate)use{module}asplatform_imp;") {
            return None;
        }
        modules.push(((*module).to_owned(), path));
    }
    Some(modules)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const SELECTOR: &str = r#"
        std::cfg_select! {
            target_os = "windows" => { mod platform_win; pub(crate) use platform_win as platform_imp; }
            target_os = "linux" => { mod platform_linux; pub(crate) use platform_linux as platform_imp; }
            target_os = "macos" => { mod platform_macos; pub(crate) use platform_macos as platform_imp; }
        }
        pub use platform_imp::thing;
    "#;

    fn scan(files: &[(&str, &str)], owner: bool) -> Vec<(String, Kind, String)> {
        let map: HashMap<PathBuf, String> = files
            .iter()
            .map(|(path, text)| (PathBuf::from(path), (*text).to_owned()))
            .collect();
        let read = move |path: &Path| map.get(path).cloned();
        scan_crate(Path::new(files[0].0), owner, &read)
            .into_iter()
            .map(|v| (v.file.display().to_string(), v.kind, v.construct))
            .collect()
    }

    fn owner(extra: &[(&str, &str)]) -> Vec<(String, Kind, String)> {
        let mut files = vec![
            ("src/lib.rs", SELECTOR),
            ("src/platform_win.rs", "mod a; use windows_sys::X; #[cfg(target_arch = \"x86_64\")] fn f() {}"),
            ("src/platform_win/a.rs", "use std::os::windows::io::AsRawHandle;"),
            ("src/platform_linux.rs", "use libc::c_int;"),
            ("src/platform_macos.rs", "use mach2::port;"),
        ];
        files.extend_from_slice(extra);
        scan(&files, true)
    }

    #[test]
    fn a_valid_selector_approves_only_its_trees() {
        assert_eq!(owner(&[]), []);
    }

    #[test]
    fn owner_package_is_not_exempt() {
        let found = owner(&[
            ("src/lib.rs", ""), // ignored: first entry wins
        ]);
        assert_eq!(found, []);
        let found = scan(
            &[
                ("src/lib.rs", &format!("{SELECTOR} mod neutral;")),
                ("src/platform_win.rs", ""),
                ("src/platform_linux.rs", ""),
                ("src/platform_macos.rs", ""),
                ("src/neutral.rs", "fn f() { if cfg!(windows) {} }"),
            ],
            true,
        );
        assert_eq!(found, [("src/neutral.rs".into(), Kind::HostCfg, "cfg!(windows)".into())]);
    }

    #[test]
    fn inactive_out_of_line_modules_are_scanned() {
        let found = scan(
            &[
                ("src/lib.rs", "#[cfg(windows)] #[path = \"win/only.rs\"] mod only; mod a;"),
                ("src/win/only.rs", "use std::os::windows::ffi::OsStrExt;"),
                ("src/a.rs", "#[cfg(target_os = \"macos\")] mod b;"),
                ("src/a/b.rs", "fn f() { unsafe { libc::getpid(); } }"),
            ],
            false,
        );
        assert_eq!(
            found,
            [
                ("src/a/b.rs".into(), Kind::NativeApi, "libc".into()),
                ("src/a.rs".into(), Kind::HostCfg, "cfg(target_os=\"macos\")".into()),
                ("src/lib.rs".into(), Kind::HostCfg, "cfg(windows)".into()),
                ("src/win/only.rs".into(), Kind::NativeApi, "std::os".into()),
            ]
        );
    }

    #[test]
    fn every_selector_and_cfg_form_is_detected() {
        for selector in SELECTORS {
            let source = format!("#[cfg(all(feature = \"x\", not({selector} = \"y\")))] fn f() {{}}");
            assert_eq!(scan(&[("lib.rs", &source)], false).len(), 1, "{selector}");
        }
        let found = scan(
            &[(
                "lib.rs",
                r#"
                #![cfg_attr(unix, allow(dead_code))]
                #[cfg_attr(windows, path = "x.rs")] mod inline {}
                fn f() -> bool { let _ = cfg!(any(test, target_env = "gnu")); true }
                macro_rules! m { () => { #[cfg(target_os = "linux")] fn g() {} } }
                cfg_select! { unix => { fn h() {} } _ => {} }
                "#,
            )],
            false,
        );
        let kinds: Vec<Kind> = found.iter().map(|f| f.1).collect();
        assert_eq!(kinds, [Kind::HostCfg; 5], "{found:?}");
    }

    #[test]
    fn feature_test_docs_and_text_do_not_match() {
        let found = scan(
            &[(
                "lib.rs",
                r##"
                //! cfg(windows) in a comment, and libc::getpid in prose.
                /// `#[cfg(unix)]` documented.
                #[cfg(all(test, feature = "windows", debug_assertions))]
                #[cfg_attr(docsrs, doc(cfg(feature = "x")))]
                fn f() { let windows = "cfg!(unix) std::os::unix"; let libc = r#"libc::x"#; let _ = (windows, libc); }
                "##,
            )],
            false,
        );
        assert_eq!(found, []);
    }

    #[test]
    fn native_paths_are_detected_in_every_spelling() {
        let found = scan(
            &[(
                "lib.rs",
                r#"
                use libc as c;
                use std::{io, os::unix::fs::PermissionsExt};
                extern crate winapi;
                fn f() { ::windows_sys::Win32::Foundation::CloseHandle(0); }
                macro_rules! m { () => { core::os::raw::c_int } }
                type S = tokio::net::UnixStream;
                use interprocess::os::windows::named_pipe;
                use objc2_app_kit::NSView;
                #[link(name = "kernel32")]
                extern "system" { fn GetTickCount() -> u32; }
                extern "C" fn callback() {}
                "#,
            )],
            false,
        );
        let constructs: Vec<&str> = found.iter().map(|f| f.2.as_str()).collect();
        assert_eq!(
            constructs,
            ["libc", "std::os", "winapi", "windows_sys", "core::os", "tokio::net::UnixStream", "interprocess::os", "objc2_app_kit", "#[link]", "extern block"],
            "{found:?}"
        );
    }

    #[test]
    fn concrete_tree_names_are_bridge_only() {
        let found = owner(&[]);
        assert_eq!(found, []);
        let found = scan(
            &[
                (
                    "src/lib.rs",
                    &format!("{SELECTOR} fn f() {{ platform_imp::g(); }} mod n;"),
                ),
                ("src/platform_win.rs", ""),
                ("src/platform_linux.rs", ""),
                ("src/platform_macos.rs", ""),
                ("src/n.rs", "use crate::platform_imp::x; use super::platform_win as w;"),
            ],
            true,
        );
        assert_eq!(
            found,
            [
                ("src/lib.rs".into(), Kind::ConcreteTree, "platform_imp".into()),
                ("src/n.rs".into(), Kind::ConcreteTree, "platform_imp".into()),
                ("src/n.rs".into(), Kind::ConcreteTree, "platform_win".into()),
            ]
        );
    }

    #[test]
    fn selector_shape_is_exact() {
        let bad = [
            // fallback arm
            r#"cfg_select! { target_os = "windows" => { mod platform_win; pub(crate) use platform_win as platform_imp; } _ => {} }"#,
            // broad unix
            r#"cfg_select! { windows => { mod platform_win; pub(crate) use platform_win as platform_imp; } unix => { mod platform_linux; pub(crate) use platform_linux as platform_imp; } }"#,
        ];
        for source in bad {
            let found = scan(&[("src/lib.rs", source)], true);
            assert_eq!(found.first().map(|f| f.1), Some(Kind::Selector), "{source}: {found:?}");
        }
        // A second valid-looking selector is a violation; clients never get one.
        let twice = format!("{SELECTOR}{}", SELECTOR.replace("pub use platform_imp::thing;", ""));
        let found = scan(
            &[
                ("src/lib.rs", &twice),
                ("src/platform_win.rs", ""),
                ("src/platform_linux.rs", ""),
                ("src/platform_macos.rs", ""),
            ],
            true,
        );
        assert_eq!(found.first().map(|f| f.1), Some(Kind::Selector), "{found:?}");
        let found = scan(
            &[
                ("src/lib.rs", SELECTOR),
                ("src/platform_win.rs", ""),
                ("src/platform_linux.rs", ""),
                ("src/platform_macos.rs", ""),
            ],
            false,
        );
        assert!(found.iter().any(|f| f.1 == Kind::HostCfg), "{found:?}");
    }

    #[test]
    fn guest_selector_is_root_only() {
        let root = format!("std::cfg_select! {{ target_family = \"wasm\" => {{ pub mod guest; }} _ => {{ {SELECTOR} }} }}");
        let found = scan(
            &[
                ("src/lib.rs", &root),
                ("src/guest.rs", "mod inner; cfg_select! { all(target_family = \"wasm\", feature = \"x\") => {} _ => {} }"),
                ("src/guest/inner.rs", "#[cfg(target_arch = \"wasm32\")] fn f() {} #[cfg(windows)] fn g() {}"),
                ("src/platform_win.rs", ""),
                ("src/platform_linux.rs", ""),
                ("src/platform_macos.rs", ""),
            ],
            true,
        );
        assert_eq!(found, [("src/guest/inner.rs".into(), Kind::HostCfg, "cfg(windows)".into())]);
        let found = scan(
            &[("src/lib.rs", "mod x;"), ("src/x.rs", "cfg_select! { target_family = \"wasm\" => {} _ => {} }")],
            true,
        );
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn missing_or_unparsable_modules_are_reported() {
        let found = scan(&[("lib.rs", "mod gone; mod bad;"), ("bad.rs", "fn f( {")], false);
        let kinds: Vec<Kind> = found.iter().map(|f| f.1).collect();
        assert_eq!(kinds, [Kind::Unreadable, Kind::Unreadable], "{found:?}");
    }

    #[test]
    fn path_attributes_resolve_relative_to_the_declaring_file() {
        let found = scan(
            &[
                ("tests/cat/main.rs", "mod a; #[path = \"../support/s.rs\"] mod s;"),
                ("tests/cat/a.rs", "mod b;"),
                ("tests/cat/a/b.rs", "use libc::x;"),
                ("tests/support/s.rs", "use std::os::unix::x;"),
            ],
            false,
        );
        let files: Vec<&str> = found.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(files, ["tests/cat/a/b.rs", "tests/support/s.rs"]);
    }
}
