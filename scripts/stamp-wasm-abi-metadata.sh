#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <wasm-file>" >&2
  exit 2
fi
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
soldr cargo run --locked --manifest-path "$root/tools/wasm-abi-gen/Cargo.toml" -- --stamp "$1"
