#!/usr/bin/env bash
# Cross-build x86_64-apple-darwin test binaries on Linux.
#
# Soldr carries its own blessed cross toolchain -- bundled LLVM plus a managed
# macOS SDK -- so this produces real Mach-O executables with no Mac involved.
# Measured locally: 25 binaries, 103 files, ~112 MB archive, ~85 s.
set -euo pipefail

OUT="${OUT:-$PWD/kernal-x64.tar.zst}"

soldr cargo nextest archive \
  --target x86_64-apple-darwin \
  --all-features \
  --archive-file "$OUT"

echo "archive: $(du -h "$OUT" | cut -f1) -> $OUT"
