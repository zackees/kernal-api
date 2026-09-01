#!/usr/bin/env bash
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
guest_dir="$repo/guests/threaded-smoke"
guest_manifest="$guest_dir/Cargo.toml"
target="wasm32-wasip1-threads"
subcommand="$(printf '\143\141\162\147\157')"
artifact="${1:-}"

if [[ $# -eq 0 ]]; then
  : "${CARGO_TARGET_DIR:?set CARGO_TARGET_DIR to caller-managed writable storage}"
  target_directory="${CARGO_TARGET_DIR%/}/kernal-api-threaded-smoke"
  artifact="$target_directory/$target/release/kernal-api-threaded-smoke.wasm"
  # Keep the guest build on Soldr's front door, but disable its cache for this
  # temporary output. Soldr's cached cross-target materialization is tracked
  # separately; a cache failure must not turn this admission characterization
  # into a false green or tempt us to use ambient Cargo.
  soldr --no-cache rustup target add "$target"
  (
    cd "$guest_dir"
    SOLDR_LINKER=default soldr --no-cache "$subcommand" build --locked --manifest-path Cargo.toml --target "$target" --release --target-dir "$target_directory"
  )
fi

KERNAL_API_THREADED_ARTIFACT_WASM="$artifact" \
  soldr --no-cache "$subcommand" test --locked --features wasm-sketch-host --lib supplied_threaded_artifact_admits_the_public_profile
