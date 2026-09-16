#!/usr/bin/env bash
# Stage the files the macOS Recovery guest fetches from http://10.0.2.2:8000/.
#
# The action serves a directory from the Linux runner, so everything the guest
# needs must be a regular file here: the cross-built nextest archive, a
# *Mach-O* cargo-nextest, and the optional extra filter expression.
#
# The archive is built by `soldr cargo nextest archive` on this Linux host,
# which means the guest's cargo-nextest must be a macOS binary -- not the one
# on PATH. nextest publishes a universal (x86_64 + arm64) macOS build for every
# release, so the guest binary is pinned to the exact version that produced the
# archive and verified against nextest's published sha256.
set -euo pipefail

ARCHIVE="${ARCHIVE:-$PWD/kernal-x64.tar.zst}"
SHARE="${SHARE:-$PWD/share}"
RELEASE_BASE="https://github.com/nextest-rs/nextest/releases/download"

test -f "$ARCHIVE" || {
  echo "missing nextest archive $ARCHIVE -- run ci/macos-x64/build-archive.sh first" >&2
  exit 1
}

# The archive and the runner must agree on nextest's archive format, so take the
# version from the same nextest that built it rather than pinning a constant
# here that can drift from `soldr`'s resolved version.
nextest_version() {
  if command -v cargo-nextest >/dev/null 2>&1; then
    cargo-nextest nextest --version
  else
    soldr cargo nextest --version
  fi | sed -n 's/^release: //p'
}

VERSION="$(nextest_version)"
test -n "$VERSION" || {
  echo "could not determine the cargo-nextest version that built the archive" >&2
  exit 1
}

# nextest names the checksum asset after the *stem*, not the tarball, so the
# two names are built separately rather than by appending `.sha256` to the URL.
STEM="cargo-nextest-${VERSION}-universal-apple-darwin"
TARBALL="${STEM}.tar.gz"
URL="${RELEASE_BASE}/cargo-nextest-${VERSION}/${TARBALL}"
CHECKSUM_URL="${RELEASE_BASE}/cargo-nextest-${VERSION}/${STEM}.sha256"

echo "staging guest share in $SHARE"
echo "  archive:  $ARCHIVE"
echo "  nextest:  $VERSION (universal-apple-darwin)"

rm -rf "$SHARE"
mkdir -p "$SHARE"

cp "$ARCHIVE" "$SHARE/kernal-x64.tar.zst"

# Download the Mach-O nextest and verify it before it can reach the guest. The
# guest has no way to check provenance, so this is the only place it happens.
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
curl -fsSL -o "$TMP/$TARBALL" "$URL"
curl -fsSL -o "$TMP/$TARBALL.sha256" "$CHECKSUM_URL"

(
  cd "$TMP"
  sha256sum --check --strict "$TARBALL.sha256"
)

tar -xzf "$TMP/$TARBALL" -C "$TMP"
test -f "$TMP/cargo-nextest" || {
  echo "$TARBALL did not contain a cargo-nextest binary" >&2
  exit 1
}

# The guest runs on Intel; confirm the payload is really a macOS universal
# binary rather than a same-named Linux artifact that would fail at boot.
magic="$(od -A n -t x1 -N 4 "$TMP/cargo-nextest" | tr -d ' \n')"
case "$magic" in
  cafebabe|bebafeca|cffaedfe|cefaedfe|feedfacf|feedface)
    echo "  verified Mach-O payload (magic $magic)"
    ;;
  *)
    echo "staged cargo-nextest is not a Mach-O binary (magic $magic)" >&2
    exit 1
    ;;
esac

install -m 0755 "$TMP/cargo-nextest" "$SHARE/cargo-nextest"

# Optional operator filter from the workflow_dispatch input. Always written, so
# the guest script can fetch it unconditionally.
printf '%s\n' "${NEXTEST_FILTER:-}" > "$SHARE/filter.txt"
chmod 0644 "$SHARE/filter.txt"

ls -lh "$SHARE"
