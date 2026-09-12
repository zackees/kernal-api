#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
generated_dir="${repo_root}/src/wasm/generated/v1"
scratch_root="$(mktemp -d)"
trap 'rm -rf "${scratch_root}"' EXIT

# Explicit output redirection makes drift verification read-only for the
# checked-in artifacts.
KERNAL_API_ABI_OUTPUT="${scratch_root}/generated" \
  soldr cargo run --locked --manifest-path "${repo_root}/tools/wasm-abi-generator/Cargo.toml"
diff -ru -x target "${generated_dir}" "${scratch_root}/generated"
