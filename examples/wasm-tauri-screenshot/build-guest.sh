#!/usr/bin/env bash
set -euo pipefail

example_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd "$example_dir/../.." && pwd)"
: "${CARGO_TARGET_DIR:?set CARGO_TARGET_DIR to caller-managed writable storage}"
case "$CARGO_TARGET_DIR" in
  /*) guest_target_dir="${CARGO_TARGET_DIR%/}/kernal-api-wasm-tauri-guest" ;;
  *) echo "CARGO_TARGET_DIR must be absolute" >&2; exit 2 ;;
esac
target="wasm32-wasip1-threads"

# Use the same pinned, cache-disabled threaded build lane as the existing
# real Rust smoke artifact. Never substitute a hand-authored Wasm fixture.
(
  cd "$example_dir/guest"
  SOLDR_LINKER=default soldr --no-cache cargo build --locked \
    --manifest-path Cargo.toml --target "$target" --release \
    --target-dir "$guest_target_dir"
)
built="$guest_target_dir/$target/release/kernal-api-wasm-tauri-guest.wasm"
admitted="$guest_target_dir/$target/release/kernal-api-wasm-tauri-guest.admitted.wasm"
cp -- "$built" "$admitted"
soldr cargo run --locked --manifest-path "$repo_dir/tools/wasm-abi-generator/Cargo.toml" \
  -- --embed-threaded-metadata "$admitted"
printf '%s\n' "$admitted"
