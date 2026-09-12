//! Explicit generation of the private core-Wasm contract; never a build script.
use fp_bindgen::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

const CAPABILITIES: u32 = 0;
const METADATA_SECTION: &str = "kernal-api.abi";
fp_import! { fn kernel_yield(); }
fp_export! {}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::var_os("KERNAL_API_ABI_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src/wasm/generated/v1")
        });
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if !arguments.is_empty() {
        if arguments.len() != 2 || arguments[0] != "--embed-threaded-metadata" {
            return Err("usage: wasm-abi-generator [--embed-threaded-metadata <artifact>]".into());
        }
        let manifest = fs::read_to_string(output.join("kernal-api-v1.abi.toml"))?;
        let contract = Contract::parse(&manifest)?;
        let path = Path::new(&arguments[1]);
        let original = fs::read(path)?;
        let embedded = embed_metadata(&original, contract.metadata.as_bytes())?;
        if embedded != original {
            fs::write(path, embedded)?;
        }
        return Ok(());
    }
    fp_bindgen!(BindingConfig {
        bindings_type: BindingsType::RustWasmtimeCoreWasm,
        path: output
            .to_str()
            .ok_or("generator output path is not UTF-8")?,
    });
    relocate_guest_package(&output)?;
    let manifest = fs::read_to_string(output.join("kernal-api-v1.abi.toml"))?;
    fs::write(
        output.join("admission_contract.rs"),
        Contract::parse(&manifest)?.render(),
    )?;
    Ok(())
}

fn relocate_guest_package(output: &Path) -> std::io::Result<()> {
    let guest = output.join("guest");
    fs::create_dir_all(guest.join("src"))?;
    // Replace only the two files owned by generation, preserving build output.
    fs::copy(output.join("Cargo.toml"), guest.join("Cargo.toml"))?;
    fs::copy(output.join("src/lib.rs"), guest.join("src/lib.rs"))?;
    fs::remove_file(output.join("Cargo.toml"))?;
    fs::remove_file(output.join("src/lib.rs"))?;
    fs::remove_dir(output.join("src"))?;
    Ok(())
}

#[derive(Debug)]
struct Contract {
    schema: String,
    schema_revision: u8,
    generator_revision: u8,
    abi_version: u8,
    namespace: String,
    import_name: String,
    metadata: String,
}
impl Contract {
    fn parse(manifest: &str) -> Result<Self, String> {
        let value: toml::Value = manifest
            .parse()
            .map_err(|error| format!("invalid manifest: {error}"))?;
        let root = value.as_table().ok_or("manifest must be a table")?;
        let string = |key: &str| -> Result<String, String> {
            root.get(key)
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("missing string field {key}"))
        };
        let revision = |key: &str| -> Result<u8, String> {
            root.get(key)
                .and_then(toml::Value::as_integer)
                .and_then(|value| u8::try_from(value).ok())
                .ok_or_else(|| format!("invalid revision field {key}"))
        };
        let imports = root
            .get("imports")
            .and_then(toml::Value::as_array)
            .ok_or("missing imports")?;
        if imports.len() != 1 {
            return Err("this admission adapter requires exactly one scalar yield import".into());
        }
        if root
            .get("exports")
            .is_some_and(|value| value.as_array().is_none_or(|values| !values.is_empty()))
        {
            return Err("this admission adapter does not yet support generated exports".into());
        }
        let import = imports[0].as_table().ok_or("import must be a table")?;
        let namespace = string("namespace")?;
        if import.get("namespace").and_then(toml::Value::as_str) != Some(namespace.as_str()) {
            return Err("import namespace disagrees with manifest namespace".into());
        }
        let import_name = import
            .get("name")
            .and_then(toml::Value::as_str)
            .ok_or("missing import name")?;
        if import.get("direction").and_then(toml::Value::as_str) != Some("guest-to-host") {
            return Err("unexpected import direction".into());
        }
        if !import
            .get("params")
            .and_then(toml::Value::as_array)
            .is_some_and(Vec::is_empty)
        {
            return Err("yield import must have no parameters".into());
        }
        let results = import
            .get("results")
            .and_then(toml::Value::as_array)
            .ok_or("missing results")?;
        if results.len() != 1
            || results[0].as_table().is_none_or(|result| {
                result.len() != 2
                    || result.get("semantic").and_then(toml::Value::as_str) != Some("()")
                    || result.get("abi").and_then(toml::Value::as_str) != Some("unit")
            })
        {
            return Err("yield import must have exactly one unit result declaration".into());
        }
        Ok(Self {
            schema: string("schema")?,
            schema_revision: revision("schema_revision")?,
            generator_revision: revision("generator_revision")?,
            abi_version: revision("abi_version")?,
            namespace,
            import_name: import_name.to_owned(),
            // Exact bytes bind every signature/version, without a lossy fingerprint.
            metadata: format!("capabilities={CAPABILITIES}\n{manifest}"),
        })
    }
    fn render(&self) -> String {
        format!(
            "// Generated from kernal-api-v1.abi.toml; do not edit.\n\
             #[cfg(test)]\npub(crate) const SCHEMA: &str = {schema:?};\n\
             #[cfg(test)]\npub(crate) const SCHEMA_REVISION: u8 = {schema_revision};\n\
             #[cfg(test)]\npub(crate) const GENERATOR_REVISION: u8 = {generator_revision};\n\
             #[cfg(test)]\npub(crate) const ABI_VERSION: u8 = {abi_version};\n\
             pub(crate) const NAMESPACE: &str = {namespace:?};\n\
             pub(crate) const KERNEL_YIELD: &str = {import_name:?};\n\
             pub(crate) const KERNEL_YIELD_PARAMS: &[wasmparser::ValType] = &[];\n\
             pub(crate) const KERNEL_YIELD_RESULTS: &[wasmparser::ValType] = &[];\n\
             pub(crate) const METADATA: &[u8] = {metadata:?}.as_bytes();\n",
            schema = self.schema,
            schema_revision = self.schema_revision,
            generator_revision = self.generator_revision,
            abi_version = self.abi_version,
            namespace = self.namespace,
            import_name = self.import_name,
            metadata = self.metadata
        )
    }
}

fn embed_metadata(original: &[u8], metadata: &[u8]) -> Result<Vec<u8>, String> {
    if !original.starts_with(b"\0asm\x01\0\0\0") {
        return Err("invalid core-Wasm artifact header".into());
    }
    let mut found = false;
    for payload in wasmparser::Parser::new(0).parse_all(original) {
        let payload = payload.map_err(|error| format!("invalid Wasm artifact: {error}"))?;
        if let wasmparser::Payload::CustomSection(section) = payload {
            if section.name() == METADATA_SECTION {
                if found {
                    return Err("duplicate ABI metadata sections".into());
                }
                if section.data() != metadata {
                    return Err("mismatched ABI metadata section".into());
                }
                found = true;
            }
        }
    }
    let mut output = original.to_vec();
    if !found {
        let mut body = Vec::new();
        push_leb(METADATA_SECTION.len(), &mut body);
        body.extend_from_slice(METADATA_SECTION.as_bytes());
        body.extend_from_slice(metadata);
        output.push(0);
        push_leb(body.len(), &mut output);
        output.extend_from_slice(&body);
    }
    Ok(output)
}
fn push_leb(mut value: usize, output: &mut Vec<u8>) {
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
    use super::*;
    const MANIFEST: &str = include_str!("../../../src/wasm/generated/v1/kernal-api-v1.abi.toml");
    const EMPTY: &[u8] = b"\0asm\x01\0\0\0";

    #[test]
    fn metadata_binds_the_entire_generated_contract() {
        let contract = Contract::parse(MANIFEST).unwrap();
        assert_eq!(contract.metadata, format!("capabilities=0\n{MANIFEST}"));
        let changed = MANIFEST.replace("abi_version = 1", "abi_version = 2");
        assert_ne!(
            contract.metadata,
            Contract::parse(&changed).unwrap().metadata
        );
    }
    #[test]
    fn unsupported_shapes_cannot_borrow_another_imports_signature() {
        let mut value: toml::Value = MANIFEST.parse().unwrap();
        let imports = value.get_mut("imports").unwrap().as_array_mut().unwrap();
        let other = imports[0].clone();
        imports[0]["params"] = toml::Value::Array(vec![toml::Value::String("i32".into())]);
        imports.push(other);
        assert!(Contract::parse(&value.to_string()).is_err());
        value["imports"].as_array_mut().unwrap().pop();
        assert!(Contract::parse(&value.to_string()).is_err());
        assert!(Contract::parse(&MANIFEST.replace("abi = \"unit\"", "abi = \"i64\"")).is_err());
    }
    #[test]
    fn embedding_is_idempotent_and_rejects_mismatch_duplicate_and_malformed_sections() {
        let metadata = Contract::parse(MANIFEST).unwrap().metadata;
        let once = embed_metadata(EMPTY, metadata.as_bytes()).unwrap();
        assert_eq!(once, embed_metadata(&once, metadata.as_bytes()).unwrap());
        assert!(embed_metadata(&once, b"wrong contract").is_err());
        let mut duplicated = once.clone();
        duplicated.extend_from_slice(&once[EMPTY.len()..]);
        assert!(embed_metadata(&duplicated, metadata.as_bytes())
            .unwrap_err()
            .contains("duplicate"));
        assert!(embed_metadata(&once[..once.len() - 1], metadata.as_bytes()).is_err());
    }
    #[test]
    fn incidental_marker_and_metadata_bytes_do_not_count_as_an_abi_section() {
        let metadata = Contract::parse(MANIFEST).unwrap().metadata;
        let mut original = EMPTY.to_vec();
        let mut body = Vec::new();
        push_leb("debug".len(), &mut body);
        body.extend_from_slice(b"debug");
        body.extend_from_slice(METADATA_SECTION.as_bytes());
        body.extend_from_slice(metadata.as_bytes());
        original.push(0);
        push_leb(body.len(), &mut original);
        original.extend_from_slice(&body);
        let result = embed_metadata(&original, metadata.as_bytes()).unwrap();
        assert!(result.len() > original.len());
        assert_eq!(
            result,
            embed_metadata(&result, metadata.as_bytes()).unwrap()
        );
    }
}
