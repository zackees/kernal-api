use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = Path::new(
        &std::env::var_os("CARGO_MANIFEST_DIR")
            .expect("cargo always sets CARGO_MANIFEST_DIR for build.rs"),
    )
    .join("conpty-sidecar.sha256.toml");
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    let rendered = render_hash_table(&read_manifest(&manifest_path));
    let out_dir = std::env::var_os("OUT_DIR").expect("cargo always sets OUT_DIR for build.rs");
    std::fs::write(
        Path::new(&out_dir).join("conpty_sidecar_hashes.rs"),
        rendered,
    )?;
    Ok(())
}

#[derive(Default)]
struct ParsedManifest {
    x64: Option<(String, u64)>,
    arm64: Option<(String, u64)>,
    x86: Option<(String, u64)>,
    arm: Option<(String, u64)>,
}

fn read_manifest(path: &Path) -> ParsedManifest {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return ParsedManifest::default();
    };
    let Ok(table) = raw.parse::<toml::Value>() else {
        return ParsedManifest::default();
    };
    let mut parsed = ParsedManifest::default();
    let Some(assets) = table.get("asset").and_then(toml::Value::as_table) else {
        return parsed;
    };
    for arch in ["x64", "arm64", "x86", "arm"] {
        let Some(entry) = assets.get(arch).and_then(toml::Value::as_table) else {
            continue;
        };
        let Some(sha) = entry.get("sha256").and_then(toml::Value::as_str) else {
            continue;
        };
        let size = entry
            .get("size_bytes")
            .and_then(toml::Value::as_integer)
            .unwrap_or_default()
            .max(0) as u64;
        let slot = match arch {
            "x64" => &mut parsed.x64,
            "arm64" => &mut parsed.arm64,
            "x86" => &mut parsed.x86,
            "arm" => &mut parsed.arm,
            _ => unreachable!(),
        };
        *slot = Some((sha.to_owned(), size));
    }
    parsed
}

fn render_hash_table(parsed: &ParsedManifest) -> String {
    let mut output = String::from(
        "pub(super) struct ExpectedAsset {\n    pub sha256_hex: &'static str,\n    pub size_bytes: u64,\n}\n\n",
    );
    for (name, value) in [
        ("EXPECTED_X64", &parsed.x64),
        ("EXPECTED_ARM64", &parsed.arm64),
        ("EXPECTED_X86", &parsed.x86),
        ("EXPECTED_ARM", &parsed.arm),
    ] {
        let body = value.as_ref().map_or_else(
            || "None".to_owned(),
            |(sha, size)| {
                format!("Some(ExpectedAsset {{ sha256_hex: \"{sha}\", size_bytes: {size} }})")
            },
        );
        output.push_str(&format!(
            "#[allow(dead_code)]\npub(super) const {name}: Option<ExpectedAsset> = {body};\n"
        ));
    }
    output
}
