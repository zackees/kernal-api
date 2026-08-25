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

#[test]
fn implementation_crates_are_not_publicly_reexported() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let forbidden = [
        "pub use addr2line",
        "pub use console_api",
        "pub use console_subscriber",
        "pub use crash_handler",
        "pub use framehop",
        "pub use interprocess",
        "pub use mimalloc_pprof",
        "pub use pdb_addr2line",
        "pub use portable_pty",
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
