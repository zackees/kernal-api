#[cfg(feature = "generators")]
use crate::types::CargoDependency;
#[cfg(feature = "generators")]
use crate::types::{Type, TypeIdent};
use crate::{functions::FunctionList, types::TypeMap};
#[cfg(feature = "generators")]
use std::collections::BTreeMap;
#[cfg(feature = "generators")]
use std::collections::BTreeSet;
#[cfg(feature = "generators")]
use std::fs;
use std::{error::Error, fmt::Display, io};

#[cfg(feature = "generators")]
pub mod rust_plugin;
#[cfg(feature = "generators")]
pub mod rust_wasmer2_runtime;
#[cfg(feature = "generators")]
pub mod rust_wasmer2_wasi_runtime;
#[cfg(feature = "generators")]
pub mod ts_runtime;
#[cfg(feature = "wasmtime-core-wasm")]
mod wasmtime_core_wasm;

#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum BindingsType {
    #[cfg(feature = "generators")]
    RustPlugin(RustPluginConfig),
    #[cfg(feature = "generators")]
    RustWasmer2Runtime,
    #[cfg(feature = "generators")]
    RustWasmer2WasiRuntime,
    #[cfg(feature = "wasmtime-core-wasm")]
    /// Generates the scalar-only `kernal-api:v1` Core Wasm ABI for Wasmtime 45 hosts.
    RustWasmtimeCoreWasm,
    #[cfg(feature = "generators")]
    TsRuntime(TsRuntimeConfig),
}

impl Display for BindingsType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            #[cfg(feature = "generators")]
            BindingsType::RustPlugin { .. } => "rust-plugin",
            #[cfg(feature = "generators")]
            BindingsType::RustWasmer2Runtime => "rust-wasmer2-runtime",
            #[cfg(feature = "generators")]
            BindingsType::RustWasmer2WasiRuntime => "rust-wasmer2-wasi-runtime",
            #[cfg(feature = "wasmtime-core-wasm")]
            BindingsType::RustWasmtimeCoreWasm => "rust-wasmtime-core-wasm",
            #[cfg(feature = "generators")]
            BindingsType::TsRuntime { .. } => "ts-runtime",
        })
    }
}

#[derive(Debug)]
pub struct BindingConfig<'a> {
    pub bindings_type: BindingsType,
    pub path: &'a str,
}

#[cfg(feature = "generators")]
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct RustPluginConfig {
    /// Name of the plugin crate that will be generated.
    pub name: Option<RustPluginConfigValue>,

    /// Authors to be listed in the plugin crate that will be generated.
    pub authors: Option<RustPluginConfigValue>,

    /// Version of the plugin crate that will be generated.
    pub version: Option<RustPluginConfigValue>,

    /// *Additional* dependencies to be listed in the plugin crate that will be
    /// generated.
    ///
    /// These are merged with a small set of dependencies that are necessary
    /// for the plugin to work and which will always be included. Specifying
    /// these dependencies yourself can be useful if you want to explicitly bump
    /// a dependency version or you want to enable a Cargo feature in them.
    pub dependencies: BTreeMap<String, CargoDependency>,

    /// The human-readable description for the generated crate.
    pub description: Option<RustPluginConfigValue>,

    /// A readme file containing some information for the generated crate.
    pub readme: Option<RustPluginConfigValue>,

    /// The license of the generated crate.
    pub license: Option<RustPluginConfigValue>,

    /// Whether the crate is marked as published or not (note: this should
    /// the string value "true" or "false")
    pub publish: Option<RustPluginConfigValue>,
}

#[cfg(feature = "generators")]
impl RustPluginConfig {
    pub fn builder() -> RustPluginConfigBuilder {
        RustPluginConfigBuilder {
            config: RustPluginConfig {
                name: None,
                authors: None,
                version: None,
                dependencies: Default::default(),
                description: None,
                readme: None,
                license: None,
                publish: None,
            },
        }
    }
}

#[cfg(feature = "generators")]
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum RustPluginConfigValue {
    String(String),
    Vec(Vec<String>),
    Workspace,
    Bool(bool),
}

#[cfg(feature = "generators")]
impl From<&str> for RustPluginConfigValue {
    fn from(value: &str) -> Self {
        Self::String(value.into())
    }
}

#[cfg(feature = "generators")]
impl From<String> for RustPluginConfigValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

#[cfg(feature = "generators")]
impl From<Vec<&str>> for RustPluginConfigValue {
    fn from(value: Vec<&str>) -> Self {
        Self::Vec(value.into_iter().map(|value| value.to_string()).collect())
    }
}

#[cfg(feature = "generators")]
impl From<Vec<String>> for RustPluginConfigValue {
    fn from(value: Vec<String>) -> Self {
        Self::Vec(value)
    }
}

#[cfg(feature = "generators")]
impl From<bool> for RustPluginConfigValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

#[cfg(feature = "generators")]
pub struct RustPluginConfigBuilder {
    config: RustPluginConfig,
}

#[cfg(feature = "generators")]
impl RustPluginConfigBuilder {
    pub fn name(mut self, value: impl Into<String>) -> Self {
        self.config.name = Some(RustPluginConfigValue::String(value.into()));
        self
    }

    pub fn version(mut self, value: impl Into<RustPluginConfigValue>) -> Self {
        self.config.version = Some(value.into());
        self
    }

    pub fn authors(mut self, value: impl Into<RustPluginConfigValue>) -> Self {
        self.config.authors = Some(value.into());
        self
    }

    pub fn author(mut self, value: impl Into<String>) -> Self {
        match &mut self.config.authors {
            Some(RustPluginConfigValue::Vec(vec)) => {
                vec.push(value.into());
            }
            None => {
                self.config.authors = Some(RustPluginConfigValue::Vec(vec![value.into()]));
            }
            _ => panic!("Cannot add an author to a non-vector 'authors' field"),
        }
        self
    }

    pub fn description(mut self, value: impl Into<RustPluginConfigValue>) -> Self {
        self.config.description = Some(value.into());
        self
    }

    pub fn readme(mut self, value: impl Into<RustPluginConfigValue>) -> Self {
        self.config.readme = Some(value.into());
        self
    }

    pub fn license(mut self, value: impl Into<RustPluginConfigValue>) -> Self {
        self.config.license = Some(value.into());
        self
    }

    pub fn dependencies<'a>(
        mut self,
        value: impl Into<BTreeMap<&'a str, CargoDependency>>,
    ) -> Self {
        let dependencies = value.into();
        self.config.dependencies = dependencies
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect();
        self
    }

    pub fn publish(mut self, value: bool) -> Self {
        self.config.publish = Some(value.into());
        self
    }

    pub fn dependency(mut self, name: impl Into<String>, dependency: CargoDependency) -> Self {
        self.config.dependencies.insert(name.into(), dependency);
        self
    }

    pub fn build(self) -> RustPluginConfig {
        assert!(
            self.config.name.is_some(),
            "'name' is required in RustPluginConfig"
        );
        self.config
    }
}

#[cfg(feature = "generators")]
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct TsRuntimeConfig {
    /// The module from which to import the MessagePack dependency.
    ///
    /// By default, "@msgpack/msgpack" is used, which should work with Node.js
    /// and most NPM-based bundlers. If you use Deno, you may wish to specify
    /// "https://unpkg.com/@msgpack/msgpack/mod.ts".
    pub msgpack_module: String,

    /// Whether or not to generate raw export wrappers.
    ///
    /// Raw export wrappers allow you to call `fp_export!` functions from the
    /// runtime while passing raw MessagePack data, which you can use in some
    /// situations to avoid (de)serialization overhead. If you don't need these
    /// wrappers, you can omit them to optimize your bundle size.
    ///
    /// Raw export wrappers are named similarly to the regular wrappers (which
    /// are generated in any case), but with a `Raw` suffix.
    pub generate_raw_export_wrappers: bool,

    /// Use `WebAssembly.instantiateStreaming()` instead of
    /// `WebAssembly.instantiate()` for optimizing instantiation in browser use
    /// cases. This changes the signature of the `createRuntime()` function to
    /// accept a `Response` instead of an `ArrayBuffer`.
    ///
    /// See also: https://developer.mozilla.org/en-US/docs/WebAssembly/JavaScript_interface/instantiateStreaming
    ///
    /// This setting is `true` by default, since MDN recommends it where
    /// possible. You may wish to opt-out if you're using the runtime in an
    /// environment that doesn't support streaming instantiation, such as
    /// Node.js.
    pub streaming_instantiation: bool,
}

#[cfg(feature = "generators")]
impl TsRuntimeConfig {
    /// Returns a new config instance with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the `msgpack_module` setting.
    pub fn with_msgpack_module(mut self, msgpack_module: &str) -> Self {
        msgpack_module.clone_into(&mut self.msgpack_module);
        self
    }

    /// Enables the `generate_raw_export_wrappers` setting.
    pub fn with_raw_export_wrappers(mut self) -> Self {
        self.generate_raw_export_wrappers = true;
        self
    }

    /// Disables the `streaming_instantiation` setting.
    pub fn without_streaming_instantiation(mut self) -> Self {
        self.streaming_instantiation = false;
        self
    }
}

#[cfg(feature = "generators")]
impl Default for TsRuntimeConfig {
    fn default() -> Self {
        Self {
            generate_raw_export_wrappers: false,
            msgpack_module: "@msgpack/msgpack".to_owned(),
            streaming_instantiation: true,
        }
    }
}

#[cfg(feature = "generators")]
impl TsRuntimeConfig {}

/// A validation or filesystem error from the scalar Wasmtime Core Wasm generator.
///
/// This generator intentionally rejects every value that would require the legacy
/// allocation, serialization, or async protocol before it creates an output directory.
#[non_exhaustive]
#[derive(Debug)]
pub enum WasmtimeCoreWasmError {
    AsyncFunction {
        direction: &'static str,
        function: String,
    },
    UnsupportedValue {
        direction: &'static str,
        function: String,
        position: String,
        ty: String,
    },
    UnsupportedTypeDefinition {
        name: String,
        ty: String,
    },
    Io {
        path: String,
        source: io::Error,
    },
}

impl Display for WasmtimeCoreWasmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AsyncFunction {
                direction,
                function,
            } => write!(
                f,
                "{direction} function `{function}` is async; the Wasmtime Core Wasm v0 ABI is synchronous"
            ),
            Self::UnsupportedValue {
                direction,
                function,
                position,
                ty,
            } => write!(
                f,
                "{direction} function `{function}` has unsupported {position} type `{ty}`; the Wasmtime Core Wasm v0 ABI accepts only scalar values"
            ),
            Self::UnsupportedTypeDefinition { name, ty } => write!(
                f,
                "type definition `{name}` (`{ty}`) is unsupported; the Wasmtime Core Wasm v0 ABI has no bulk or user-defined types"
            ),
            Self::Io { path, source } => write!(f, "could not write `{path}`: {source}"),
        }
    }
}

impl Error for WasmtimeCoreWasmError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Generates the scalar-only `kernal-api:v1` Core Wasm ABI and reports validation errors.
///
/// Unlike the historical binding targets, this entry point never falls back to a FatPtr or
/// serialization path. Invalid declarations are rejected before any output is written.
#[cfg(feature = "wasmtime-core-wasm")]
pub fn try_generate_wasmtime_core_wasm_bindings(
    import_functions: FunctionList,
    export_functions: FunctionList,
    types: TypeMap,
    path: &str,
) -> Result<(), WasmtimeCoreWasmError> {
    wasmtime_core_wasm::generate_bindings(import_functions, export_functions, types, path)
}

pub fn generate_bindings(
    import_functions: FunctionList,
    export_functions: FunctionList,
    types: TypeMap,
    config: BindingConfig,
) -> Result<(), WasmtimeCoreWasmError> {
    #[cfg(feature = "generators")]
    {
        fs::create_dir_all(config.path).expect("Could not create output directory");
        display_warnings(&import_functions, &export_functions, &types);
    }

    match config.bindings_type {
        #[cfg(feature = "wasmtime-core-wasm")]
        BindingsType::RustWasmtimeCoreWasm => try_generate_wasmtime_core_wasm_bindings(
            import_functions,
            export_functions,
            types,
            config.path,
        ),
        #[cfg(feature = "generators")]
        BindingsType::RustPlugin(plugin_config) => {
            rust_plugin::generate_bindings(
                import_functions,
                export_functions,
                types,
                plugin_config,
                config.path,
            );
            Ok(())
        }
        #[cfg(feature = "generators")]
        BindingsType::RustWasmer2Runtime => {
            rust_wasmer2_runtime::generate_bindings(
                import_functions,
                export_functions,
                types,
                config.path,
            );
            Ok(())
        }
        #[cfg(feature = "generators")]
        BindingsType::RustWasmer2WasiRuntime => {
            rust_wasmer2_wasi_runtime::generate_bindings(
                import_functions,
                export_functions,
                types,
                config.path,
            );
            Ok(())
        }
        #[cfg(feature = "generators")]
        BindingsType::TsRuntime(runtime_config) => {
            ts_runtime::generate_bindings(
                import_functions,
                export_functions,
                types,
                runtime_config,
                config.path,
            );
            Ok(())
        }
    }
}

#[cfg(feature = "generators")]
fn display_warnings(
    import_functions: &FunctionList,
    export_functions: &FunctionList,
    types: &TypeMap,
) {
    let all_functions = import_functions.iter().chain(export_functions.iter());
    let all_function_signature_types = all_functions.flat_map(|func| {
        func.args
            .iter()
            .map(|arg| &arg.ty)
            .chain(func.return_type.iter())
    });
    warn_about_custom_serializer_usage(
        all_function_signature_types.clone(),
        "function signature",
        types,
    );

    // Finding usages as generic arguments is more tricky, because we need to
    // find all places where generic arguments can be used.
    let all_idents = all_function_signature_types
        .chain(
            types
                .values()
                .filter_map(|ty| match ty {
                    Type::Struct(ty) => Some(ty),
                    _ => None,
                })
                .flat_map(|ty| ty.fields.iter().map(|field| &field.ty)),
        )
        .chain(
            types
                .values()
                .filter_map(|ty| match ty {
                    Type::Enum(ty) => Some(ty),
                    _ => None,
                })
                .flat_map(|ty| ty.variants.iter())
                .filter_map(|variant| match &variant.ty {
                    Type::Struct(ty) => Some(ty),
                    _ => None,
                })
                .flat_map(|ty| ty.fields.iter().map(|field| &field.ty)),
        );
    warn_about_custom_serializer_usage(
        all_idents.flat_map(|ident| ident.generic_args.iter().map(|(arg, _)| arg)),
        "generic argument",
        types,
    );
}

#[cfg(feature = "generators")]
fn warn_about_custom_serializer_usage<'a, T>(idents: T, context: &str, types: &TypeMap)
where
    T: Iterator<Item = &'a TypeIdent>,
{
    let mut idents_with_custom_serializers = BTreeSet::new();

    for ident in idents {
        let ty = types.get(ident);
        if let Some(Type::Custom(ty)) = ty {
            if ty.serde_attrs.iter().any(|attr| {
                attr.starts_with("with = ")
                    || attr.starts_with("serialize_with = ")
                    || attr.starts_with("deserialize_with = ")
            }) {
                idents_with_custom_serializers.insert(ident);
            }
        }
    }

    for ident in idents_with_custom_serializers {
        println!(
            "WARNING: Type `{ident}` is used directly in a {context}, but relies on a custom Serde \
            (de)serializer. This (de)serializer is NOT used when using the type directly \
            in a {context}. This may result in unexpected (de)serialization issues, for instance \
            when passing data between Rust and TypeScript.\n\
            You may wish to create a newtype to avoid this warning.\n\
            See `examples/example-protocol/src/types/time.rs` for an example."
        );
    }
}
