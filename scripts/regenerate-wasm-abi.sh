#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output="$root/abi/generated"
check=false
if [[ "${1:-}" == "--check" && $# -eq 1 ]]; then
  check=true
elif [[ $# -ne 0 ]]; then
  echo "usage: $0 [--check]" >&2
  exit 2
fi
tmp="$(mktemp -d "${TMPDIR:-/tmp}/kernal-api-abi.XXXXXX")"
previous=""
cleanup() {
  [[ -z "$tmp" ]] || rm -rf "$tmp"
  [[ -z "$previous" ]] || rm -rf "$previous"
}
trap cleanup EXIT

soldr cargo run --locked --manifest-path "$root/tools/wasm-abi-gen/Cargo.toml" -- "$tmp"
if "$check"; then
  diff -ruN "$output" "$tmp"
else
  # `mv` makes the generated tree exactly match a clean generation: obsolete
  # files cannot survive an overlay copy and make a later drift check fail.
  if [[ -e "$output" ]]; then
    previous="$(mktemp -d "${TMPDIR:-/tmp}/kernal-api-abi-previous.XXXXXX")"
    rmdir "$previous"
    mv "$output" "$previous"
  fi
  mv "$tmp" "$output"
  tmp=""
fi
