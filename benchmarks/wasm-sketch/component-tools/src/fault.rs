//! Explicit artifact-only fault injection; never used by the normal encoder.
use super::*;

pub(super) fn trap_realloc(module: &mut [u8]) -> Result<()> {
    let mut imported = 0_u32;
    let mut target = None;
    let mut body_index = 0_u32;
    let mut replacement = None;
    for payload in Parser::new(0).parse_all(module) {
        match payload? {
            Payload::Version { encoding, .. } => {
                ensure!(encoding == Encoding::Module, "expected core module")
            }
            Payload::ImportSection(section) => {
                for import in section.into_imports() {
                    if matches!(
                        import?.ty,
                        wasmparser::TypeRef::Func(_) | wasmparser::TypeRef::FuncExact(_)
                    ) {
                        imported += 1;
                    }
                }
            }
            Payload::ExportSection(section) => {
                for export in section {
                    let export = export?;
                    if export.name == "cabi_realloc" {
                        ensure!(
                            export.kind == wasmparser::ExternalKind::Func,
                            "realloc must be a function"
                        );
                        target = Some(export.index);
                    }
                }
            }
            Payload::CodeSectionEntry(body) => {
                if target == Some(imported + body_index) {
                    replacement = Some(body.range());
                }
                body_index += 1;
            }
            _ => {}
        }
    }
    let range = replacement.context("defined cabi_realloc export required")?;
    ensure!(range.len() >= 3, "realloc body too short");
    // Preserve section sizes and every other byte, including component metadata.
    // Empty locals, unreachable, padding nops, end: valid for any result type.
    let body = &mut module[range];
    body.fill(0x01);
    body[0] = 0;
    body[1] = 0;
    body[body.len() - 1] = 0x0b;
    Validator::new_with_features(WasmFeatures::all()).validate_all(module)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn realloc_fault_handles_imported_function_indices_and_preserves_metadata() {
        use wasm_encoder::*;
        let mut module = Module::new();
        let mut types = TypeSection::new();
        types.ty().function([], []);
        module.section(&types);
        let mut imports = ImportSection::new();
        imports.import("test", "unused", EntityType::Function(0));
        module.section(&imports);
        let mut functions = FunctionSection::new();
        functions.function(0);
        module.section(&functions);
        let mut exports = ExportSection::new();
        exports.export("cabi_realloc", ExportKind::Func, 1);
        module.section(&exports);
        let mut code = CodeSection::new();
        let mut function = Function::new([]);
        function
            .instruction(&Instruction::Nop)
            .instruction(&Instruction::End);
        code.function(&function);
        module.section(&code);
        module.section(&CustomSection {
            name: "fixture-metadata".into(),
            data: b"unchanged".as_slice().into(),
        });
        let mut bytes = module.finish();
        let original = bytes.clone();
        trap_realloc(&mut bytes).unwrap();
        assert_eq!(bytes.len(), original.len());
        let differences: Vec<_> = bytes
            .iter()
            .zip(&original)
            .filter(|(a, b)| a != b)
            .collect();
        assert_eq!(
            differences,
            vec![(&0, &1)],
            "only the original nop becomes unreachable"
        );
    }

    #[test]
    fn missing_realloc_is_rejected_without_mutation() {
        let mut bytes = b"\0asm\x01\0\0\0".to_vec();
        let original = bytes.clone();
        assert!(trap_realloc(&mut bytes).is_err());
        assert_eq!(bytes, original);
    }
}
