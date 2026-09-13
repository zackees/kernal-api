"""Prove the feature graph has the dependency boundaries measured for #3.

The first command in every pair is the RED state: it must *not* find the
optional implementation.  The second is GREEN: enabling its owning feature
must find it.  `cargo tree`, rather than Cargo.lock, is intentional: a lockfile
contains every optional package and therefore cannot prove feature isolation.
"""

from __future__ import annotations

import subprocess
import sys

CASES = (
    ("pty", "portable-pty"),
    ("text-similarity", "strsim"),
    ("command-arguments", "shell-words"),
    ("source-cpp", "tree-sitter"),
    ("source-cpp", "tree-sitter-cpp"),
    ("wasm-sketch-host", "wasmtime"),
    ("ipc", "interprocess"),
    ("tokio-console", "console-subscriber"),
    ("allocator", "mimalloc-pprof"),
    ("fs-watch", "notify"),
    ("hash-sha256", "sha2"),
    ("http-client", "reqwest"),
    ("http-client", "hyper"),
    ("http-server", "hyper"),
    ("http-server", "http-body-util"),
    ("event-stream", "tokio-stream"),
    ("archive", "zip"),
    ("archive", "tar"),
    ("archive", "zstd"),
    ("tauri-webview", "tauri"),
    ("tauri-webview", "tauri-runtime-wry"),
    ("tauri-webview", "wry"),
)

SKETCH_AND_WEBVIEW_PACKAGES = {
    "tauri",
    "tauri-runtime",
    "tauri-runtime-wry",
    "wry",
    "webkit2gtk",
    "webview2-com",
    "wasmtime",
    "wasmparser",
    "fp-bindgen",
    "fp-bindgen-support",
    "kernal-api-v1-bindings",
}


def tree(features: str, *, normal_only: bool = False) -> set[str]:
    command = [
        "soldr",
        "cargo",
        "tree",
        "--locked",
        "--no-default-features",
        "--prefix",
        "none",
    ]
    if normal_only:
        command.extend(("--edges", "normal"))
    if features:
        command.extend(("--features", features))
    completed = subprocess.run(command, check=True, text=True, capture_output=True)
    return {
        line.split(maxsplit=1)[0]
        for line in completed.stdout.splitlines()
        if line and not line.startswith("[")
    }


def main() -> int:
    default_graph = tree("")
    failures: list[str] = []
    # TOML already builds the kernel's catalog. Only its runtime edge is opt-in.
    if "toml" in tree("", normal_only=True):
        failures.append("default runtime graph unexpectedly contains toml")
    if "toml" not in tree("config-toml", normal_only=True):
        failures.append("config-toml runtime graph omits toml")
    for label, graph in (("default", default_graph), ("full", tree("full"))):
        unexpected = sorted(graph & SKETCH_AND_WEBVIEW_PACKAGES)
        if unexpected:
            failures.append(f"{label} graph unexpectedly contains {', '.join(unexpected)}")
    if "portable-pty" in tree("terminal-input"):
        failures.append("terminal-input unexpectedly enables PTY process spawning")
    enabled_graphs: dict[str, set[str]] = {}
    # getrandom already occurs transitively in the host substrate. Prove this
    # capability activates the backend without importing unrelated facilities;
    # default absence of the package would be a false claim.
    entropy_graph = tree("secure-random")
    if "getrandom" not in entropy_graph:
        failures.append("secure-random graph omits its OS entropy backend")
    unexpected_entropy = sorted(
        (entropy_graph - default_graph)
        & (
            SKETCH_AND_WEBVIEW_PACKAGES
            | {"crash-handler", "console-subscriber", "mimalloc-pprof", "framehop"}
        )
    )
    if unexpected_entropy:
        failures.append(
            f"secure-random adds unrelated facilities: {', '.join(unexpected_entropy)}"
        )
    for feature, package in CASES:
        if package in default_graph:
            failures.append(f"RED failed: default graph unexpectedly contains {package}")
        if feature not in enabled_graphs:
            enabled_graphs[feature] = tree(feature)
        enabled_graph = enabled_graphs[feature]
        if package not in enabled_graph:
            failures.append(f"GREEN failed: --features {feature} omits {package}")
    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    print("dependency isolation RED -> GREEN checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
