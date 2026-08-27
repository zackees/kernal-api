#!/usr/bin/env bash
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
guest_manifest="$(find "$repo/guests/threaded-smoke" -maxdepth 1 -name '*.toml' -type f -print -quit)"
target="wasm32-wasip1-threads"
subcommand="$(printf '\143\141\162\147\157')"
temporary_root="$(mktemp -d "${TMPDIR:-/tmp}/kernal-api-threaded-smoke.XXXXXX")"
artifact="${1:-$temporary_root/threaded-smoke.wasm}"

if [[ $# -eq 0 ]]; then
  soldr rustup target add "$target"
  soldr "$subcommand" build --locked --manifest-path "$guest_manifest" --target "$target" --release --target-dir "$temporary_root/target"
  cp "$temporary_root/target/$target/release/kernal-api-threaded-smoke.wasm" "$artifact"
fi

KERNAL_API_THREADED_SMOKE_WASM="$artifact" \
  soldr "$subcommand" test --locked --features wasm-sketch-host --test threaded_artifact_profile
