# Canonical `running-process` re-export audit

Issue #196 records this audit and migration plan. It does not itself authorize
a repository-wide blind refactor. Implemented canonical exceptions include
#189's independent spawn API, the opt-in broker contract, v2 registration,
async process/containment primitives, explicit foreground execution, and native
exit-status conversion.
Implementation here means source changes, not completed validation or release.
Every remaining row needs the listed follow-up
to preserve wire, error, lifetime, and consumer compatibility.

## Rules and evidence

The facade depends one-way on `running-process`; it must never expose
`running-process-platform-internal` directly. A confirmed equivalent is
replaced by `pub use running_process::path::Symbol` (or `as PublicName`), not a
same-shaped local type. Each migration must first add a compile-time
cross-namespace assignment test in both directions, plus the focused behavior
or wire test named below. Matching variants, debug output, or serialized happy
paths are insufficient evidence of type identity.

The source paths below refer to this checkout and the selected substrate
revision in `_vender/running-process` during integration. Before release, the
temporary path must be replaced by its published exact registry version.

The inventory covered every module that originally imported `running_process`
directly (`src/process_adapter.rs`, `src/daemon_frame_v1.rs`,
`src/daemon_registration.rs`, `src/daemon_registration_v2.rs`, and
`src/daemon_identity.rs`) and independently compared public process names in
`src/platform/process.rs` and `src/sync_spawn_group.rs`. The three frozen
frame/registration facade modules have subsequently been deleted after their
compatibility contracts moved into the substrate; `daemon_identity` remains a
facade-owned semantic wrapper. The inventory also searched the remaining public
adapter-facing modules and crate root for matching public names. Absence of an
import is not evidence that a copied API is unique: a copied native type can
have no substrate spelling at all. Native modules below are therefore recorded
as *unresolved/non-candidates*, not cleared.

The independent-spawn namespace now aliases the entire public substrate module
in `src/lib.rs` (`pub use running_process::independent_spawn`).
There is no local module export list to update when the selected dependency adds
public items. Root convenience exports retain their existing names. Type and
function identity checks are authored but have not yet been executed.

Substrate references use integration revision
[`0e39d9d403883dc87c71d677012bc0c0a5d0c693`](https://github.com/zackees/running-process/tree/0e39d9d403883dc87c71d677012bc0c0a5d0c693)
(workspace version `4.10.12`, pending publication). Line references are intentionally included so a
later substrate revision can be compared rather than assumed equivalent.

| Facade surface | Current canonical/source comparison | Finding and proposed action | Gate/consumer risk | Closure evidence |
| --- | --- | --- | --- | --- |
| `independent_spawn::{LaunchSpec, Readiness, IndependentChild, spawn}` plus root `SpawnMode`, `SpawnOptions`, `SpawnLifetime`, and `IndependentBackend` | Canonical public substrate contract owned by native spawn | Confirmed canonical contract: the entire module is directly aliased in `src/lib.rs`, with root types directly re-exported; no local module or mapping. The default remains inherited placement and independent placement never falls back | `kernel-substrate`; native scheduler/broker selection is explicit | Cross-namespace assignment; default is `Inherited`; native Linux/Windows placement, unsupported, cancellation/readiness, partial-launch cleanup tests |
| `broker::{protocol, protocol_v2, session_codec, server, backend_lifecycle, backend_sdk, host_identity, …}` and its root aliases (`src/broker.rs`) | Existing direct aliases exported through [`broker/mod.rs`](https://github.com/zackees/running-process/blob/0e39d9d403883dc87c71d677012bc0c0a5d0c693/crates/running-process/src/broker/mod.rs), [`backend_identity.rs:17-31`](https://github.com/zackees/running-process/blob/0e39d9d403883dc87c71d677012bc0c0a5d0c693/crates/running-process/src/backend_identity.rs#L17-L31), and `broker::client` | Confirmed canonical opt-in broker namespace: direct re-export of the public broker module tree exposes handle/probe, identity sidecar, frozen framing, route/refusal/client types, protocol v1/v2, session codec, server and lifecycle utilities without a type layer. `BackendHandle` already has public `service_name`, `service_version`, and `daemon_process`; its endpoint is `daemon_process.ipc_endpoint`, so no redundant accessor is needed | opt-in `broker = [running-process/client]`; no default graph expansion | Cross-namespace assignments; frozen-frame golden bytes; refusal class/code/detail and verified probe behavior |
| `broker::verify_pid` | Exact `pub use running_process::broker::backend_lifecycle::verify_pid` namespace alias | Confirmed identity-safe stale-daemon migration surface. `verify_daemon_process_for_control` verifies boot, liveness, exact executable path and hash while retaining a control-capable `ProcessHandle`; `force_kill_handle` acts on that same native object, avoiding PID reuse between verification and destructive cleanup. The handle distinguishes a terminated Windows process from a merely open handle through `is_alive`. No PID-only executable-path alias is exposed because it cannot be made generation-safe | opt-in `broker`; no default graph expansion | Cross-namespace namespace/type/function assignments and Windows terminated-handle, PID reuse, path/hash mismatch tests |
| `daemon_registration_v2::canonical::{ServiceDefinition, ServiceDefinitionBuilder, ServiceDefinitionError, LoadedServiceDefinitionV2, read_service_definition_v2, service_definition_*}` | Existing public substrate v2 API, including the integration reader that returns decoded definition plus frozen bytes | Confirmed canonical subset: direct namespace is available alongside the narrower compatibility API. `read_service_definition_v2(root, name)` preserves bytes and uses typed errors for missing/private/malformed/name-mismatch cases without creating a directory | `daemon-registration-v2`; no default graph expansion | Cross-namespace assignments and v2 persisted roundtrip/error tests |
| `daemon_frame_v1::{DaemonFrame, DaemonFrameKind, DaemonPayloadEncoding, DaemonFrameCodec, DaemonFrameDecode, DaemonFrameError}` (former `src/daemon_frame_v1.rs`) | `running_process::daemon_frame_v1` now owns the raw `i32` kind/encoding and raw trace-header compatibility contract, rather than exposing only generated protobuf enums | Resolved: the former facade implementation was moved into the substrate's public compatibility module and KA whole-namespace re-exports it. This preserves unknown additive values and frozen bytes without a KA mirror type | `daemon-frame-v1`; zccache payload `0x7A63`, golden bytes, unknown additive values | Golden bytes; unknown kind/encoding and raw trace header decode→encode; cross-namespace identity through `kernal_api::daemon_frame_v1` |
| `daemon_registration::{CacheRootKind, CacheRoot, CacheManifest*, ServiceDefinition*, DaemonRegistrationError}` (former `src/daemon_registration.rs`) | `running_process::daemon_registration_compat` now owns the borrowed root view, unknown kind preservation, compatibility builders, and error classification | Resolved: the exact frozen-v1 compatibility API moved to the substrate and KA uses `pub use ... as daemon_registration`; no facade copy or conversion table remains | `daemon-registration`; v1 persistence and owner-private directory behavior | Existing registration fixtures plus invalid root/name/error and persistence checks; cross-namespace identity through `kernal_api::daemon_registration` |
| `daemon_registration_v2::{ServiceDefinition*, DaemonRegistrationV2Error, service_definition_*}` (former `src/daemon_registration_v2.rs`) | `running_process::daemon_registration_v2_compat` owns the limited shared-broker compatibility surface; its `canonical` submodule exposes the broader direct v2 types | Resolved: KA aliases the compatibility namespace rather than re-implementing its narrowed builder/error contract. The direct `canonical` submodule remains available for consumers that need the complete substrate surface | `daemon-registration-v2`; dual-write rollout and validation diagnostics | v2 fixture bytes/path/private-directory/non-atomic-write tests; cross-namespace identity for compatibility and `canonical` surfaces |
| `daemon_identity::{DaemonEndpoint, DaemonIdentity, DaemonIdentityHashPolicy, Probe*, ProductFrame, DaemonMuxError}` (`src/daemon_identity.rs:18-369`) | [`backend_identity.rs:17-31`](https://github.com/zackees/running-process/blob/0e39d9d403883dc87c71d677012bc0c0a5d0c693/crates/running-process/src/backend_identity.rs#L17-L31) exports endpoint/process/probe/backend handles | Not equivalent: facade owns `ProductFrame` and mux refusal behavior; `DaemonEndpoint` and identity fields also rename/translate backend endpoint/process types. First define which endpoint/identity subset has identical error, serialization, and trait contracts | `daemon-identity`; endpoint selection and application payload ownership | Probe/sidecar/mux characterization and endpoint policy tests; only split narrowly equivalent identity records after API review |
| Root process session/output/error types and `src/process_adapter.rs:1-650` | `AsyncProcessSessionEvent`, `StreamKind`, and `ProcessError` are root exports at [`lib.rs:213-217,261-264`](https://github.com/zackees/running-process/blob/0e39d9d403883dc87c71d677012bc0c0a5d0c693/crates/running-process/src/lib.rs#L213-L264) | Facade session/output groups remain non-equivalent because they add owner binding, single-consumer output, bounded capture, post-exit drain, and `io::Error` compatibility. `async_process` now directly aliases the independently useful `StreamKind` and `ProcessError` as `AsyncProcessError`, preserving stream routing and native `ErrorKind`/raw OS detail | mandatory lightweight `kernel-substrate`; primary application process API | Preserve lifecycle/timeout/cancellation/output-limit/owner-death tests; cross-namespace identity for StreamKind/error |
| `async_process::{AsyncProcess, AsyncProcessBuilder, AsyncProcessSession, AsyncProcessSessionControl, AsyncProcessSessionOutput, AsyncProcessSessionEvent, AsyncProcessSessionChunk, AsyncCapturedOutput, AsyncStdio}` | Root direct exports at [`lib.rs:213-217`](https://github.com/zackees/running-process/blob/0e39d9d403883dc87c71d677012bc0c0a5d0c693/crates/running-process/src/lib.rs#L213-L217) | Confirmed canonical semantic child subset; direct aliases now provide child/session, piped output, lifecycle, and bounded capture without Tokio types in their public API | mandatory lightweight `kernel-substrate`; no additional feature expansion | Cross-namespace assignment plus child stdout/wait/kill/try-wait/bounded-capture tests |
| `process::{spawn, spawn_daemon, spawn_daemon_with_stdio_and_env_policy, spawn_with_environment, spawn_daemon_with_environment, spawn_with_explicit_environment, spawn_daemon_with_explicit_environment, SyncEnvironment, DaemonChild, DaemonStdio, DaemonStdioSource, SpawnStdio, StdioSource, EnvironmentPolicy, observer::*, blake3_file}` | Selected root/submodule exports from `running-process::spawn`, `observer`, and client hash API | Confirmed canonical migration set for Soldr/zccache. Live and explicit base entrypoints preserve environment semantics, daemon stdio/breakaway, and Unix drop-time shutdown callbacks without a facade adapter. It is a reviewed list, not a crate-wide export; `broker` feature selects the needed client hash capability while preserving default=[] | opt-in `broker`; no default graph expansion | Cross-namespace assignment plus existing spawn/observer/hash behavior tests |
| `containment::{ContainedProcessGroup, SpawnedChild, ORIGINATOR_ENV_VAR}` and `async_process::{spawn_tokio, TokioSpawnOptions}` | Root exports at the selected substrate, with `spawn_tokio` gated by `client-async` | Containment aliases preserve synchronous group ownership. The opt-in Tokio spawn bridge still accepts/returns native Tokio process types: it is transitional, not evidence that application process migration is complete | containment is default-light; `async-process-client = [running-process/client-async]` is opt-in | Cross-namespace assignment and contained-child cleanup/Tokio options tests; remove client bridge usage before claiming the broader migration complete |
| Root and `platform::process::exit_code` | `running_process::native_exit_code` directly exports the native substrate function; all three selected platform modules rename that exact function | Implemented: removed equivalent Linux/macOS/Windows copies without changing negative Unix signal codes or Windows fallback. Narrow exact alias is policy-tested | Default-light, standard-library argument/result, no new runtime or feature dependency | Authored real-child exit37 and Unix SIGTERM tests compare all three namespaces; not yet run |
| `platform::process::SpawnStdio` | [`spawn.rs:32-34`](https://github.com/zackees/running-process/blob/0e39d9d403883dc87c71d677012bc0c0a5d0c693/crates/running-process/src/spawn.rs#L32-L34) re-exports the same canonical struct | Resolved: direct `pub use running_process::SpawnStdio` replaced the copied platform struct, preserving the existing path and exact type identity | Default process surface | Existing contained stdio/default/drain tests plus bidirectional platform/root identity test |
| `platform::process::{DaemonStdio, DaemonStdioSource, StdioSource}` | Same canonical re-exports at [`spawn.rs:32-34`](https://github.com/zackees/running-process/blob/0e39d9d403883dc87c71d677012bc0c0a5d0c693/crates/running-process/src/spawn.rs#L32-L34) | Resolved: direct aliases replaced copied definitions, preserving variants/defaults and borrowed-file lifetime exactly | Default process surface | Stdio construction/default and native handle inheritance behavior plus identity test |
| `platform::process::DaemonChild` | Same canonical public re-export at [`spawn.rs:32-34`](https://github.com/zackees/running-process/blob/0e39d9d403883dc87c71d677012bc0c0a5d0c693/crates/running-process/src/spawn.rs#L32-L34); canonical `spawn_daemon_with_environment` owns the native daemon boundary | Resolved: platform path directly aliases the canonical handle, and generic Unix/macOS/Windows daemon launch no longer adapts a local child control. The native implementation retains environment selection, stdio, Job/breakaway, and lifecycle actions | Default process surface | Child id/wait/try_wait/kill plus platform/root identity tests |
| `platform::process::SpawnedChild` | Same canonical root re-export; canonical `spawn_with_environment` owns the generic group/Job control and `from_parts` remains available for distinct local controls | Resolved: platform path aliases the canonical handle and generic Unix/Windows contained launch no longer retains a KA group/Job control. Unix/macOS worker adapters may still retain the canonical child after taking public pipes, so dropping that retained child performs the one shutdown | Default process surface | Existing cleanup tests plus bidirectional identity and one-drop control regression test |
| `platform::process::SyncEnvironment` and contained/daemon launch in `sync_spawn_group.rs` and `platform_win/sync_spawn.rs` | `running_process::{SyncEnvironment, spawn_with_environment, spawn_daemon_with_environment}` | Resolved: the copied environment enum is an exact direct alias, and every live or explicit environment launch now delegates. Unix/macOS pass KA's drop-time `KERNAL_API_KILL_DRAIN_TIMEOUT_MS` callback; Windows deliberately supplies no Unix callback and uses the canonical Job-close path. Daemon delegation retains supplied stdio and breakaway | Default process surface | Bidirectional type/function assignment and source contract tests; native Job-before-child/assign-before-resume regressions live with the canonical implementation |
| `platform::process::ConsoleWindowInfo` and platform `monitor_console_windows` | `running_process::{ConsoleWindowInfo, monitor_console_windows}`; KA's three platform implementations matched the native substrate's fields and delegation exactly | Resolved: direct aliases remove the copied record, empty Unix stubs, and Windows polling/enumeration implementation. The canonical function retains the Windows visible-window baseline/dedup timing and empty non-Windows behavior | Default process surface | Bidirectional type/function assignment plus existing Windows popup and non-Windows empty-list behavior tests |
| `platform::process::CaptureStream` | `running_process::StreamKind` has the same two variants and is already the canonical stream tag used by the async/session APIs | Resolved: direct `pub use running_process::StreamKind as CaptureStream` removes the copied enum while preserving existing capture-reader hook signatures and their stdout/stderr cancellation slots | Default process surface | Bidirectional type assignment plus existing capture reader completion/cancellation tests |
| `daemon_frame_v1` and `register_daemon_frame_payload_protocol!` | Feature-gated `running_process::{daemon_frame_v1, register_daemon_frame_payload_protocol}` | Resolved: KA now whole-namespace aliases the canonical frozen v1 compatibility module and re-exports its macro; the local codec/schema/macro implementation is deleted | `daemon-frame-v1 = [running-process/frame-v1-codec]`; no broker/runtime selection | Exact frame type identity plus RP golden-byte/registration tests; unrun |
| Root `ProcessLiveness`, `process_same_executable_path`, and `platform::process::{ProcessInspectError, ProcessInspectErrorKind}` | Broker-free root `running_process::{ProcessLiveness, process_same_executable_path, ProcessInspectError, ProcessInspectErrorKind}` | Resolved: KA directly aliases canonical inspection types and deleted its copied Linux/macOS/Windows inspect, liveness, and PID-signal implementations. The canonical fallible `has_exited() -> io::Result<bool>` preserves observation failures instead of falsely latching them as an exit. Path comparison only canonicalizes or compares original spellings; it makes no generation/hash assurance. PID-only mutation remains unexported | Default process surface; no broker/client feature | Bidirectional type assignment and exact fallible method-signature checks; native liveness error/latch/control regressions live with RP and remain unrun |
| `platform::fs::{LinkKind, symlink_file, hard_link_count, classify}` | zccache-platform's Linux/macOS/Windows `fs/links.rs` implemented the same generic host capability, but no public substrate counterpart exists | KA now owns the generic capability: Unix link counts follow metadata and classification uses no-follow metadata; Windows opens link-count handles with share-delete, distinguishes name-surrogate symlinks from other reparse points, and treats unreadable reparse tags conservatively as `Reparse`. This is a new KA semantic API, not a copied facade enum or substrate re-export | `fs`; zccache staged/root safety and hardlink materialization | Authored hard-link-count, regular-entry, Unix no-follow symlink tests; Windows junction/reparse and symlink privilege tests remain unrun |
| `platform::executable::file_name_os` | zccache-platform's three target `native_name(&OsStr) -> OsString` helpers | KA now owns the lossless host-name capability. Unix clones the supplied OS string unchanged; Windows appends `.exe` only when no extension is present, preserving arbitrary existing extensions and never creating `.exe.exe`. The existing Windows string helper retains its distinct always-append bare-name contract | Default executable surface; zccache program discovery with non-Unicode names | Authored OS-string identity and Windows extension-idempotence tests; not yet run |
| `platform::fs::{open_shared_append, sync_directory_if_supported}` | zccache-platform's three target `fs/durability.rs` helpers | KA now owns this generic durability capability. The existing `sync_directory` retains its stricter cross-host contract (including Windows path validation); the explicitly portable variant fsyncs Unix directories and is an unconditional documented no-op on Windows. Shared append never truncates bytes and, on Windows, permits read/write/delete sharing so rename can proceed while the handle is held | `fs`; zccache lifecycle logs and atomic replacement | Authored append-preservation, rename-with-held-handle, and host-specific sync-contract tests; not yet run |
| `platform::fs::{VolumeIdentity, volume_identity, volume_identity_u128, file_id_width, allocated_bytes}` | zccache-platform's three target `fs/volume.rs` helpers | KA now owns opaque comparable volume identity plus the explicitly raw `u128` form. Unix preserves `st_dev`, 64-bit file-id width, and `st_blocks * 512`; Windows opens with zero access plus backup semantics, keeps the full `FileIdInfo` 64-bit volume serial with legacy fallback, and preserves `GetCompressedFileSizeW`'s immediate-last-error sentinel/fallback rule | `fs`; zccache cache placement and allocation accounting | Authored same-volume equality/raw-value and width/allocation tests, plus Windows directory/readonly, high-word, sentinel, and fallback tests; not yet run. Product hard-link limits are deliberately not presented as universal host guarantees |
| `platform::fs::{create_symlink, remove_link, is_link_or_reparse, hard_link_count_file}` | soldr-platform's target `fs/links.rs::{create, remove, is_link_or_reparse, hard_link_count}` | KA now owns explicit link creation/removal and metadata/held-handle inspection beside the pre-existing path helpers. Windows chooses file versus directory reparse flavor and removes file then directory links; Unix creation does not need the kind bit and removal intentionally leaves file-type validation to callers | `fs`; Soldr staged directory links | Authored held-object hard-link-count and dangling-directory-link creation/removal tests; not yet run. Soldr keeps archive replay, containment, and copy-fallback policy |
| Platform native helpers (host, executable, fs, IPC, process inspection) | `src/platform/**` is facade-owned native HAL, with some substrate calls below private adapters | Not a `running-process` duplicate audit candidate unless a concrete public substrate equivalent and identical contract is demonstrated. Do not export private platform crate symbols | Capability-specific feature gates | Per-capability contract and feature-isolation checks |

The initial search also examined the rest of `src/lib.rs`, `src/async_engine.rs`,
hashing, archive, HTTP, filesystem, profiling, allocator, crash, snapshot,
symbolization, terminal, Wasm, and webview modules for `running_process` imports.
That search does not clear these modules of duplication: copied code may have
no backend import. The broader #1/#5 migration still requires capability-by-
capability source comparison and ownership resolution. Similar third-party APIs
are not automatically eligible for public re-export.

## Downstream integration evidence (unvalidated)

`foreground` is now a whole-module namespace re-export from `running-process`,
which re-exports the native substrate's module. It provides `status`,
`output`, `spawn`, and Unix `exec` on caller-configured standard-library commands without cloning the
command or recreating its native configuration. This is deliberately distinct
from contained, bounded, and detached spawning: explicit stdio overrides,
inheritable descriptors, launch hooks, and native exit status are preserved;
no timeout, capture cap, owner-death policy, or independent placement is added.
The default formatter runner, wrapper passthrough, and Meson setup in zccache
now use this boundary; the public formatter embedding callback stays unchanged.
Compile-signature/module-alias checks and native behavioral tests are authored
but unrun. Windows handle/console and macOS group/descriptor acceptance remain
missing; source delegation alone is not platform acceptance evidence.

The executable-name migration now exposes `file_name_os` as zccache's
`native_name` through a renamed direct re-export. This lossless API preserves
existing extensions on Windows and all OS-string bytes/code units. It must not
be confused with the older `file_name(&str)` contract: on Windows that helper
still appends `.exe` to the supplied bare-name string, even when the string
contains a dot. Regression tests for both contracts are authored, not run.

Retained daemon control closes the verification-to-signal PID-reuse window,
but does not prove that a persisted manifest describes the same process birth:
the current records do not supply that complete proof. Path/hash checks also
do not establish immutable loaded-image identity after in-place replacement.
Cleanup callers must use fallible `has_exited` on the retained handle and only
remove live endpoint state after confirmed termination; `!is_alive()` can
flatten an observation error and is not equivalent evidence. macOS control
remains unsupported rather than falling back to a numeric PID signal.

Root `ProcessPriority`, `async_process::ProcessPriority`, and `SpawnAdmission` directly re-export native
substrate scheduling intent and actor-thread spawn exclusion. Priority offers
separate strict and best-effort builder methods; admission retains a caller
permit across synchronous native creation without holding a non-Send guard
across the application's await. Cross-namespace identity and builder-signature
tests are authored, not executed. Callback blocking/cancellation behavior and
real materialization races remain validation requirements.

`async_process::ProcessTreeKill` is likewise canonical. Explicit
`AsyncProcessSessionControl::kill_tree(Duration)` distinguishes a confirmed
captured-tree sweep from direct-process-only fallback. The opt-in
`AsyncProcessSessionOptions::kill_tree_on_drop` defaults to `false`; today its
sweep only runs before the root is reaped and is a snapshot rather than ongoing
descendant containment. No caller may represent it as cleanup of descendants
created later or surviving after root reaping.

Fbuild now directly re-exports canonical `StdioSource` and renames
`EnvironmentPolicy` to `DetachedEnvironment`. Its three identical detached
launch adapters have been consolidated into the neutral adapter, which passes
borrowed files to canonical `DaemonStdioSource::File` instead of duplicating
native handle conversion. Its shared subprocess runner uses canonical capture,
session, passthrough, and executable-busy retry paths. These changes do not prove
that every fbuild process caller or its remaining native platform layer has
migrated. Windows' product exit-code fallback remains deliberately separate.

Zccache's `available_parallelism` is a direct facade re-export. The remaining
zccache and Soldr capability migrations are not cleared by that isolated change
or by removal of direct running-process dependencies. Full native ownership,
runtime/IPC migration, wire preservation, and release validation remain gates.

## Ordered follow-ups

Soldr's remaining link helpers have different observation boundaries from the
initial path-based filesystem API. Its `hard_link_count(&File)` queries an
already-open object, while `is_link_or_reparse(&Metadata)` classifies a caller's
metadata snapshot. Neither can be replaced by reopening a path without adding
a race and changing error behavior. Canonical capability work must preserve
these held-file/snapshot arguments. Directory-link creation also requires the
caller-supplied directory intent on Windows; dangling-link removal must not
inspect or remove the referent. Archive replay and copy fallback remain
separate consumer policy, not justification for duplicating native link APIs.

The embedded runtime migration remains a separate closure gate. Source review
initially found `zccache-daemon-core::embedded::RuntimeHooks::handle` and
`AuditSink::start` accepting `tokio::runtime::Handle`. Current source replaces
those signatures with `async_engine::RuntimeHandle`, and the audit writer uses
canonical bounded and oneshot channels plus explicit task detachment. This is
source progress, not proof that all callers and task owners have migrated.
Removal of Tokio imports from `zccache-core` alone does not close this
requirement. The replacement must preserve caller-selected executor ownership
and explicitly retain or detach tasks:
dropping `async_engine::Task` cancels work, whereas dropping a Tokio join handle
does not. The selected-versus-ambient runtime regression has been authored in
`src/async_engine.rs`, but has not been executed. Audit writer queue pressure,
flush acknowledgment, and shutdown ordering remain consumer validation gates.

1. Complete #189 and publish the substrate's canonical independent-spawn API.
   Replace the temporary path with the exact released version, remove the
   nested checkout, and retain the identity/policy tests.
2. Define a substrate-owned frame-v1 compatibility contract that preserves raw
   unknown values and trace bytes. Migrate `daemon_frame_v1` only after golden
   wire tests demonstrate no protocol change.
3. Decide v1 registration's compatibility/error surface with current users;
   expose it from `running-process` and migrate the confirmed subset.
4. Make the explicit v2 registration compatibility choice (limited
   shared-broker facade versus wider substrate builder) before aliasing any
   type. This is source-breaking if product policy moves.
5. Audit process-session stream enums after extracting facade owner/lifecycle
   policy. Migrate only values with identical traits and errors.
6. Revisit daemon identity only after endpoint and product-frame ownership is
   independently settled. It must not turn a frozen payload boundary into a
   generic broker re-export.

No row permits an automatic third-party re-export, a direct dependency on a
substrate-private crate, a reverse edge, a blanket `pub use running_process::*`,
or a default-feature expansion. Each batch needs a release/consumer migration
note because an alias can remove facade-only methods or change serialization
and trait surfaces even while it preserves Rust type identity.
