//! Private component encoding probe with opt-in compilation and execution checks.
use anyhow::{bail, ensure, Context, Result};
use std::io::{Read, Write};
use wasmparser::{Encoding, Parser, Payload, Validator, WasmFeatures};

const MAX_MODULE_BYTES: u64 = 32 * 1024 * 1024;

#[cfg(feature = "execution-probe")]
mod host;

#[cfg(feature = "execution-probe")]
mod hash_host;

#[cfg(feature = "engine-probe")]
fn compile_component(bytes: &[u8]) -> wasmtime::Result<()> {
    let mut config = wasmtime::Config::new();
    config.wasm_component_model_async(true);
    let engine = wasmtime::Engine::new(&config)?;
    wasmtime::component::Component::new(&engine, bytes)?;
    Ok(())
}

fn validate_component(bytes: &[u8]) -> Result<()> {
    Validator::new_with_features(WasmFeatures::all()).validate_all(bytes)?;
    let mut depth = 0_u32;
    let mut imports = std::collections::BTreeSet::new();
    for payload in Parser::new(0).parse_all(bytes) {
        match payload? {
            Payload::Version { encoding, .. } => {
                if depth == 0 {
                    ensure!(encoding == Encoding::Component, "expected a component");
                }
                depth += 1;
            }
            Payload::End(_) => depth -= 1,
            Payload::ComponentImportSection(section) if depth == 1 => {
                for import in section {
                    let import = import?;
                    ensure!(
                        matches!(
                            import.name.name,
                            "kernal:probe/blobs@0.1.0" | "kernal:hash-experiment/hashes@0.1.0"
                        ),
                        "non-kernel component import: {}",
                        import.name.name
                    );
                    ensure!(
                        matches!(import.ty, wasmparser::ComponentTypeRef::Instance(_)),
                        "expected the typed kernel interface"
                    );
                    ensure!(
                        imports.insert(import.name.name),
                        "duplicate kernel interface"
                    );
                }
            }
            _ => {}
        }
    }
    ensure!(
        imports.len() == 2,
        "expected exactly the blob and hash kernel interfaces"
    );
    Ok(())
}

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let input = args.next().context("expected core Wasm input path")?;
    let output = args.next().context("expected new component output path")?;
    ensure!(args.next().is_none(), "expected exactly two paths");
    let mut module = Vec::new();
    std::fs::File::open(input)?
        .take(MAX_MODULE_BYTES + 1)
        .read_to_end(&mut module)?;
    if module.len() as u64 > MAX_MODULE_BYTES {
        bail!("core Wasm exceeds the 32 MiB input limit");
    }
    let component = wit_component::ComponentEncoder::default()
        .module(&module)?
        .validate(true)
        .encode()?;
    validate_component(&component)?;
    #[cfg(feature = "engine-probe")]
    compile_component(&component).map_err(|error| anyhow::anyhow!("{error:#}"))?;
    #[cfg(feature = "execution-probe")]
    host::execute(&component).map_err(|error| anyhow::anyhow!("{error:#}"))?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    file.write_all(&component)?;
    #[cfg(not(feature = "execution-probe"))]
    println!(
        "validated component: {} bytes; two kernel imports; not executed",
        component.len()
    );
    #[cfg(all(feature = "engine-probe", not(feature = "execution-probe")))]
    println!("Wasmtime 45 component compilation passed; not instantiated or executed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "engine-probe")]
    #[test]
    fn engine_compilation_rejects_core_module_input() {
        assert!(compile_component(b"\0asm\x01\0\0\0").is_err());
    }

    #[test]
    fn ambient_and_other_kernel_interfaces_are_rejected() {
        for name in ["wasi:cli/run@0.2.0", "kernal:probe/other@0.1.0"] {
            let mut component = wasm_encoder::Component::new();
            let mut types = wasm_encoder::ComponentTypeSection::new();
            types.instance(&wasm_encoder::InstanceType::new());
            component.section(&types);
            let mut imports = wasm_encoder::ComponentImportSection::new();
            imports.import(name, wasm_encoder::ComponentTypeRef::Instance(0));
            component.section(&imports);
            let bytes = component.finish();
            Validator::new_with_features(WasmFeatures::all())
                .validate_all(&bytes)
                .unwrap();
            assert!(validate_component(&bytes)
                .unwrap_err()
                .to_string()
                .contains("non-kernel"));
        }
    }

    #[test]
    fn core_module_is_not_a_component() {
        assert!(validate_component(b"\0asm\x01\0\0\0").is_err());
    }

    #[test]
    fn empty_component_lacks_kernel_contract() {
        assert!(validate_component(b"\0asm\x0d\0\x01\0").is_err());
    }
}
#[cfg(test)]
#[path = "../../shared/rustc_policy.rs"]
mod rustc_policy;

#[test]
fn native_shared_rustc_policy() {
    assert!(rustc_policy::proof());
}
