//! Deterministic scalar-only Core Wasm bindings for a Wasmtime 45 host.
//!
//! This target is intentionally independent from the legacy Wasmer generators.
//! It emits source text only and never selects an allocation, serialization, or
//! async transport protocol.

use crate::{
    functions::{Function, FunctionList},
    generators::WasmtimeCoreWasmError,
    primitives::Primitive,
    types::{Type, TypeIdent, TypeMap},
};
use std::{collections::BTreeSet, fs, path::Path};

const ABI_NAMESPACE: &str = "kernal-api:v1";
const ABI_VERSION: u8 = 1;
const SCHEMA: &str = "fp-bindgen.core-wasm-abi";
const SCHEMA_REVISION: u8 = 1;
const GENERATOR_REVISION: u8 = 1;
const GUEST_CARGO_FILE: &str = "Cargo.toml";
const GUEST_SOURCE_FILE: &str = "src/lib.rs";
const HOST_LINKER_FILE: &str = "wasmtime45_host_linker.rs";
const MANIFEST_FILE: &str = "kernal-api-v1.abi.toml";

struct RenderedBindings {
    guest_cargo: String,
    guest_source: String,
    host_linker: String,
    manifest: String,
}

pub(crate) fn generate_bindings(
    import_functions: FunctionList,
    export_functions: FunctionList,
    types: TypeMap,
    path: &str,
) -> Result<(), WasmtimeCoreWasmError> {
    let rendered = render_bindings(&import_functions, &export_functions, &types)?;
    let output = Path::new(path);
    fs::create_dir_all(output.join("src")).map_err(|source| io_error(output, source))?;
    write_file(output.join(GUEST_CARGO_FILE), rendered.guest_cargo)?;
    write_file(output.join(GUEST_SOURCE_FILE), rendered.guest_source)?;
    write_file(output.join(HOST_LINKER_FILE), rendered.host_linker)?;
    write_file(output.join(MANIFEST_FILE), rendered.manifest)?;
    Ok(())
}

fn io_error(path: &Path, source: std::io::Error) -> WasmtimeCoreWasmError {
    WasmtimeCoreWasmError::Io {
        path: path.display().to_string(),
        source,
    }
}

fn write_file(path: impl AsRef<Path>, contents: String) -> Result<(), WasmtimeCoreWasmError> {
    let path = path.as_ref();
    // Generated files participate in kernal-api's checked-in drift gate.
    // Preserve one POSIX newline while avoiding an otherwise meaningless blank
    // line at EOF from assembled templates.
    let contents = format!("{}\n", contents.trim_end());
    fs::write(path, contents).map_err(|source| io_error(path, source))
}

fn render_bindings(
    import_functions: &FunctionList,
    export_functions: &FunctionList,
    types: &TypeMap,
) -> Result<RenderedBindings, WasmtimeCoreWasmError> {
    validate_functions(import_functions, "import")?;
    validate_functions(export_functions, "export")?;
    validate_type_definitions(types)?;
    Ok(RenderedBindings {
        guest_cargo: render_guest_cargo(),
        guest_source: render_guest_source(import_functions, export_functions),
        host_linker: render_host_linker(import_functions, export_functions),
        manifest: render_manifest(import_functions, export_functions),
    })
}

fn validate_functions(
    functions: &FunctionList,
    direction: &'static str,
) -> Result<(), WasmtimeCoreWasmError> {
    for function in functions {
        if function.is_async {
            return Err(WasmtimeCoreWasmError::AsyncFunction {
                direction,
                function: function.name.clone(),
            });
        }
        for argument in &function.args {
            lower_value_type(
                &argument.ty,
                direction,
                &function.name,
                format!("argument `{}`", argument.name),
            )?;
        }
        if let Some(return_type) = &function.return_type {
            lower_return_type(return_type, direction, &function.name)?;
        }
    }
    Ok(())
}

fn validate_type_definitions(types: &TypeMap) -> Result<(), WasmtimeCoreWasmError> {
    for (ident, ty) in types {
        match ty {
            Type::Primitive(primitive) if lower_primitive(*primitive).is_some() => {}
            Type::Unit => {}
            _ => {
                return Err(WasmtimeCoreWasmError::UnsupportedTypeDefinition {
                    name: ident.to_string(),
                    ty: ty.name(),
                });
            }
        }
    }
    Ok(())
}

fn lower_value_type(
    ty: &TypeIdent,
    direction: &'static str,
    function: &str,
    position: String,
) -> Result<LoweredType, WasmtimeCoreWasmError> {
    if ty.is_array() || !ty.generic_args.is_empty() || ty.name == "()" {
        return Err(unsupported_value(direction, function, position, ty));
    }
    ty.as_primitive()
        .and_then(lower_primitive)
        .ok_or_else(|| unsupported_value(direction, function, position, ty))
}

fn lower_return_type(
    ty: &TypeIdent,
    direction: &'static str,
    function: &str,
) -> Result<Option<LoweredType>, WasmtimeCoreWasmError> {
    if ty.name == "()" && !ty.is_array() && ty.generic_args.is_empty() {
        return Ok(None);
    }
    lower_value_type(ty, direction, function, "return".to_owned()).map(Some)
}

fn unsupported_value(
    direction: &'static str,
    function: &str,
    position: String,
    ty: &TypeIdent,
) -> WasmtimeCoreWasmError {
    WasmtimeCoreWasmError::UnsupportedValue {
        direction,
        function: function.to_owned(),
        position,
        ty: ty.to_string(),
    }
}

#[derive(Clone, Copy)]
struct LoweredType {
    semantic: &'static str,
    abi: &'static str,
}

fn lower_primitive(primitive: Primitive) -> Option<LoweredType> {
    use Primitive::*;
    Some(match primitive {
        Bool => LoweredType {
            semantic: "bool",
            abi: "i32",
        },
        I8 => LoweredType {
            semantic: "i8",
            abi: "i32",
        },
        I16 => LoweredType {
            semantic: "i16",
            abi: "i32",
        },
        I32 => LoweredType {
            semantic: "i32",
            abi: "i32",
        },
        U8 => LoweredType {
            semantic: "u8",
            abi: "i32",
        },
        U16 => LoweredType {
            semantic: "u16",
            abi: "i32",
        },
        U32 => LoweredType {
            semantic: "u32",
            abi: "i32",
        },
        I64 => LoweredType {
            semantic: "i64",
            abi: "i64",
        },
        U64 => LoweredType {
            semantic: "u64",
            abi: "i64",
        },
        F32 => LoweredType {
            semantic: "f32",
            abi: "f32",
        },
        F64 => LoweredType {
            semantic: "f64",
            abi: "f64",
        },
    })
}

fn lower_value_type_unchecked(ty: &TypeIdent) -> LoweredType {
    ty.as_primitive()
        .and_then(lower_primitive)
        .expect("Core Wasm declarations are validated before rendering")
}

fn lower_return_type_unchecked(ty: &TypeIdent) -> Option<LoweredType> {
    if ty.name == "()" && !ty.is_array() && ty.generic_args.is_empty() {
        None
    } else {
        Some(lower_value_type_unchecked(ty))
    }
}

fn render_guest_cargo() -> String {
    "[package]\n\
     name = \"kernal-api-v1-bindings\"\n\
     version = \"0.1.0\"\n\
     edition = \"2021\"\n\
     publish = false\n\n\
     [lib]\n\
     crate-type = [\"cdylib\", \"rlib\"]\n"
        .to_owned()
}

fn render_guest_source(import_functions: &FunctionList, export_functions: &FunctionList) -> String {
    let raw_imports = import_functions
        .iter()
        .map(render_guest_raw_import)
        .collect::<Vec<_>>()
        .join("\n");
    let imports = import_functions
        .iter()
        .map(render_guest_typed_import)
        .collect::<Vec<_>>()
        .join("\n\n");
    let export_fields = export_functions
        .iter()
        .map(render_guest_export_field)
        .collect::<Vec<_>>()
        .join("\n");
    let export_wrappers = export_functions
        .iter()
        .map(render_guest_export_wrapper)
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "// Generated scalar Core Wasm guest bindings for `{ABI_NAMESPACE}`.\n\
         // The public API uses semantic Rust scalar types; raw ABI values stay private.\n\n\
         use std::sync::OnceLock;\n\n\
         #[derive(Clone, Debug, Eq, PartialEq)]\n\
         pub enum AbiError {{\n\
         \u{20}   InvalidBoolean(i32),\n\
         \u{20}   OutOfRange {{ ty: &'static str, value: i32 }},\n\
         }}\n\n\
         #[derive(Clone, Debug, Eq, PartialEq)]\n\
         pub enum ExportInstallError {{\n\
         \u{20}   AlreadyInstalled,\n\
         }}\n\n\
         fn bool_from_i32(value: i32) -> Result<bool, AbiError> {{ match value {{ 0 => Ok(false), 1 => Ok(true), value => Err(AbiError::InvalidBoolean(value)) }} }}\n\
         fn i8_from_i32(value: i32) -> Result<i8, AbiError> {{ value.try_into().map_err(|_| AbiError::OutOfRange {{ ty: \"i8\", value }}) }}\n\
         fn i16_from_i32(value: i32) -> Result<i16, AbiError> {{ value.try_into().map_err(|_| AbiError::OutOfRange {{ ty: \"i16\", value }}) }}\n\
         fn u8_from_i32(value: i32) -> Result<u8, AbiError> {{ value.try_into().map_err(|_| AbiError::OutOfRange {{ ty: \"u8\", value }}) }}\n\
         fn u16_from_i32(value: i32) -> Result<u16, AbiError> {{ value.try_into().map_err(|_| AbiError::OutOfRange {{ ty: \"u16\", value }}) }}\n\n\
         fn i32_from_i32(value: i32) -> Result<i32, AbiError> {{ Ok(value) }}\n\
         fn u32_from_i32(value: i32) -> Result<u32, AbiError> {{ Ok(value as u32) }}\n\
         fn i64_from_i64(value: i64) -> Result<i64, AbiError> {{ Ok(value) }}\n\
         fn u64_from_i64(value: i64) -> Result<u64, AbiError> {{ Ok(value as u64) }}\n\
         fn f32_from_f32(value: f32) -> Result<f32, AbiError> {{ Ok(value) }}\n\
         fn f64_from_f64(value: f64) -> Result<f64, AbiError> {{ Ok(value) }}\n\
         fn bool_to_i32(value: bool) -> i32 {{ i32::from(value) }}\n\
         fn i8_to_i32(value: i8) -> i32 {{ value as i32 }}\n\
         fn i16_to_i32(value: i16) -> i32 {{ value as i32 }}\n\
         fn i32_to_i32(value: i32) -> i32 {{ value }}\n\
         fn u8_to_i32(value: u8) -> i32 {{ value as i32 }}\n\
         fn u16_to_i32(value: u16) -> i32 {{ value as i32 }}\n\
         fn u32_to_i32(value: u32) -> i32 {{ value as i32 }}\n\
         fn i64_to_i64(value: i64) -> i64 {{ value }}\n\
         fn u64_to_i64(value: u64) -> i64 {{ value as i64 }}\n\
         fn f32_to_f32(value: f32) -> f32 {{ value }}\n\
         fn f64_to_f64(value: f64) -> f64 {{ value }}\n\n\
         mod raw_imports {{\n\
         \u{20}   #[link(wasm_import_module = \"{ABI_NAMESPACE}\")]\n\
         \u{20}   extern \"C\" {{\n\
         {raw_imports}\n\
         \u{20}   }}\n\
         }}\n\n\
         pub mod imports {{\n\
         \u{20}   use super::*;\n\n\
         {imports}\n\
         }}\n\n\
         pub struct KernalApiV1Exports {{\n\
         {export_fields}\n\
         }}\n\n\
         static EXPORTS: OnceLock<KernalApiV1Exports> = OnceLock::new();\n\n\
         pub fn install_exports(exports: KernalApiV1Exports) -> Result<(), ExportInstallError> {{ EXPORTS.set(exports).map_err(|_| ExportInstallError::AlreadyInstalled) }}\n\n\
         fn installed_exports() -> &'static KernalApiV1Exports {{ EXPORTS.get().expect(\"install KernalApiV1Exports before invoking guest exports\") }}\n\
         fn require_abi<T>(value: Result<T, AbiError>) -> T {{ value.expect(\"host passed an invalid scalar Core Wasm ABI value\") }}\n\n\
         {export_wrappers}\n"
    )
}

fn render_guest_raw_import(function: &Function) -> String {
    format!(
        "        #[link_name = \"{}\"]\n        pub(super) fn {}({}){};",
        function.name,
        raw_import_name(function),
        render_abi_arguments(function),
        render_abi_result(function)
    )
}

fn render_guest_typed_import(function: &Function) -> String {
    let call_arguments = function
        .args
        .iter()
        .map(|argument| encode_expression(&argument.name, lower_value_type_unchecked(&argument.ty)))
        .collect::<Vec<_>>()
        .join(", ");
    let call = format!(
        "unsafe {{ raw_imports::{}({call_arguments}) }}",
        raw_import_name(function)
    );
    let body = match function
        .return_type
        .as_ref()
        .and_then(lower_return_type_unchecked)
    {
        Some(lowered) => format!(
            "let raw = {call};\n        {}",
            decode_expression("raw", lowered)
        ),
        None => format!("{call};\n        Ok(())"),
    };
    format!(
        "    pub fn {}({}) -> Result<{}, AbiError> {{\n        {body}\n    }}",
        function.name,
        render_semantic_arguments(function),
        semantic_return(function)
    )
}

fn render_guest_export_field(function: &Function) -> String {
    let arguments = function
        .args
        .iter()
        .map(|argument| lower_value_type_unchecked(&argument.ty).semantic)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "    pub {}: fn({arguments}) -> {},",
        function.name,
        semantic_return(function)
    )
}

fn render_guest_export_wrapper(function: &Function) -> String {
    let decoded = function
        .args
        .iter()
        .map(|argument| {
            format!(
                "let {} = require_abi({});",
                argument.name,
                decode_expression(&argument.name, lower_value_type_unchecked(&argument.ty))
            )
        })
        .collect::<Vec<_>>()
        .join("\n    ");
    let names = function
        .args
        .iter()
        .map(|argument| argument.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let result = function
        .return_type
        .as_ref()
        .and_then(lower_return_type_unchecked)
        .map(|lowered| format!("{}(result)", encode_function(lowered)))
        .unwrap_or_default();
    format!(
        "#[no_mangle]\npub extern \"C\" fn {}({}){} {{\n    {decoded}\n    let result = (installed_exports().{})({names});\n    {result}\n}}",
        function.name,
        render_abi_arguments(function),
        render_abi_result(function),
        function.name
    )
}

fn render_host_linker(import_functions: &FunctionList, export_functions: &FunctionList) -> String {
    let helpers = render_host_helpers(import_functions, export_functions);
    let trait_methods = import_functions
        .iter()
        .map(render_host_trait_method)
        .collect::<Vec<_>>()
        .join("\n");
    let registrations = import_functions
        .iter()
        .map(render_host_registration)
        .collect::<Vec<_>>()
        .join("\n");
    let invocations = export_functions
        .iter()
        .map(render_host_invocation)
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "// Generated private Wasmtime 45 glue for scalar Core Wasm ABI `{ABI_NAMESPACE}`.\n\
         // Host trait and invocation helpers use semantic Rust scalar types.\n\n\
         {helpers}\n\n\
         pub(crate) trait KernalApiV1Imports {{\n{trait_methods}\n}}\n\n\
         pub(crate) fn link_kernal_api_v1<T>(linker: &mut wasmtime::Linker<T>) -> wasmtime::Result<()>\n\
         where\n\
         \u{20}   T: KernalApiV1Imports + Send + 'static,\n\
         {{\n{registrations}\n    Ok(())\n}}\n\n\
         {invocations}\n"
    )
}

fn render_host_helpers(import_functions: &FunctionList, export_functions: &FunctionList) -> String {
    let mut used = BTreeSet::new();
    for function in import_functions.iter().chain(export_functions.iter()) {
        for argument in &function.args {
            used.insert(lower_value_type_unchecked(&argument.ty).semantic);
        }
        if let Some(return_type) = &function.return_type {
            if let Some(lowered) = lower_return_type_unchecked(return_type) {
                used.insert(lowered.semantic);
            }
        }
    }
    [
        "bool", "i8", "i16", "i32", "u8", "u16", "u32", "i64", "u64", "f32", "f64",
    ]
    .iter()
    .filter(|semantic| used.contains(*semantic))
    .map(|semantic| host_helper_source(semantic))
    .collect::<Vec<_>>()
    .join("\n")
}

fn host_helper_source(semantic: &str) -> &'static str {
    match semantic {
        "bool" => "fn bool_from_i32(value: i32) -> wasmtime::Result<bool> {\n    match value {\n        0 => Ok(false),\n        1 => Ok(true),\n        value => Err(wasmtime::Error::msg(format!(\n            \"invalid bool ABI value: {value}\"\n        ))),\n    }\n}\nfn bool_to_i32(value: bool) -> i32 {\n    i32::from(value)\n}",
        "i8" => "fn i8_from_i32(value: i32) -> wasmtime::Result<i8> {\n    value\n        .try_into()\n        .map_err(|_| wasmtime::Error::msg(format!(\"invalid i8 ABI value: {value}\")))\n}\nfn i8_to_i32(value: i8) -> i32 {\n    value as i32\n}",
        "i16" => "fn i16_from_i32(value: i32) -> wasmtime::Result<i16> {\n    value\n        .try_into()\n        .map_err(|_| wasmtime::Error::msg(format!(\"invalid i16 ABI value: {value}\")))\n}\nfn i16_to_i32(value: i16) -> i32 {\n    value as i32\n}",
        "i32" => "fn i32_from_i32(value: i32) -> wasmtime::Result<i32> {\n    Ok(value)\n}\nfn i32_to_i32(value: i32) -> i32 {\n    value\n}",
        "u8" => "fn u8_from_i32(value: i32) -> wasmtime::Result<u8> {\n    value\n        .try_into()\n        .map_err(|_| wasmtime::Error::msg(format!(\"invalid u8 ABI value: {value}\")))\n}\nfn u8_to_i32(value: u8) -> i32 {\n    value as i32\n}",
        "u16" => "fn u16_from_i32(value: i32) -> wasmtime::Result<u16> {\n    value\n        .try_into()\n        .map_err(|_| wasmtime::Error::msg(format!(\"invalid u16 ABI value: {value}\")))\n}\nfn u16_to_i32(value: u16) -> i32 {\n    value as i32\n}",
        "u32" => "fn u32_from_i32(value: i32) -> wasmtime::Result<u32> {\n    Ok(value as u32)\n}\nfn u32_to_i32(value: u32) -> i32 {\n    value as i32\n}",
        "i64" => "fn i64_from_i64(value: i64) -> wasmtime::Result<i64> {\n    Ok(value)\n}\nfn i64_to_i64(value: i64) -> i64 {\n    value\n}",
        "u64" => "fn u64_from_i64(value: i64) -> wasmtime::Result<u64> {\n    Ok(value as u64)\n}\nfn u64_to_i64(value: u64) -> i64 {\n    value as i64\n}",
        "f32" => "fn f32_from_f32(value: f32) -> wasmtime::Result<f32> {\n    Ok(value)\n}\nfn f32_to_f32(value: f32) -> f32 {\n    value\n}",
        "f64" => "fn f64_from_f64(value: f64) -> wasmtime::Result<f64> {\n    Ok(value)\n}\nfn f64_to_f64(value: f64) -> f64 {\n    value\n}",
        _ => unreachable!("scalar Core Wasm lowering table is closed"),
    }
}

fn render_host_trait_method(function: &Function) -> String {
    let arguments = render_semantic_arguments(function);
    let arguments = if arguments.is_empty() {
        String::new()
    } else {
        format!(", {arguments}")
    };
    format!(
        "    fn {}(&mut self{arguments}) -> wasmtime::Result<{}>;",
        function.name,
        semantic_return(function)
    )
}

fn render_host_registration(function: &Function) -> String {
    let decoded = function
        .args
        .iter()
        .map(|argument| {
            format!(
                "let {} = {}?;",
                argument.name,
                decode_expression(&argument.name, lower_value_type_unchecked(&argument.ty))
            )
        })
        .collect::<Vec<_>>()
        .join("\n            ");
    let decoded = if decoded.is_empty() {
        String::new()
    } else {
        format!("{decoded}\n            ")
    };
    let names = function
        .args
        .iter()
        .map(|argument| argument.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let result = match function
        .return_type
        .as_ref()
        .and_then(lower_return_type_unchecked)
    {
        Some(lowered) => format!(
            "Ok({}(caller.data_mut().{}({names})?))",
            encode_function(lowered),
            function.name
        ),
        None => format!(
            "caller.data_mut().{}({names})?;\n            Ok(())",
            function.name
        ),
    };
    let abi_arguments = render_abi_arguments(function);
    let abi_arguments = if abi_arguments.is_empty() {
        String::new()
    } else {
        format!(", {abi_arguments}")
    };
    let closure = if abi_arguments.is_empty() {
        format!(
            "|mut caller: wasmtime::Caller<'_, T>| -> wasmtime::Result<{}> {{\n            {result}\n        }}",
            render_abi_result_only(function)
        )
    } else {
        let arguments = abi_arguments
            .trim_start_matches(", ")
            .replace(", ", ",\n         ");
        format!(
            "|mut caller: wasmtime::Caller<'_, T>,\n         {arguments}|\n         -> wasmtime::Result<{}> {{\n            {decoded}{result}\n        }}",
            render_abi_result_only(function)
        )
    };
    format!(
        "    linker.func_wrap(\n        \"{ABI_NAMESPACE}\",\n        \"{}\",\n        {closure},\n    )?;",
        function.name
    )
}

fn render_host_invocation(function: &Function) -> String {
    let call_arguments = render_wasm_call_params(function);
    let result = match function
        .return_type
        .as_ref()
        .and_then(lower_return_type_unchecked)
    {
        Some(lowered) => decode_expression("raw", lowered),
        None => "Ok(())".to_owned(),
    };
    let arguments = render_semantic_arguments(function);
    let arguments = if arguments.is_empty() {
        String::new()
    } else {
        format!(",\n    {}", arguments.replace(", ", ",\n    "))
    };
    format!(
        "pub(crate) fn invoke_{}<T>(\n    store: &mut wasmtime::Store<T>,\n    instance: &wasmtime::Instance{arguments},\n) -> wasmtime::Result<{}> {{\n    let function = instance.get_typed_func::<{}, {}>(&mut *store, \"{}\")?;\n    let raw = function.call(&mut *store, {call_arguments})?;\n    {result}\n}}",
        function.name,
        semantic_return(function),
        render_wasm_function_params(function),
        render_abi_result_only(function),
        function.name
    )
}

fn render_manifest(import_functions: &FunctionList, export_functions: &FunctionList) -> String {
    let imports = import_functions
        .iter()
        .map(|function| render_manifest_record("imports", "guest-to-host", function))
        .collect::<Vec<_>>()
        .join("\n");
    let exports = export_functions
        .iter()
        .map(|function| render_manifest_record("exports", "host-to-guest", function))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "schema = \"{SCHEMA}\"\nschema_revision = {SCHEMA_REVISION}\ngenerator_revision = {GENERATOR_REVISION}\nabi_version = {ABI_VERSION}\nnamespace = \"{ABI_NAMESPACE}\"\n\n{imports}\n{exports}"
    )
}

fn render_manifest_record(table: &str, direction: &str, function: &Function) -> String {
    let params = function
        .args
        .iter()
        .map(|argument| {
            let lowered = lower_value_type_unchecked(&argument.ty);
            format!(
                "{{ semantic = \"{}\", abi = \"{}\" }}",
                lowered.semantic, lowered.abi
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let results = function
        .return_type
        .as_ref()
        .and_then(lower_return_type_unchecked)
        .map(|lowered| {
            format!(
                "{{ semantic = \"{}\", abi = \"{}\" }}",
                lowered.semantic, lowered.abi
            )
        })
        .unwrap_or_else(|| "{ semantic = \"()\", abi = \"unit\" }".to_owned());
    format!(
        "[[{table}]]\nnamespace = \"{ABI_NAMESPACE}\"\nname = \"{}\"\ndirection = \"{direction}\"\nparams = [{params}]\nresults = [{results}]\n",
        function.name
    )
}

fn raw_import_name(function: &Function) -> String {
    format!("__kernal_api_v1_import_{}", function.name)
}

fn render_semantic_arguments(function: &Function) -> String {
    function
        .args
        .iter()
        .map(|argument| {
            format!(
                "{}: {}",
                argument.name,
                lower_value_type_unchecked(&argument.ty).semantic
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_abi_arguments(function: &Function) -> String {
    render_abi_arguments_only(function)
}

fn render_abi_arguments_only(function: &Function) -> String {
    function
        .args
        .iter()
        .map(|argument| {
            format!(
                "{}: {}",
                argument.name,
                lower_value_type_unchecked(&argument.ty).abi
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_wasm_function_params(function: &Function) -> String {
    let params = function
        .args
        .iter()
        .map(|argument| lower_value_type_unchecked(&argument.ty).abi)
        .collect::<Vec<_>>();
    match params.as_slice() {
        [] => "()".to_owned(),
        [one] => (*one).to_owned(),
        many => format!("({})", many.join(", ")),
    }
}

fn render_wasm_call_params(function: &Function) -> String {
    let params = function
        .args
        .iter()
        .map(|argument| encode_expression(&argument.name, lower_value_type_unchecked(&argument.ty)))
        .collect::<Vec<_>>();
    match params.as_slice() {
        [] => "()".to_owned(),
        [one] => one.clone(),
        many => format!("({})", many.join(", ")),
    }
}

fn render_abi_result(function: &Function) -> String {
    format!(" -> {}", render_abi_result_only(function))
}

fn render_abi_result_only(function: &Function) -> &'static str {
    function
        .return_type
        .as_ref()
        .and_then(lower_return_type_unchecked)
        .map(|lowered| lowered.abi)
        .unwrap_or("()")
}

fn semantic_return(function: &Function) -> &'static str {
    function
        .return_type
        .as_ref()
        .and_then(lower_return_type_unchecked)
        .map(|lowered| lowered.semantic)
        .unwrap_or("()")
}

fn encode_expression(name: &str, lowered: LoweredType) -> String {
    format!("{}({name})", encode_function(lowered))
}

fn encode_function(lowered: LoweredType) -> &'static str {
    match lowered.semantic {
        "bool" => "bool_to_i32",
        "i8" => "i8_to_i32",
        "i16" => "i16_to_i32",
        "i32" => "i32_to_i32",
        "u8" => "u8_to_i32",
        "u16" => "u16_to_i32",
        "u32" => "u32_to_i32",
        "i64" => "i64_to_i64",
        "u64" => "u64_to_i64",
        "f32" => "f32_to_f32",
        "f64" => "f64_to_f64",
        _ => unreachable!("closed lowering table"),
    }
}

fn decode_expression(name: &str, lowered: LoweredType) -> String {
    let function = match lowered.semantic {
        "bool" => "bool_from_i32",
        "i8" => "i8_from_i32",
        "i16" => "i16_from_i32",
        "i32" => "i32_from_i32",
        "u8" => "u8_from_i32",
        "u16" => "u16_from_i32",
        "u32" => "u32_from_i32",
        "i64" => "i64_from_i64",
        "u64" => "u64_from_i64",
        "f32" => "f32_from_f32",
        "f64" => "f64_from_f64",
        _ => unreachable!("closed lowering table"),
    };
    format!("{function}({name})")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar_functions() -> (FunctionList, FunctionList) {
        let mut imports = FunctionList::new();
        imports
            .add_function("fn zeta(flag: bool, count: i32, total: u64, ratio: f32, precise: f64);");
        imports.add_function("fn alpha() -> bool;");
        let mut exports = FunctionList::new();
        exports.add_function("fn guest_value(value: u64) -> f64;");
        (imports, exports)
    }

    #[test]
    fn scalar_fixture_is_typed_bidirectional_and_free_of_legacy_runtime_words() {
        let (imports, exports) = scalar_functions();
        let rendered = render_bindings(&imports, &exports, &TypeMap::new()).unwrap();
        assert!(rendered
            .guest_cargo
            .contains("crate-type = [\"cdylib\", \"rlib\"]"));
        assert!(rendered.guest_source.contains("pub fn zeta(flag: bool, count: i32, total: u64, ratio: f32, precise: f64) -> Result<(), AbiError>"));
        assert!(rendered
            .guest_source
            .contains("#[no_mangle]\npub extern \"C\" fn guest_value(value: i64) -> f64"));
        assert!(rendered.host_linker.contains("fn zeta(&mut self, flag: bool, count: i32, total: u64, ratio: f32, precise: f64) -> wasmtime::Result<()>"));
        assert!(rendered
            .host_linker
            .contains("T: KernalApiV1Imports + Send + 'static"));
        assert!(rendered.host_linker.contains("invoke_guest_value"));
        for legacy in ["FatPtr", "MessagePack", "Wasmer", "Tokio", "rmp"] {
            assert!(!rendered.guest_source.contains(legacy));
            assert!(!rendered.host_linker.contains(legacy));
            assert!(!rendered.manifest.contains(legacy));
        }
    }

    #[test]
    fn manifest_is_parseable_toml_with_canonical_sorted_records() {
        let (imports, exports) = scalar_functions();
        let first = render_bindings(&imports, &exports, &TypeMap::new()).unwrap();
        let manifest: toml::Value = toml::from_str(&first.manifest).unwrap();
        assert_eq!(manifest["schema"].as_str(), Some(SCHEMA));
        assert_eq!(manifest["namespace"].as_str(), Some(ABI_NAMESPACE));
        assert_eq!(manifest["imports"][0]["name"].as_str(), Some("alpha"));
        assert_eq!(
            manifest["imports"][1]["params"][2]["semantic"].as_str(),
            Some("u64")
        );
        assert_eq!(
            first.manifest,
            r#"schema = "fp-bindgen.core-wasm-abi"
schema_revision = 1
generator_revision = 1
abi_version = 1
namespace = "kernal-api:v1"

[[imports]]
namespace = "kernal-api:v1"
name = "alpha"
direction = "guest-to-host"
params = []
results = [{ semantic = "bool", abi = "i32" }]

[[imports]]
namespace = "kernal-api:v1"
name = "zeta"
direction = "guest-to-host"
params = [{ semantic = "bool", abi = "i32" }, { semantic = "i32", abi = "i32" }, { semantic = "u64", abi = "i64" }, { semantic = "f32", abi = "f32" }, { semantic = "f64", abi = "f64" }]
results = [{ semantic = "()", abi = "unit" }]

[[exports]]
namespace = "kernal-api:v1"
name = "guest_value"
direction = "host-to-guest"
params = [{ semantic = "u64", abi = "i64" }]
results = [{ semantic = "f64", abi = "f64" }]
"#
        );
        let mut reordered = FunctionList::new();
        reordered.add_function("fn alpha() -> bool;");
        reordered
            .add_function("fn zeta(flag: bool, count: i32, total: u64, ratio: f32, precise: f64);");
        let second = render_bindings(&reordered, &exports, &TypeMap::new()).unwrap();
        assert_eq!(first.manifest, second.manifest);
        assert_eq!(first.guest_source, second.guest_source);
        assert_eq!(first.host_linker, second.host_linker);
    }

    #[test]
    fn invalid_bool_and_narrow_values_use_checked_decoder_paths() {
        let (imports, exports) = scalar_functions();
        let rendered = render_bindings(&imports, &exports, &TypeMap::new()).unwrap();
        assert!(rendered
            .guest_source
            .contains("match value { 0 => Ok(false), 1 => Ok(true)"));
        let mut narrow_imports = FunctionList::new();
        narrow_imports.add_function("fn narrow(value: u8) -> u16;");
        let narrow =
            render_bindings(&narrow_imports, &FunctionList::new(), &TypeMap::new()).unwrap();
        assert!(narrow
            .host_linker
            .contains("u16_from_i32(value: i32) -> wasmtime::Result<u16>"));
        assert!(rendered.guest_source.contains("u64_to_i64"));
        assert!(rendered.host_linker.contains("u64_from_i64"));
    }

    #[test]
    fn non_scalar_and_async_values_are_rejected_before_output() {
        for declaration in [
            "fn takes_string(value: String);",
            "fn takes_array(value: [u8; 4]);",
            "fn takes_list(value: Vec<u8>);",
            "fn takes_result(value: Result<u32, u32>);",
            "fn takes_struct(value: Payload);",
        ] {
            let mut imports = FunctionList::new();
            imports.add_function(declaration);
            assert!(matches!(
                render_bindings(&imports, &FunctionList::new(), &TypeMap::new()),
                Err(WasmtimeCoreWasmError::UnsupportedValue { .. })
            ));
        }
        let mut async_exports = FunctionList::new();
        async_exports.add_function("async fn later() -> i32;");
        assert!(matches!(
            render_bindings(&FunctionList::new(), &async_exports, &TypeMap::new()),
            Err(WasmtimeCoreWasmError::AsyncFunction { .. })
        ));
    }

    #[test]
    fn target_feature_is_dependency_free_and_default_isolated() {
        let cargo: toml::Value = include_str!("../../Cargo.toml").parse().unwrap();
        let features = cargo["features"].as_table().unwrap();
        let dependencies = cargo["dependencies"].as_table().unwrap();

        assert!(features["wasmtime-core-wasm"]
            .as_array()
            .unwrap()
            .is_empty());
        let integration = features["wasmtime45-integration"].as_array().unwrap();
        assert_eq!(integration.len(), 2);
        assert_eq!(integration[0].as_str(), Some("wasmtime-core-wasm"));
        assert_eq!(integration[1].as_str(), Some("dep:wasmtime"));
        assert!(dependencies["wasmtime"]["optional"].as_bool().unwrap());
        assert_eq!(dependencies["wasmtime"]["version"].as_str(), Some("45"));
        assert!(!features["default"]
            .as_array()
            .unwrap()
            .iter()
            .any(|feature| feature
                .as_str()
                .is_some_and(|feature| feature.contains("wasmtime"))));
        assert!(!dependencies.contains_key("wasmer"));
        assert!(!dependencies.contains_key("tokio"));
        assert!(!dependencies.contains_key("fp-bindgen-support"));
    }

    #[test]
    fn wasmtime45_compile_fixture_is_exact_generated_host_golden() {
        let mut imports = FunctionList::new();
        imports.add_function("fn checked(flag: bool, tiny: u8, total: u64) -> bool;");
        imports.add_function("fn reset();");
        let mut exports = FunctionList::new();
        exports.add_function("fn guest_measure(value: i16, ratio: f32) -> u16;");
        exports.add_function("fn guest_unit();");
        let rendered = render_bindings(&imports, &exports, &TypeMap::new()).unwrap();
        assert_eq!(
            rendered.host_linker,
            include_str!("../../tests/fixtures/wasmtime45_scalar_host.rs")
        );
    }
}
