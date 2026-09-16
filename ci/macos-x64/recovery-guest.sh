# Run the prebuilt x86_64-apple-darwin nextest archive inside a macOS Recovery
# guest booted by the `zackees/docker-mac-x64` action on a Linux runner.
#
# Recovery gives exactly one boot and one script: there is no per-command
# execution channel, no Rust toolchain, no Xcode CLT and no Homebrew. Everything
# this script needs is fetched over the action's HTTP share at 10.0.2.2:8000.
#
# This script never exits non-zero. A guest boot costs minutes, so a failure
# must still produce collected evidence for the host to read -- the verdict is
# decided by ci/macos-x64/verify-guest-results.py from the files written under
# /tmp/results, not from this process's exit status.
#
# Shell target is Recovery's bash 3.2: no associative arrays, no ${v^^}, no
# `local -n`, and no comments inside the continued `nextest run` command below
# (a prior harness split a continued command on an embedded comment and
# executed an option as if it were a program).

set -u

SHARE=http://10.0.2.2:8000
WORK=/tmp/kernal-x64
COLLECT=/tmp/results
ARCHIVE=kernal-x64.tar.zst
NEXTEST_BIN=cargo-nextest

# Source-inspection policy tests read this crate's own tree through
# `env!("CARGO_MANIFEST_DIR")`, a compile-time constant still pointing at the
# Linux builder's path. `--workspace-remap` cannot rewrite a compile-time
# constant, and none of the guest's files live there. They assert on source
# text that is identical on every host, and `rust-native (ubuntu-latest)`
# already runs them. Mirrors the aarch64 lane's exclusion list in ci.yml.
POLICY_FILTER='not (binary(daemon_frame_v1) or binary(daemon_identity) or binary(version_policy) or binary(facade_policy))'

mkdir -p "$WORK" "$COLLECT"
cd "$WORK" || exit 0

echo "guest kernel: $(uname -a)"
echo "guest arch:   $(uname -m)"

fail() {
  echo "guest staging failed: $1" >&2
  echo "$1" > "$COLLECT/stage-failure.txt"
  exit 0
}

fetch() {
  curl -fsS -o "$WORK/$1" "$SHARE/$1" 2>"$COLLECT/fetch-$1.err"
}

fetch "$ARCHIVE" || fail "could not fetch $ARCHIVE from $SHARE"
fetch "$NEXTEST_BIN" || fail "could not fetch $NEXTEST_BIN from $SHARE"
fetch filter.txt || fail "could not fetch filter.txt from $SHARE"

chmod +x "$WORK/$NEXTEST_BIN"

# Record what actually ran, so the host can prove the guest Nextest matches the
# one that built the archive rather than assuming it.
"$WORK/$NEXTEST_BIN" nextest --version > "$COLLECT/nextest-version.txt" 2>&1 || true

# Replaying an archive makes nextest require a workspace root containing a
# Cargo.toml (`ReuseWithWorkspaceRemap`), and it exits 96 without one. The guest
# has no source tree and does not need one: every fixture these tests read is
# compiled into the binaries, and the four tests that do read the tree are
# excluded above. The stub satisfies the root check and leaves config discovery
# empty, which is what the archive's own cargo-metadata.json already implies.
# Written with printf rather than a heredoc because this script arrives as
# typed input, not as a file with a real shebang.
WORKSPACE="$WORK/workspace"
mkdir -p "$WORKSPACE"
printf '%s\n' '[workspace]' 'members = []' > "$WORKSPACE/Cargo.toml"

FILTER="$POLICY_FILTER"
EXTRA_FILTER="$(cat "$WORK/filter.txt" 2>/dev/null || true)"
if [ -n "$EXTRA_FILTER" ]; then
  FILTER="($POLICY_FILTER) and ($EXTRA_FILTER)"
fi
echo "$FILTER" > "$COLLECT/filter.txt"

# --no-fail-fast: one guest boot is expensive, so a run must report every
# failure it can find rather than stopping at the first.
"$WORK/$NEXTEST_BIN" nextest run --archive-file "$WORK/$ARCHIVE" --workspace-remap "$WORKSPACE" --no-fail-fast -E "$FILTER" > "$COLLECT/nextest.log" 2>&1
echo $? > "$COLLECT/nextest.rc"

echo "--- nextest log tail ---"
tail -n 40 "$COLLECT/nextest.log" 2>/dev/null || true
echo "--- guest script complete ---"
