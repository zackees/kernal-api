#!/usr/bin/env bash
set -euo pipefail
repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Only inspect script argument construction; no compiler or file copies run.
soldr() {
  case "$*" in
    *'rustc --print target-libdir'*) printf '/mock/guest/lib\n' ;;
    *) printf 'SOLD R'; printf ' <%s>' "$@"; printf '\n' ;;
  esac
}
# This argument-only test models a healthy target; no actual filesystem probe.
compgen() { return 0; }
cp() { :; }
export -f soldr cp compgen
export CARGO_TARGET_DIR="/tmp/screenshot mock storage"
for mode in normal trap block; do
  case "$mode" in
    normal) set -- ;;
    trap) set -- --trap-after-capture ;;
    block) set -- --block-after-capture ;;
  esac
  result="$(bash "$repo_dir/examples/wasm-tauri-screenshot/build-guest.sh" "$@")"
  case "$result" in
    *'<--target> <wasm32-wasip1-threads> <'*) ;;
    *) echo 'target argument missing' >&2; exit 1 ;;
  esac
  if [ "$mode" = normal ]; then
    case "$result" in *'<--features>'*) echo 'normal guest gained fault feature' >&2; exit 1 ;; esac
  else
    case "$result" in *"<--features> <proof-$mode-after-capture>"*) ;; *) echo 'fault feature missing' >&2; exit 1 ;; esac
  fi
  case "$result" in *'<--target-dir> </tmp/screenshot mock storage/'*) ;; *) echo 'storage path split' >&2; exit 1 ;; esac
  printf 'PASS %s\n' "$mode"
done
