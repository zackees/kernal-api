#!/usr/bin/env bash
set -euo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target="wasm32-wasip1-threads"
artifact="${1:-}"
if [[ -z "$artifact" ]]; then
  : "${CARGO_TARGET_DIR:?set CARGO_TARGET_DIR to caller-managed writable storage}"
  artifact="$CARGO_TARGET_DIR/generated-core-smoke/$target/release/kernal-api-generated-core-smoke.wasm"
  soldr --no-cache rustup target add "$target"
  (
    cd "$repo/guests/generated-core-smoke"
    SOLDR_LINKER=default soldr --no-cache cargo build --locked --release --target "$target" \
      --target-dir "$CARGO_TARGET_DIR/generated-core-smoke"
  )
  bash "$repo/scripts/stamp-wasm-abi-metadata.sh" "$artifact"
fi
KERNAL_API_GENERATED_ARTIFACT_WASM="$artifact" soldr cargo test --locked \
  --manifest-path "$repo/Cargo.toml" --features wasm-sketch-host \
  --test generated_core_abi supplied_generated_guest_admits_and_executes -- --ignored --nocapture
