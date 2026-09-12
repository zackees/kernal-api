use fp_bindgen::prelude::*;

include!("../../../abi/kernal-api-v1.rs");

fn main() {
    let output = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: kernal-api-wasm-abi-gen <output-directory>");
        std::process::exit(2);
    });
    if output == "--stamp" {
        let Some(path) = std::env::args().nth(2) else {
            eprintln!("usage: kernal-api-wasm-abi-gen --stamp <wasm-file>");
            std::process::exit(2);
        };
        let bytes = std::fs::read(&path).expect("read wasm artifact");
        let stamped = stamp_metadata(&bytes).expect("valid, compatible core-Wasm artifact");
        std::fs::write(path, stamped).expect("stamp wasm artifact");
        return;
    }
    fp_bindgen!(BindingConfig {
        bindings_type: BindingsType::RustWasmtimeCoreWasm,
        path: &output,
    });
    let manifest = std::fs::read_to_string(format!("{output}/kernal-api-v1.abi.toml"))
        .expect("read generated ABI manifest");
    let descriptor = render_admission_descriptor(&manifest);
    std::fs::write(format!("{output}/admission.rs"), descriptor)
        .expect("write generated admission descriptor");
    std::fs::write(
        format!("{output}/contract.rs"),
        include_str!("../../../abi/constants.rs"),
    )
    .expect("write generated protocol constants");
}

fn take_leb(bytes: &[u8], cursor: &mut usize) -> Result<usize, &'static str> {
    let mut value = 0_u32;
    for shift in (0..35).step_by(7) {
        let byte = *bytes.get(*cursor).ok_or("truncated section length")?;
        *cursor += 1;
        if shift == 28 && byte > 15 {
            return Err("section length overflow");
        }
        value |= u32::from(byte & 127) << shift;
        if byte & 128 == 0 {
            return Ok(value as usize);
        }
    }
    Err("invalid section length")
}

fn stamp_metadata(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    const NAME: &[u8] = b"kernal-api.core-abi";
    if !bytes.starts_with(b"\0asm\x01\0\0\0") {
        return Err("expected core-Wasm v1 header");
    }
    let mut cursor = 8;
    let mut seen = false;
    while cursor < bytes.len() {
        let id = bytes[cursor];
        cursor += 1;
        let length = take_leb(bytes, &mut cursor)?;
        let end = cursor.checked_add(length).ok_or("section overflow")?;
        let payload = bytes.get(cursor..end).ok_or("truncated section")?;
        if id == 0 {
            let mut name_start = 0;
            let name_len = take_leb(payload, &mut name_start)?;
            let name_end = name_start.checked_add(name_len).ok_or("name overflow")?;
            let name = payload.get(name_start..name_end).ok_or("truncated name")?;
            if name == NAME {
                if seen || payload.get(name_end..) != Some(CORE_ABI_METADATA) {
                    return Err("duplicate or incompatible core ABI metadata");
                }
                seen = true;
            }
        }
        cursor = end;
    }
    let mut output = bytes.to_vec();
    if !seen {
        let mut payload = Vec::new();
        leb(NAME.len() as u32, &mut payload);
        payload.extend_from_slice(NAME);
        payload.extend_from_slice(CORE_ABI_METADATA);
        output.push(0);
        leb(payload.len() as u32, &mut output);
        output.extend(payload);
    }
    Ok(output)
}

fn render_admission_descriptor(manifest: &str) -> String {
    let value: toml::Value = manifest.parse().expect("generated ABI manifest is TOML");
    let namespace = value["namespace"].as_str().expect("manifest namespace");
    let mut rows = Vec::new();
    for import in value["imports"].as_array().expect("manifest imports") {
        assert_eq!(
            import["namespace"].as_str(),
            Some(namespace),
            "one exact ABI namespace"
        );
        let name = import["name"].as_str().expect("import name");
        let types = |key: &str| -> String {
            let values = import[key].as_array().expect("signature values");
            let mapped = values
                .iter()
                .map(|value| match value["abi"].as_str().unwrap() {
                    "i32" => "ValType::I32",
                    "i64" => "ValType::I64",
                    "f32" => "ValType::F32",
                    "f64" => "ValType::F64",
                    other => panic!("unsupported ABI {other}"),
                })
                .collect::<Vec<_>>();
            format!("&[{}]", mapped.join(", "))
        };
        rows.push(format!(
            "    GeneratedImport {{ name: {name:?}, params: {}, results: {} }},",
            types("params"),
            types("results")
        ));
    }
    format!("// Generated from kernal-api-v1.abi.toml; do not edit.\npub(crate) const ABI_NAMESPACE: &str = {namespace:?};\npub(crate) const GENERATED_IMPORTS: &[GeneratedImport] = &[\n{}\n];\n", rows.join("\n"))
}

fn leb(mut value: u32, output: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::render_admission_descriptor;

    #[test]
    fn metadata_stamp_is_idempotent_and_rejects_conflicts() {
        let minimal = b"\0asm\x01\0\0\0";
        let stamped = super::stamp_metadata(minimal).unwrap();
        assert_eq!(super::stamp_metadata(&stamped).unwrap(), stamped);
        let mut duplicate = stamped.clone();
        duplicate.extend_from_slice(&stamped[8..]);
        assert!(super::stamp_metadata(&duplicate).is_err());
        let mut incompatible = stamped.clone();
        *incompatible.last_mut().unwrap() = b'1';
        assert!(super::stamp_metadata(&incompatible).is_err());
        assert!(super::stamp_metadata(b"not wasm").is_err());
        assert!(super::stamp_metadata(&stamped[..stamped.len() - 1]).is_err());
    }

    #[test]
    fn descriptor_follows_manifest_names_and_signatures() {
        let manifest = r#"namespace = "kernal-api:v1"
[[imports]]
namespace = "kernal-api:v1"
name = "first"
params = [{ abi = "i32" }]
results = [{ abi = "i64" }]
"#;
        let changed = manifest.replace("first", "renamed").replace("i64", "f32");
        let first = render_admission_descriptor(manifest);
        let second = render_admission_descriptor(&changed);
        assert!(first.contains("first") && first.contains("ValType::I64"));
        assert!(second.contains("renamed") && second.contains("ValType::F32"));
        assert_ne!(first, second);
    }
}
