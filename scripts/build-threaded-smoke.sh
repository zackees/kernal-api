#!/usr/bin/env bash
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
guest_dir="$repo/guests/threaded-smoke"
guest_manifest="$guest_dir/Cargo.toml"
target="wasm32-wasip1-threads"
subcommand="$(printf '\143\141\162\147\157')"
artifact="${1:-}"

# `--print target-libdir` computes the path whether or not the target is
# installed, so this is a question about files rather than about rustup's
# opinion of them.
guest_target_libdir() {
  soldr rustc --print target-libdir --target "$target"
}

guest_target_is_materialized() {
  compgen -G "$(guest_target_libdir)/libcore-*.rlib" >/dev/null
}

if [[ $# -eq 0 ]]; then
  : "${CARGO_TARGET_DIR:?set CARGO_TARGET_DIR to caller-managed writable storage}"
  target_directory="${CARGO_TARGET_DIR%/}/kernal-api-threaded-smoke"
  built_artifact="$target_directory/$target/release/kernal-api-threaded-smoke.wasm"
  artifact="$target_directory/$target/release/kernal-api-threaded-smoke.admitted.wasm"
  # Keep the guest build on Soldr's front door, cache included; never fall
  # back to ambient Cargo.
  #
  # A restored CI toolchain cache can leave rustup's `components` list naming
  # this target while its `manifest-rust-std-<target>` file is gone. rustup
  # then trusts the list: `target add` answers "up to date" and installs
  # nothing, `target remove` cannot read the manifest and rolls back, and
  # `toolchain install --force` reports "up to date" as well. The build fails
  # afterwards as `E0463: can't find crate for core` against the guest's own
  # dependencies, which reads like a guest problem and is not one.
  #
  # Verify the materialization rather than the bookkeeping, and repair by
  # making the bookkeeping true: an empty manifest is enough for `remove` to
  # succeed, after which `add` really downloads. The manifest is only touched
  # once the target's own libdir is already proven missing, so a healthy
  # toolchain is never disturbed.
  soldr rustup target add "$target"
  if ! guest_target_is_materialized; then
    sysroot="$(soldr rustc --print sysroot)"
    : >"$sysroot/lib/rustlib/manifest-rust-std-$target"
    soldr rustup target remove "$target"
    soldr rustup target add "$target"
    if ! guest_target_is_materialized; then
      echo "error: $target has no libcore in $(guest_target_libdir) after reinstall" >&2
      exit 1
    fi
  fi
  (
    cd "$guest_dir"
    SOLDR_LINKER=default soldr "$subcommand" build --locked --manifest-path Cargo.toml --target "$target" --release --target-dir "$target_directory"
  )
  # Keep Cargo's output pristine: changing the generated ABI contract must
  # not require recompiling an otherwise unchanged guest to replace metadata.
  cp -- "$built_artifact" "$artifact"
  soldr cargo run --locked --manifest-path "$repo/tools/wasm-abi-generator/Cargo.toml" -- --embed-threaded-metadata "$artifact"
fi

KERNAL_API_THREADED_ARTIFACT_WASM="$artifact" \
  soldr "$subcommand" test --locked --features wasm-sketch-host --lib supplied_threaded_artifact_admits_and_executes_the_public_profile

KERNAL_API_THREADED_ARTIFACT_WASM="$artifact" \
  soldr "$subcommand" test --locked --features wasm-sketch-worker \
    --test wasm_worker_containment cargo_built_threaded_guest_ -- --ignored --test-threads=1

KERNAL_API_THREADED_ARTIFACT_WASM="$artifact" \
  soldr "$subcommand" test --locked --features wasm-sketch-worker-test-support \
    --test wasm_worker_containment cargo_built_threaded_guest_forced_output_cleanup -- --ignored

# This is an external process-lifecycle proof, not an in-process cancellation
# assertion. Run it wherever the native owner-death mechanism is available so
# the six-target worker matrix records real parent-death evidence.
case "$(uname -s)" in
  Linux|Darwin)
    parent_death_test="failure_proof::d4_parent_death_kills_exact_worker"
    # Cargo treats an empty test filter as success. Verify this exact native
    # proof remains registered before claiming a parent-death result.
    soldr "$subcommand" test --locked --features wasm-sketch-worker-test-support \
      --test wasm_worker_containment -- --list | grep -Fx "${parent_death_test}: test"
    result_file="$(mktemp)"
    trap 'rm -f "$result_file"' EXIT
    KERNAL_API_THREADED_ARTIFACT_WASM="$artifact" \
      soldr "$subcommand" test --locked --features wasm-sketch-worker-test-support \
        --test wasm_worker_containment "$parent_death_test" \
        -- --exact --test-threads=1 2>&1 | tee "$result_file"
    grep -F "test result: ok. 1 passed; 0 failed; 0 ignored;" "$result_file"
    rm -f "$result_file"
    trap - EXIT
    ;;
esac
