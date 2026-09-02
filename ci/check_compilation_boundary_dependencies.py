#!/usr/bin/env python3
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
    ("wasm-sketch-host", "wasmtime"),
    ("ipc", "interprocess"),
    ("tokio-console", "console-subscriber"),
    ("allocator", "mimalloc-pprof"),
    ("fs-watch", "notify"),
)


def tree(features: str) -> set[str]:
    command = [
        "soldr",
        "cargo",
        "tree",
        "--locked",
        "--no-default-features",
        "--prefix",
        "none",
    ]
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
    for feature, package in CASES:
        if package in default_graph:
            failures.append(f"RED failed: default graph unexpectedly contains {package}")
        enabled_graph = tree(feature)
        if package not in enabled_graph:
            failures.append(f"GREEN failed: --features {feature} omits {package}")
    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    print("dependency isolation RED -> GREEN checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
