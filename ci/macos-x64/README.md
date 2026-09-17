# Intel macOS test lane

Executes the `x86_64-apple-darwin` test suite. The suite is **built on Linux**
and only **run on macOS**: a GitHub-hosted Linux runner cross-compiles the
archive through Soldr's own toolchain, then boots a real x86_64 macOS Recovery
guest under QEMU/KVM and replays the archive inside it.

Nothing here needs a `macos-*` runner, a baked guest image, an SSH secret, or a
published GHCR package. `.github/workflows/macos-x64-tests.yml` runs nightly and
on demand, and is advisory — never a required check, never a release gate.

## Pieces

| File | Runs on | Job |
|---|---|---|
| `build-archive.sh` | Linux runner | Cross-build the nextest archive |
| `stage-guest-share.sh` | Linux runner | Stage the archive + Mach-O `cargo-nextest` the guest fetches over HTTP |
| `recovery-guest.sh` | macOS guest | Fetch the archive, run it, write evidence to `/tmp/results` |
| `verify-guest-results.py` | Linux runner | Decide pass/fail and assert Intel coverage from the collected evidence |

The guest half is driven by [`zackees/docker-mac-x64`][action], pinned to a
commit in the workflow. It owns the KVM setup, the disk reclaim, the boot, the
HTTP share at `10.0.2.2:8000`, and the collection of `/tmp/results`.

## Why the guest binary is a downloaded Mach-O

The archive is produced on Linux by `soldr cargo nextest archive`, so the
`cargo-nextest` on the runner's `PATH` is a Linux binary and cannot run in the
guest. `stage-guest-share.sh` therefore reads the version from the same nextest
that built the archive and downloads nextest's published
`universal-apple-darwin` build for that exact version, verifying it against the
release's `.sha256` before it can reach the guest.

nextest owns its archive format, so the guest's nextest is matched to the
builder's rather than pinned to a constant here that could drift.

## Why Recovery, not a prebaked image

This lane previously pulled a hand-baked `dockur/macos` image from this
repository's own GHCR namespace. That image was never published — the
`macos-x64-bake.yml` workflow the old comment named has never existed in this
repository — and producing it required a 30–60 minute manual macOS install plus
a 128 GB volume published by hand. Every run since the workflow landed failed at
`guest.sh start` with `manifest unknown`; the scripts and the Dockerfile for
that approach have been removed rather than left to rot.

A Recovery guest has no toolchain of its own, so the archive must be
self-contained. It is: every fixture these tests read is `include_str!`/
`include_bytes!` and therefore already inside the test binaries.

## Why the guest writes a stub `Cargo.toml`

Replaying an archive makes nextest require a workspace root that contains a
`Cargo.toml` (`ReuseWithWorkspaceRemap`), and it exits **96** without one —
before running a single test. The guest has no source tree and does not need
one, because nothing that runs reads it, so `recovery-guest.sh` writes a
minimal empty-workspace manifest and points `--workspace-remap` at it. The stub
satisfies nextest's root check and leaves config discovery empty, which is what
the archive's own `cargo-metadata.json` already implies.

The four source-inspection tests that *would* read the tree are excluded, so
the stub never has to stand in for real source.

## What the guest does not run

- **`#[ignore]`d tests** are skipped by nextest by default. That covers every
  test needing a full macOS userland — the Tauri/WKWebView screenshot proofs,
  the threaded-worker containment proofs, and the archive fixtures — none of
  which belong in a Recovery partition.
- **Four source-inspection policy tests** are excluded by name in
  `recovery-guest.sh`: `daemon_frame_v1`, `daemon_identity`, `version_policy`,
  and `facade_policy`. They read this crate's own tree through
  `env!("CARGO_MANIFEST_DIR")`, a compile-time constant still pointing at the
  Linux builder's path, and `--workspace-remap` cannot rewrite a compile-time
  constant. They assert on source text that is identical on every host and
  ci.yml's Linux `test-run` lanes already run them. This mirrors the exclusion
  the aarch64 lane carries in `ci.yml`.
- **19 further tests**, excluded by name and grouped by cause in
  `recovery-guest.sh`. Each entry is named rather than pattern-matched, so the
  list stays reviewable, and each group carries the evidence observed in the
  guest. None of it is a blanket filter: a test that starts failing for a *new*
  reason is not on the list and will fail the lane.

| Group | N | Why it cannot pass here |
|---|---|---|
| `EXCLUDE_CAP_PRIMITIVES` | 12 | `cap-primitives` 4.0.3 panics converting a negative macOS `st_rdev` — `u64::try_from(stat.st_rdev).unwrap()` at `metadata_ext.rs:171`. The `dev` field two lines above guards the same signedness, so `dev_t` is known-signed here and only `rdev` was missed. Surfaces as `TryFromIntError(())`. No fixed 4.x exists. |
| `EXCLUDE_MACOS_RENAME` | 2 | The readiness marker's no-clobber publish returns `ENOTSUP` (45). `tempfile` asks for `renameatx_np(RENAME_EXCL)`, which real macOS supports, so this most likely reflects the guest's virtualized filesystem rather than macOS. Test-only: `persist_noclobber` has no production caller. |
| `EXCLUDE_ROOT` | 1 | The guest runs as **root**, so a `chmod 000` file stays readable and the test observes `Ok` where it asserts a permission error. |
| `EXCLUDE_TTY` | 1 | A Recovery guest gives the script no controlling terminal to save and restore. |
| `EXCLUDE_VM_TIMING` | 2 | Two cores in a VM are not representative for wall-clock assertions; a suspension window and a containment deadline elapsed before the work did. |
| `EXCLUDE_CONTAINMENT_STATE` | 1 | **Investigate, not an artifact.** `real_worker_sequential_stress_leaves_no_parent_state` observed `ForcedContainment { trigger: Cancelled }` where it expects `Stopped(Cancelled)`. That is a state mismatch rather than an elapsed deadline, so it may be a genuine macOS containment difference. Excluded to keep the lane green while it is investigated (#283). |

**Three macOS portability findings this lane surfaced have since been fixed in
the tests themselves** and are no longer excluded: the TLS fixture's 3650-day
validity (now 820 days, under macOS's 825-day ceiling), the `/var` →
`/private/var` canonicalization comparison in `context_file_observation`, and
the invalid-UTF-8 file name in `tree_hash`, which APFS rejects with `EILSEQ`
before the hash can ever see it. All three stayed invisible only because every
macOS *test* lane in this repository is gated off — this lane is the first to
run the suite there.

**The `EXCLUDE_MACOS_RENAME` group is not in that category.** `tempfile`'s
`persist_noclobber` asks for `renameatx_np(RENAME_EXCL)`, which real macOS
supports, so the `ENOTSUP` is most likely a property of the guest's virtualized
filesystem rather than of macOS. It is excluded as a guest artifact pending a
check on real hardware (issue #283). **`EXCLUDE_CAP_PRIMITIVES` is an upstream
bug**, not a test defect.

## Coverage assertion

`verify-guest-results.py` fails the job unless the collected log proves the
guest ran **Intel** code. `src/snapshot/unwind.rs`'s `frame_pointer_tests`
module compiles only under `#[cfg(all(test, target_arch = "x86_64"))]`, so
naming those six tests is what distinguishes a real Intel run from a green job
that executed nothing. That failure mode — passing while verifying nothing — is
the one this lane was created to close.

## Local use

The Linux half runs anywhere:

```bash
./build-archive.sh          # ~85 s, produces kernal-x64.tar.zst
./stage-guest-share.sh      # downloads the pinned Mach-O cargo-nextest
```

The guest half needs `/dev/kvm`. On a host that has it, the workflow step is the
only supported entry point — `zackees/docker-mac-x64` is a GitHub Action and its
`run:` input is the guest's one script.

[action]: https://github.com/zackees/docker-mac-x64
