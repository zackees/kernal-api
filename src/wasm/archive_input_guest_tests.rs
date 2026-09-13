use super::*;
use crate::operations::archive_input::EncryptedInput;
use std::io::Write;

#[test]
#[ignore = "requires Cargo-built auth-proof artifact in KERNAL_EXTENSION2_AUTH_WASM"]
fn authenticated_input_actual_guest_authenticates_large_zip_and_rejects_bad_tag_or_nonce() {
    authenticated_guest_control("KERNAL_EXTENSION2_AUTH_WASM");
}

#[test]
#[ignore = "requires Cargo-built inventory-proof artifact in KERNAL_EXTENSION2_INVENTORY_WASM"]
fn authenticated_input_actual_guest_enumerates_large_zip_and_rejects_bad_tag_or_nonce() {
    authenticated_guest_control("KERNAL_EXTENSION2_INVENTORY_WASM");
}

fn authenticated_guest_control(artifact_variable: &str) {
    let bytes =
        std::fs::read(std::env::var_os(artifact_variable).expect("authenticated guest artifact"))
            .unwrap();
    let chunk = 64 * 1024;
    let blobs = SketchBlobLimits::new(chunk, 2 * chunk, 4 * chunk, 4, 4, 4).unwrap();
    // Checking every byte of 17 MiB needs more than the tiny compatibility
    // default (100k root fuel). Keep a fixed, finite workload-specific budget.
    let execution = SketchExecutionLimits::default()
        .with_blob_limits(blobs)
        .with_fuel_limits(SketchFuelLimits::new(501_600_000, 500_000_000, 100_000).unwrap())
        .unwrap();
    let compiler = SketchCompiler::new(
        SketchCompilerConfig::default()
            .with_execution_limits(execution)
            .unwrap(),
    )
    .unwrap();
    let policy =
        SketchModulePolicy::threaded_rust_v1(bytes.len() + 1, THREADED_RUST_MAX_PAGES).unwrap();
    let runtime = crate::async_engine::RuntimeBuilder::current_thread()
        .enable_all()
        .build()
        .unwrap();
    for case in 0..3 {
        let sketch = compiler.admit(&bytes, policy).unwrap();
        let header = br#"{ "schemaVersion":1, "algorithm":"AES-128-GCM", "version":"synthetic-1", "commit":"synthetic-commit", "keyId":"synthetic-key", "nonce":"AAAAAAAAAAAAAAAA" }"#;
        let (input, length) = crate::operations::archive_input::tests::encrypted_zip_with_header(
            header,
            [7; 16],
            if case == 2 { [8; 12] } else { [0; 12] },
            case == 1,
        );
        assert!(length > 16 * 1024 * 1024);
        let result = runtime.run(sketch.execute_threaded_root_with_grant(
            runtime.handle(),
            crate::async_engine::CancellationSource::new().token(),
            RootGrants {
                archive: Some(input),
                ..RootGrants::default()
            },
        ));
        assert_eq!(
            result,
            if case == 0 {
                Ok(ThreadedRootOutcome::Started)
            } else {
                Err(SketchExecutionError::NonzeroExit { code: 1 })
            }
        );
        let snapshot = sketch
            .root_execution_observation_for_test()
            .unwrap()
            .operation_snapshot
            .unwrap();
        assert_eq!(snapshot.live_resources, 0);
        assert_eq!(snapshot.pending_operations, 0);
        assert_eq!(snapshot.archive_staging_bytes, 0);
        assert_eq!(snapshot.active_archive_jobs, 0);
        assert_eq!(snapshot.retained_transfer_capacity, 0);
        assert!(snapshot.peak_retained_transfer_capacity <= blobs.maximum_transfer_bytes());
        assert!(snapshot.peak_buffered_blob_bytes <= blobs.maximum_blob_bytes());
        if case == 0 && artifact_variable == "KERNAL_EXTENSION2_STREAM_WASM" {
            assert!(snapshot.peak_buffered_blob_bytes > 0);
        }
        sketch.close_threaded_root().unwrap();
    }
    assert_eq!(
        compiler.execution_limits_snapshot(),
        SketchExecutionSnapshot::default()
    );
}

#[test]
#[ignore = "requires Cargo-built guest-proof artifact in KERNAL_EXTENSION2_STREAM_WASM"]
fn authenticated_input_actual_guest_streams_large_zip_and_rejects_bad_tag_or_nonce() {
    authenticated_guest_control("KERNAL_EXTENSION2_STREAM_WASM");
}

#[test]
#[ignore = "requires Cargo-built header-proof artifact in KERNAL_EXTENSION2_HEADER_WASM"]
fn authenticated_input_actual_guest_validates_header_and_rejects_missing_or_wrong_identity() {
    let artifact =
        std::env::var_os("KERNAL_EXTENSION2_HEADER_WASM").expect("header-proof artifact");
    let bytes = std::fs::read(artifact).unwrap();
    let compiler = SketchCompiler::new(SketchCompilerConfig::default()).unwrap();
    let policy =
        SketchModulePolicy::threaded_rust_v1(bytes.len() + 1, THREADED_RUST_MAX_PAGES).unwrap();
    let runtime = crate::async_engine::RuntimeBuilder::current_thread()
        .enable_all()
        .build()
        .unwrap();
    for case in 0..3 {
        let sketch = compiler.admit(&bytes, policy).unwrap();
        let archive = if case == 2 {
            None
        } else {
            let mut header = br#"{ "schemaVersion":1, "algorithm":"AES-128-GCM", "version":"synthetic-1", "commit":"synthetic-commit", "keyId":"synthetic-key", "nonce":"AAAAAAAAAAAAAAAA" }"#.to_vec();
            if case == 1 {
                let offset = header
                    .windows(b"synthetic-1".len())
                    .position(|part| part == b"synthetic-1")
                    .unwrap();
                header[offset] = b'X';
            }
            let mut source = tempfile::tempfile().unwrap();
            source.write_all(b"TWPV1AES").unwrap();
            source
                .write_all(&(header.len() as u32).to_be_bytes())
                .unwrap();
            source.write_all(&header).unwrap();
            // Header-only control: this sparse tail is deliberately NOT a
            // valid encrypted ZIP and must never be cited as crypto evidence.
            source
                .set_len((12 + header.len() + 17 * 1024 * 1024 + 16) as u64)
                .unwrap();
            Some(EncryptedInput::open(source, [0; 16], 512 * 1024 * 1024).unwrap())
        };
        let result = runtime.run(sketch.execute_threaded_root_with_grant(
            runtime.handle(),
            crate::async_engine::CancellationSource::new().token(),
            RootGrants {
                archive,
                ..RootGrants::default()
            },
        ));
        if case == 0 {
            assert_eq!(result, Ok(ThreadedRootOutcome::Started));
        } else {
            assert_eq!(result, Err(SketchExecutionError::NonzeroExit { code: 1 }));
        }
        let snapshot = sketch
            .root_execution_observation_for_test()
            .unwrap()
            .operation_snapshot
            .unwrap();
        assert_eq!(snapshot.live_resources, 0);
        assert_eq!(snapshot.pending_operations, 0);
        sketch.close_threaded_root().unwrap();
    }
    assert_eq!(
        compiler.execution_limits_snapshot(),
        SketchExecutionSnapshot::default()
    );
}
