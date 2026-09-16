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

# ---------------------------------------------------------------------------
# Tests that cannot pass in this guest, grouped by cause.
#
# Every entry is excluded BY NAME so the list stays visible and reviewable, and
# each group records the evidence observed in the guest. None of this is a
# blanket filter: a test that starts failing for a NEW reason is not on this
# list and will fail the lane.
#
# The macOS-specific groups below are portability findings, not guest quirks --
# they would fail on any macOS host. They stayed invisible because every macOS
# *test* lane in this repository is gated off; this lane is the first to run
# the suite there. They are catalogued in issue #283 and in README.md.
# ---------------------------------------------------------------------------

# cap-primitives 4.0.3 panics converting a negative macOS st_rdev:
# `rdev: u64::try_from(stat.st_rdev).unwrap()` at
# cap-primitives-4.0.3/src/rustix/fs/metadata_ext.rs:171, surfacing as
# `TryFromIntError(())`. The `dev` field two lines above guards the same
# signedness, so `dev_t` is known-signed here and only `rdev` was missed.
# APFS device numbers in this guest are negative; there is no fixed 4.x release.
EXCLUDE_CAP_PRIMITIVES='commit_error_cleans_staging_after_destination_parent_is_renamed completed_symlink_is_not_published_or_followed missing_completed_output_preserves_destination_and_cleans_staging parent_discard_cleans_staging_after_destination_parent_is_renamed parent_discard_removes_worker_partial_and_completed_files replacement_failure_preserves_existing_directory_and_cleans_staging staged_output_is_invisible_until_parent_commit cancellation_after_parent_sync_preserves_output deadline_after_parent_sync_preserves_output_and_reports_cleanup_failure dispatcher_retry_releases_ownership_and_records_one_forced_reap output_cleanup_failure_reports_whether_publication_occurred parent_output_discards_on_failure_or_stop_and_commits_only_success'

# macOS refuses the fixtures' certificates: "The validity period in the
# certificate exceeds the maximum allowed" (Security framework, -67901).
EXCLUDE_MACOS_TLS='trusted_https_downgrade_is_rejected_before_plaintext_connection trusted_local_tls_preserves_body'

# macOS reports the canonical /private/var path where the test expects /var.
EXCLUDE_MACOS_PATH='context_metadata_and_link_read_do_not_follow_final_symlinks'

# macOS returns ENOTSUP (45) for the atomic rename the readiness marker relies
# on; Linux's exclusive-rename semantics have no direct macOS equivalent.
EXCLUDE_MACOS_RENAME='failed_marker_write_is_cleaned_up_and_existing_marker_is_preserved marker_is_invisible_until_payload_is_complete'

# APFS rejects an invalid-UTF-8 file name with EILSEQ (92) at creation, so the
# test never reaches the collision it asserts about.
EXCLUDE_MACOS_FILENAME='invalid_utf8_file_names_fail_instead_of_colliding'

# The guest runs as root, so a chmod-000 file is still readable and the test
# observes Ok where it asserts a permission error.
EXCLUDE_ROOT='bounded_context_read_reports_unreadable_input'

# A Recovery guest gives this script no controlling terminal to save and
# restore, so the termios flags it compares are not the ones it set.
EXCLUDE_TTY='native_session_rejects_overlap_and_restores_mode'

# Two cores in a VM are not representative for wall-clock assertions: a
# suspension window and two containment deadlines elapsed before the work did.
EXCLUDE_VM_TIMING='a_handful_of_threads_fills_a_small_ring_long_before_the_window_ends a_child_bound_to_another_owner_dies_when_that_owner_does real_worker_sequential_stress_leaves_no_parent_state'

EXCLUDED_TESTS="$EXCLUDE_CAP_PRIMITIVES $EXCLUDE_MACOS_TLS $EXCLUDE_MACOS_PATH $EXCLUDE_MACOS_RENAME $EXCLUDE_MACOS_FILENAME $EXCLUDE_ROOT $EXCLUDE_TTY $EXCLUDE_VM_TIMING"

GUEST_EXCLUDE=''
for name in $EXCLUDED_TESTS; do
  if [ -z "$GUEST_EXCLUDE" ]; then
    GUEST_EXCLUDE="test(~$name)"
  else
    GUEST_EXCLUDE="$GUEST_EXCLUDE or test(~$name)"
  fi
done

# --- end of exclusions ------------------------------------------------------

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

FILTER="$POLICY_FILTER and not ($GUEST_EXCLUDE)"
EXTRA_FILTER="$(cat "$WORK/filter.txt" 2>/dev/null || true)"
if [ -n "$EXTRA_FILTER" ]; then
  FILTER="($FILTER) and ($EXTRA_FILTER)"
fi
echo "$GUEST_EXCLUDE" > "$COLLECT/excluded-tests.txt"
echo "$FILTER" > "$COLLECT/filter.txt"

# --no-fail-fast: one guest boot is expensive, so a run must report every
# failure it can find rather than stopping at the first.
"$WORK/$NEXTEST_BIN" nextest run --archive-file "$WORK/$ARCHIVE" --workspace-remap "$WORKSPACE" --no-fail-fast -E "$FILTER" > "$COLLECT/nextest.log" 2>&1
echo $? > "$COLLECT/nextest.rc"

echo "--- nextest log tail ---"
tail -n 40 "$COLLECT/nextest.log" 2>/dev/null || true
echo "--- guest script complete ---"
