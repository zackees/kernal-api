#!/usr/bin/env bash
set -euo pipefail

example_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd "$example_dir/../.." && pwd)"
: "${CARGO_TARGET_DIR:?set CARGO_TARGET_DIR to caller-managed writable storage}"
guest_name="kernal-api-wasm-tauri-guest"
guest_feature=""
case "${1-}" in
  "") ;;
  --trap-after-capture) guest_name="${guest_name}-trap"; guest_feature=proof-trap-after-capture ;;
  --block-after-capture) guest_name="${guest_name}-block"; guest_feature=proof-block-after-capture ;;
  *) echo "usage: build-guest.sh [--trap-after-capture|--block-after-capture]" >&2; exit 2 ;;
esac
if [ "$#" -gt 1 ]; then echo "too many arguments" >&2; exit 2; fi
case "$CARGO_TARGET_DIR" in
  /*) guest_target_dir="${CARGO_TARGET_DIR%/}/$guest_name" ;;
  *) echo "CARGO_TARGET_DIR must be absolute" >&2; exit 2 ;;
esac
target="wasm32-wasip1-threads"

# Use the same pinned, cache-disabled threaded build lane as the existing
# real Rust smoke artifact. Never substitute a hand-authored Wasm fixture.
(
  cd "$example_dir/guest"
  set -- --no-cache cargo build --locked \
    --manifest-path Cargo.toml --target "$target" --release \
    --target-dir "$guest_target_dir"
  # Bash 3.2 treats an empty array as unset under nounset. Positional
  # arguments preserve exact quoting without relying on that behavior.
  if [ -n "$guest_feature" ]; then set -- "$@" --features "$guest_feature"; fi
  SOLDR_LINKER=default soldr "$@"
)
built="$guest_target_dir/$target/release/kernal-api-wasm-tauri-guest.wasm"
admitted="$guest_target_dir/$target/release/kernal-api-wasm-tauri-guest.admitted.wasm"
cp -- "$built" "$admitted"
soldr cargo run --locked --manifest-path "$repo_dir/tools/wasm-abi-generator/Cargo.toml" \
  -- --embed-threaded-metadata "$admitted"
printf '%s\n' "$admitted"
