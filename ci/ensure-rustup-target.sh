#!/usr/bin/env bash
# Materialize a rustup target even when a restored toolchain cache's manifest
# says it is installed but its standard-library archives are absent.
set -euo pipefail

case "$#" in
    1)
        toolchain=""
        target="$1"
        rustc=(soldr rustc)
        ;;
    2)
        toolchain="$1"
        target="$2"
        rustc=(soldr rustup run "$toolchain" rustc)
        ;;
    *)
        printf 'usage: %s [<toolchain>] <target>\n' "$0" >&2
        exit 2
        ;;
esac

target_command() {
    if [ -n "$toolchain" ]; then
        soldr rustup target "$1" --toolchain "$toolchain" "$target"
    else
        soldr rustup target "$1" "$target"
    fi
}

require_absolute_path() {
    case "$1" in
        /*) ;;
        *)
            printf '%s is not absolute: %s\n' "$2" "$1" >&2
            exit 1
            ;;
    esac
}

target_materialized() {
    local libdir
    libdir="$("${rustc[@]}" --print target-libdir --target "$target")"
    require_absolute_path "$libdir" "rustc target-libdir"
    compgen -G "$libdir/libcore-*.rlib" >/dev/null \
        && compgen -G "$libdir/libstd-*.rlib" >/dev/null
}

if target_materialized; then
    exit 0
fi

target_command add
if target_materialized; then
    exit 0
fi

# A missing manifest makes rustup treat an absent target as installed. Creating
# the empty manifest lets `target remove` clear that stale bookkeeping before
# the final install attempt.
sysroot="$("${rustc[@]}" --print sysroot)"
require_absolute_path "$sysroot" "rustc sysroot"
manifest="$sysroot/lib/rustlib/manifest-rust-std-$target"
if [ ! -e "$manifest" ]; then
    : > "$manifest"
fi
target_command remove
target_command add

if ! target_materialized; then
    printf 'target %s still lacks core/std libraries after reinstall\n' "$target" >&2
    exit 1
fi
