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
        "pub use interprocess",
        "pub use mimalloc_pprof",
        "pub use pdb_addr2line",
        "pub use portable_pty",
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
        manifest
            .contains("# Exact first-party pre-1.0 pin."),
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
