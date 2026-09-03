//! Source-level release guards for the public facade and protocol boundary.

use std::path::{Path, PathBuf};

fn rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut sources = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).expect("read source directory") {
            let path = entry.expect("source entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push(path);
            }
        }
    }
    sources
}

fn workflow_job<'a>(workflow: &'a str, name: &str) -> &'a str {
    let marker = format!("  {name}:\n");
    let start = workflow
        .find(&marker)
        .unwrap_or_else(|| panic!("release workflow must contain the {name} job"));
    let body = &workflow[start + marker.len()..];
    for (index, _) in body.match_indices("\n  ") {
        if body.as_bytes().get(index + 3) != Some(&b' ') {
            return &body[..index];
        }
    }
    body
}

#[test]
fn implementation_crates_are_not_publicly_reexported() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let forbidden = [
        "pub use addr2line",
        "pub use blake3",
        "pub use console_api",
        "pub use console_subscriber",
        "pub use crash_handler",
        "pub use framehop",
        "pub use globset",
        "pub use interprocess",
        "pub use jwalk",
        "pub use memmap2",
        "pub use mimalloc_pprof",
        "pub use notify",
        "pub use pdb_addr2line",
        "pub use portable_pty",
        "pub use reflink_copy",
        "pub use running_process",
        "pub use sysinfo",
        "pub use tokio",
    ];
    for path in rust_sources(&root) {
        let source = std::fs::read_to_string(&path).expect("read Rust source");
        for spelling in forbidden {
            assert!(
                !source.contains(spelling),
                "{} exposes forbidden backend spelling {spelling:?}",
                path.display()
            );
        }
    }
}

#[test]
fn process_substrate_is_exact_feature_minimal_and_private() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("read manifest");
    assert!(
        manifest.contains(
            "running-process = { version = \"=4.10.10\", default-features = false, features = [\"kernel-substrate\"] }"
        ),
        "the facade must retain the exact published running-process pin and minimal feature set"
    );
    assert!(
        manifest.contains("# Exact first-party pre-1.0 pin."),
        "the released process substrate must retain its exact first-party pin rationale"
    );

    let release_workflow = std::fs::read_to_string(root.join(".github/workflows/release.yml"))
        .expect("read release workflow");
    let release_guard = workflow_job(&release_workflow, "release-guard");
    assert!(
        release_guard.contains("uv run --no-project --with tomli==2.2.1 python")
            && release_guard.contains("ci/check_release_process_substrate.py"),
        "the dedicated release guard must invoke the TOML-aware process-substrate validator"
    );
    for cargo_job in ["validate-and-package", "symbolizer-workers"] {
        assert!(
            workflow_job(&release_workflow, cargo_job).contains("needs: release-guard"),
            "{cargo_job} must depend on release-guard before invoking soldr/cargo"
        );
    }
    assert!(
        workflow_job(&release_workflow, "publish-crates")
            .contains("needs: [release-guard, validate-and-package]"),
        "publish-crates must directly depend on release-guard before cargo publish"
    );

    let lib = std::fs::read_to_string(root.join("src/lib.rs")).expect("read facade root");
    assert!(
        !lib.contains("tokio::process"),
        "the migrated process facade must not retain a Tokio child fallback"
    );
    assert!(
        !lib.contains("configure_command("),
        "the migrated process facade must not retain native spawn configuration"
    );
    let adapter = std::fs::read_to_string(root.join("src/process_adapter.rs"))
        .expect("read private process adapter");
    for mapping in [
        ".create_process_group(create_process_group)",
        ".kill_when_owner_dies(kill_when_owner_dies)",
        ".nice(priority.substrate_nice())",
    ] {
        assert!(
            adapter.contains(mapping),
            "the private adapter must preserve SpawnSpec's {mapping} policy"
        );
    }

    for path in rust_sources(&root.join("src")) {
        let source = std::fs::read_to_string(&path).expect("read Rust source");
        for line in source.lines() {
            let line = line.trim_start();
            if line.starts_with("pub ") {
                assert!(
                    !line.contains("running_process"),
                    "{} exposes a running-process type in {line:?}",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn daemon_identity_remains_opt_in_and_out_of_full() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("read manifest");
    assert!(
        manifest.contains("daemon-identity = [\"running-process/backend-identity\"]"),
        "daemon identity must compose only the direct backend-identity substrate feature"
    );

    let full = manifest
        .split("full = [")
        .nth(1)
        .and_then(|tail| tail.split(']').next())
        .expect("locate full feature");
    assert!(
        !full.contains("daemon-identity"),
        "full must retain the established heavyweight feature set without direct-daemon identity"
    );

    let lib = std::fs::read_to_string(root.join("src/lib.rs")).expect("read facade root");
    assert!(
        lib.contains("#[cfg(feature = \"daemon-identity\")]\npub mod daemon_identity;"),
        "default builds must omit the daemon identity facade module"
    );
}

#[test]
fn daemon_frame_v1_remains_transport_free_and_product_neutral() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("read manifest");
    assert!(
        manifest.contains("daemon-frame-v1 = [\"running-process/frame-v1-codec\"]"),
        "the facade feature must select only the upstream frame-only codec"
    );
    let feature = manifest
        .split("daemon-frame-v1 = [")
        .nth(1)
        .and_then(|tail| tail.split(']').next())
        .expect("locate daemon-frame-v1 feature");
    for forbidden in [
        "backend-identity",
        "client",
        "ipc",
        "blake3",
        "sha2",
        "getrandom",
        "tokio",
    ] {
        assert!(
            !feature.contains(forbidden),
            "daemon-frame-v1 must not select {forbidden:?}"
        );
    }

    let full = manifest
        .split("full = [")
        .nth(1)
        .and_then(|tail| tail.split(']').next())
        .expect("locate full feature");
    assert!(
        !full.contains("daemon-frame-v1"),
        "full must retain its established heavyweight feature set without daemon-frame-v1"
    );

    let lib = std::fs::read_to_string(root.join("src/lib.rs")).expect("read facade root");
    assert!(
        lib.contains("#[cfg(feature = \"daemon-frame-v1\")]\npub mod daemon_frame_v1;"),
        "default builds must omit the daemon frame facade module"
    );

    let frame = std::fs::read_to_string(root.join("src/daemon_frame_v1.rs"))
        .expect("read daemon-frame facade");
    assert!(
        !frame.contains("0x7A63"),
        "zccache's product protocol identifier must not be owned by kernal-api"
    );
    for line in frame
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("pub "))
    {
        for forbidden in [
            "running_process",
            "prost",
            "BytesMut",
            "tokio",
            "RawFd",
            "RawHandle",
        ] {
            assert!(
                !line.contains(forbidden),
                "daemon-frame facade leaks {forbidden:?}: {line}"
            );
        }
    }
}

#[test]
fn daemon_registration_remains_opt_in_and_client_free() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("read manifest");
    assert!(
        manifest.contains("daemon-registration = [\"running-process/daemon-registration\"]"),
        "daemon registration must select only the direct registration substrate"
    );
    let feature = manifest
        .split("daemon-registration = [")
        .nth(1)
        .and_then(|tail| tail.split(']').next())
        .expect("locate daemon-registration feature");
    for forbidden in [
        "backend-identity",
        "client",
        "ipc",
        "blake3",
        "tokio",
        "runtime",
    ] {
        assert!(
            !feature.contains(forbidden),
            "daemon-registration must not select {forbidden:?}"
        );
    }

    let full = manifest
        .split("full = [")
        .nth(1)
        .and_then(|tail| tail.split(']').next())
        .expect("locate full feature");
    assert!(
        !full.contains("daemon-registration"),
        "full must retain its established heavyweight feature set without daemon registration"
    );

    let lib = std::fs::read_to_string(root.join("src/lib.rs")).expect("read facade root");
    assert!(
        lib.contains("#[cfg(feature = \"daemon-registration\")]\npub mod daemon_registration;"),
        "default builds must omit the daemon-registration facade module"
    );

    let registration = std::fs::read_to_string(root.join("src/daemon_registration.rs"))
        .expect("read daemon-registration facade");
    for forbidden in ["protocol_v2", "0x7A63", "zccache"] {
        assert!(
            !registration.contains(forbidden),
            "daemon-registration must not own {forbidden:?}"
        );
    }
    for line in registration
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("pub "))
    {
        for forbidden in [
            "running_process",
            "prost",
            "backend::",
            "tokio",
            "BytesMut",
            "RawFd",
            "RawHandle",
            "platform",
        ] {
            assert!(
                !line.contains(forbidden),
                "daemon-registration facade leaks {forbidden:?}: {line}"
            );
        }
    }
    assert!(
        registration.contains("self.inner.install().map_err(service_error)")
            && registration.contains("self.inner.install_in(root.as_ref()).map_err(service_error)")
            && !registration.contains("fs::write"),
        "service-definition persistence must delegate to the frozen upstream non-atomic v1 writer"
    );

    let consumer_root = root.join("tests/daemon-registration-consumer");
    let consumer_manifest = std::fs::read_to_string(consumer_root.join("Cargo.toml"))
        .expect("read external daemon-registration consumer manifest");
    assert!(
        consumer_manifest.contains("[workspace]")
            && consumer_manifest
                .contains("default-features = false, features = [\"daemon-registration\"]"),
        "external consumer must compile the opt-in facade outside this workspace"
    );
    let consumer_source = std::fs::read_to_string(consumer_root.join("src/main.rs"))
        .expect("read external daemon-registration consumer source");
    // A forbidden-substring loop is vacuously true on an emptied file, so
    // require the calls this fixture claims to make. CI's `facade-consumers`
    // job is what actually compiles it; this is only the backstop against the
    // fixture being hollowed out.
    for required in [
        "use kernal_api::daemon_registration::",
        "CacheManifestBuilder::new",
        "ServiceDefinitionBuilder::shared_broker",
    ] {
        assert!(
            consumer_source.contains(required),
            "external consumer must still exercise {required:?}"
        );
    }
    for forbidden in ["running_process", "prost", "tokio", "platform"] {
        assert!(
            !consumer_source.contains(forbidden),
            "external consumer must not require {forbidden:?}"
        );
    }
}

#[test]
fn daemon_registration_v2_remains_opt_in_and_client_free() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("read manifest");
    assert!(
        manifest.contains("daemon-registration-v2 = [\"running-process/daemon-registration-v2\"]"),
        "daemon registration v2 must select only the direct v2 registration substrate"
    );
    let feature = manifest
        .split("daemon-registration-v2 = [")
        .nth(1)
        .and_then(|tail| tail.split(']').next())
        .expect("locate daemon-registration-v2 feature");
    for forbidden in [
        "daemon-registration\"",
        "backend-identity",
        "client",
        "ipc",
        "blake3",
        "sha2",
        "tokio",
        "runtime",
    ] {
        assert!(
            !feature.contains(forbidden),
            "daemon-registration-v2 must not select {forbidden:?}"
        );
    }

    let full = manifest
        .split("full = [")
        .nth(1)
        .and_then(|tail| tail.split(']').next())
        .expect("locate full feature");
    assert!(
        !full.contains("daemon-registration-v2"),
        "full must retain its established heavyweight feature set without daemon registration v2"
    );

    let lib = std::fs::read_to_string(root.join("src/lib.rs")).expect("read facade root");
    assert!(
        lib.contains(
            "#[cfg(feature = \"daemon-registration-v2\")]\npub mod daemon_registration_v2;"
        ),
        "default builds must omit the daemon-registration-v2 facade module"
    );

    let registration = std::fs::read_to_string(root.join("src/daemon_registration_v2.rs"))
        .expect("read daemon-registration-v2 facade");
    for forbidden in ["protocol_v2", "http_server", "zccache"] {
        assert!(
            !registration.contains(forbidden),
            "daemon-registration-v2 must not own {forbidden:?}"
        );
    }
    for line in registration
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("pub "))
    {
        for forbidden in [
            "running_process",
            "prost",
            "backend",
            "tokio",
            "BytesMut",
            "RawFd",
            "RawHandle",
            "platform",
        ] {
            assert!(
                !line.contains(forbidden),
                "daemon-registration-v2 facade leaks {forbidden:?}: {line}"
            );
        }
    }
    assert!(
        registration.contains("backend::write_service_definition_v2")
            && !registration.contains("fs::write"),
        "v2 persistence must delegate to the frozen upstream non-atomic writer"
    );

    let consumer_root = root.join("tests/daemon-registration-v2-consumer");
    let consumer_manifest = std::fs::read_to_string(consumer_root.join("Cargo.toml"))
        .expect("read external daemon-registration-v2 consumer manifest");
    assert!(
        consumer_manifest.contains("[workspace]")
            && consumer_manifest
                .contains("default-features = false, features = [\"daemon-registration-v2\"]"),
        "external consumer must compile the opt-in v2 facade outside this workspace"
    );
    let consumer_source = std::fs::read_to_string(consumer_root.join("src/main.rs"))
        .expect("read external daemon-registration-v2 consumer source");
    // Same backstop as the v1 consumer above: keep the emptied-fixture case
    // from passing the forbidden-substring loop by default.
    for required in [
        "use kernal_api::daemon_registration_v2::",
        "ServiceDefinitionBuilder::shared_broker",
        "service_definition_path(",
    ] {
        assert!(
            consumer_source.contains(required),
            "external v2 consumer must still exercise {required:?}"
        );
    }
    for forbidden in ["running_process", "prost", "tokio", "platform"] {
        assert!(
            !consumer_source.contains(forbidden),
            "external consumer must not require {forbidden:?}"
        );
    }
}

#[test]
fn process_session_surface_keeps_backend_and_native_status_types_private() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib = std::fs::read_to_string(root.join("src/lib.rs")).expect("read facade root");
    let session = lib
        .split("/// Explicit bounds and terminal-owner policy for [`ProcessSession`].")
        .nth(1)
        .and_then(|surface| {
            surface
                .split("/// Status-preserving result of bounded child-output capture.")
                .next()
        })
        .expect("locate public process-session facade");
    for forbidden in [
        "running_process",
        "tokio::",
        "std::process::Command",
        "std::process::Child",
        "ExitStatus",
    ] {
        assert!(
            !session.contains(forbidden),
            "process-session facade leaks {forbidden:?}"
        );
    }
    let session_exit = lib
        .split("pub struct ProcessSessionExit")
        .nth(1)
        .and_then(|surface| {
            surface
                .split("/// Signed compatibility termination code")
                .next()
        })
        .expect("locate facade-owned session exit status");
    assert!(
        session_exit.contains("native_status: u32") && session_exit.contains("signal: Option<i32>"),
        "session exit must retain a facade-owned native status plus Unix signal semantics"
    );
}

/// Owned-crate spellings that name a backend type wherever they appear in a
/// type position. Mirrors `OWNED_IMPLEMENTATION_CRATES` in
/// `dylints/kernal_api_boundary`.
const OWNED_BACKEND_PATHS: [&str; 18] = [
    "addr2line::",
    "blake3::",
    "console_api::",
    "console_subscriber::",
    "crash_handler::",
    "framehop::",
    "globset::",
    "interprocess::",
    "jwalk::",
    "memmap2::",
    "mimalloc_pprof::",
    "notify::",
    "pdb_addr2line::",
    "portable_pty::",
    "reflink_copy::",
    "running_process::",
    "sysinfo::",
    "tokio::",
];

/// A Dylint pass resolves types, but only for the code the compiler compiles,
/// and `default = []`. A `dylints` job without `--all-features` skips `crash`,
/// `wasm`, `symbolize`, and every other gated module while still reporting
/// green, which is the failure this guard exists to prevent (#108).
#[test]
fn the_dylint_job_lints_every_feature_gated_module() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workflow =
        std::fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("read CI workflow");
    assert!(
        workflow_job(&workflow, "dylints")
            .contains("--all --workspace -- --all-features --all-targets"),
        "the boundary lints must run over every feature-gated module and target"
    );
}

/// An always-on companion to `kernal_api_boundary`, whose HIR pass sees only
/// the code compiled on the lint host -- every feature since #108, but never a
/// `cfg(windows)` or `cfg(target_os = "macos")` body, because that job runs on
/// Linux. This scan is deliberately coarse: it covers the single-line shapes
/// -- `pub` items, every variant of a `pub enum`, and the `pub` fields of a
/// `pub struct` -- and leaves wrapped signatures, bounds, and alias chasing to
/// the lint.
#[test]
fn backend_types_are_absent_from_public_type_positions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for path in rust_sources(&root) {
        let source = std::fs::read_to_string(&path).expect("read Rust source");
        for (line, position) in public_type_positions(&source) {
            for spelling in OWNED_BACKEND_PATHS {
                assert!(
                    !position.contains(spelling),
                    "{}:{line} names backend type {spelling:?} in a public type position: {}",
                    path.display(),
                    position.trim()
                );
            }
        }
    }
}

/// Split a single-line tuple-struct declaration into its header -- name,
/// generics, and bounds -- and its parenthesized field list.
fn tuple_struct_split(line: &str) -> Option<(&str, &str)> {
    if !line.starts_with("pub struct") {
        return None;
    }
    let open = line.find('(')?;
    let close = line.rfind(')')?;
    (open < close).then(|| (&line[..open], &line[open + 1..close]))
}

/// The one-based line number and type-bearing text of every public type
/// position in `source`.
fn public_type_positions(source: &str) -> Vec<(usize, &str)> {
    let mut positions = Vec::new();
    // Indentation of the `pub enum`/`pub struct` header whose body is open,
    // plus whether every field of that body is public. A `pub(crate)` field
    // does not begin with `pub `, so it is not a public type position; every
    // field of a public enum variant is as visible as the enum itself.
    let mut open: Option<(usize, bool)> = None;
    for (index, raw) in source.lines().enumerate() {
        let line = raw.trim_start();
        let indent = raw.len() - line.len();
        if line.starts_with("//") {
            continue;
        }
        if let Some((body_indent, fields_are_public)) = open {
            if line == "}" && indent == body_indent {
                open = None;
            } else if fields_are_public || line.starts_with("pub ") {
                positions.push((index + 1, line));
            }
            continue;
        }
        if !line.starts_with("pub ") {
            continue;
        }
        // A `pub const`/`pub static` initializer is a value, not a type, and
        // the fields of a tuple struct carry their own visibility.
        let declaration = if line.starts_with("pub const") || line.starts_with("pub static") {
            line.split('=').next().unwrap_or(line)
        } else if let Some((header, fields)) = tuple_struct_split(line) {
            if fields
                .split(',')
                .any(|field| field.trim_start().starts_with("pub "))
            {
                line
            } else {
                header
            }
        } else {
            line
        };
        positions.push((index + 1, declaration));
        if line.ends_with('{') {
            let variant_fields_are_public = line.starts_with("pub enum");
            if variant_fields_are_public || line.starts_with("pub struct") {
                open = Some((indent, variant_fields_are_public));
            }
        }
    }
    positions
}

#[test]
fn json_is_confined_to_the_external_firefox_export() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for path in rust_sources(&root) {
        let relative = path.strip_prefix(&root).expect("source below root");
        let normalized = relative.to_string_lossy().replace('\\', "/");
        let source = std::fs::read_to_string(&path).expect("read Rust source");
        if source.contains("serde_json") {
            assert!(
                matches!(
                    normalized.as_str(),
                    "profile/export/firefox.rs" | "profile/tests.rs"
                ),
                "{} uses JSON outside the Firefox export boundary",
                path.display()
            );
        }
    }
}
