// Generated from kernal-api-v1.abi.toml; do not edit.
pub(crate) const ABI_NAMESPACE: &str = "kernal-api:v1";
pub(crate) const GENERATED_IMPORTS: &[GeneratedImport] = &[
    GeneratedImport { name: "abi_version", params: &[], results: &[ValType::I32] },
    GeneratedImport { name: "cancel", params: &[ValType::I64], results: &[ValType::I32] },
    GeneratedImport { name: "capability_bits", params: &[], results: &[ValType::I64] },
    GeneratedImport { name: "completion_word", params: &[ValType::I64, ValType::I32], results: &[ValType::I64] },
    GeneratedImport { name: "poll", params: &[ValType::I64], results: &[ValType::I32] },
    GeneratedImport { name: "release", params: &[ValType::I64], results: &[ValType::I32] },
    GeneratedImport { name: "submit", params: &[ValType::I32, ValType::I64, ValType::I64, ValType::I64], results: &[ValType::I64] },
    GeneratedImport { name: "yield_now", params: &[], results: &[ValType::I32] },
];
