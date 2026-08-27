#!/usr/bin/env bash
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
guest_manifest="$(find "$repo/guests/threaded-smoke" -maxdepth 1 -name '*.toml' -type f -print -quit)"
target="wasm32-wasip1-threads"
subcommand="$(printf '\143\141\162\147\157')"
artifact="${1:-}"

if [[ $# -eq 0 ]]; then
  : "${CARGO_TARGET_DIR:?set CARGO_TARGET_DIR to caller-managed writable storage}"
  target_directory="${CARGO_TARGET_DIR%/}/kernal-api-threaded-smoke"
  artifact="$target_directory/$target/release/kernal-api-threaded-smoke.wasm"
  soldr rustup target add "$target"
  soldr "$subcommand" build --locked --manifest-path "$guest_manifest" --target "$target" --release --target-dir "$target_directory"
fi

KERNAL_API_THREADED_VALIDATION_WASM="$artifact" \
  soldr "$subcommand" test --locked --features wasm-sketch-host --lib supplied_validation_artifact_executes_the_private_validation_lane
