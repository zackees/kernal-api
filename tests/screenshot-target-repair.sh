#!/usr/bin/env bash
set -euo pipefail
repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
test_root="$(mktemp -d)"
trap 'rm -r -- "$test_root"' EXIT
export CARGO_TARGET_DIR="$test_root/build"
export MOCK_SYSROOT="$test_root/sysroot"
mkdir -p "$MOCK_SYSROOT/lib/rustlib"
manifest="$MOCK_SYSROOT/lib/rustlib/manifest-rust-std-wasm32-wasip1-threads"
soldr() {
  case "$*" in
    *'rustc --print target-libdir'*)
      if [ "$MOCK_CASE" = probe-fails ]; then return 19; fi
      printf '%s/lib\n' "$MOCK_SYSROOT" ;;
    *'rustc --print sysroot'*) printf '%s\n' "$MOCK_SYSROOT" ;;
    *'rustup target remove'*)
      echo REMOVE
      if [ "$MOCK_CASE" = remove-fails ]; then return 17; fi
      MOCK_REMOVED=yes ;;
    *'rustup target add'*)
      echo ADD
      if [ "$MOCK_CASE" = install ] || { [ "${MOCK_REMOVED-no}" = yes ] && [ "$MOCK_CASE" != still-missing ]; }; then MOCK_READY=yes; fi ;;
    *'cargo build'*) echo BUILD ;;
    *'cargo run'*) echo EMBED ;;
    *) echo 'unexpected invocation' >&2; return 20 ;;
  esac
}
compgen() {
  [ "${MOCK_READY-no}" = yes ] || { [ "$MOCK_CASE" = core-only ] && [[ "$*" == *libcore* ]]; }
}
cp() { :; }
export -f soldr compgen cp
for MOCK_CASE in healthy install stale existing-manifest core-only remove-fails still-missing probe-fails; do
  export MOCK_CASE
  export MOCK_READY=no MOCK_REMOVED=no
  if [ "$MOCK_CASE" = healthy ]; then MOCK_READY=yes; fi
  if [ -e "$manifest" ]; then rm -- "$manifest"; fi
  if [ "$MOCK_CASE" = existing-manifest ]; then printf 'preserve\n' > "$manifest"; fi
  status=0
  output="$(bash "$repo_dir/examples/wasm-tauri-screenshot/build-guest.sh" 2>&1)" || status=$?
  case "$MOCK_CASE" in
    remove-fails|still-missing|probe-fails)
      [ "$status" -ne 0 ] || { echo "unexpected success: $MOCK_CASE"; exit 1; }
      case "$output" in *BUILD*) echo 'built after failed repair'; exit 1 ;; esac ;;
    *) [ "$status" -eq 0 ] || { printf '%s\n' "$output"; exit 1; } ;;
  esac
  case "$MOCK_CASE" in
    healthy|probe-fails)
      case "$output" in *ADD*|*REMOVE*) echo 'mutated healthy or unprobed toolchain'; exit 1 ;; esac ;;
    install) case "$output" in *REMOVE*) echo 'unnecessary removal'; exit 1 ;; esac ;;
    *) case "$output" in *REMOVE*) ;; *) echo 'repair not attempted'; exit 1 ;; esac ;;
  esac
  if [ "$MOCK_CASE" = existing-manifest ]; then [ "$(< "$manifest")" = preserve ]; fi
  printf 'PASS %s\n' "$MOCK_CASE"
done
