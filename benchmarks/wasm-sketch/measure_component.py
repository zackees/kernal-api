"""Issue #13 component diagnostic; invoke with uv run --no-project.

Builds an isolated committed snapshot; retains all artifacts and logs. The
encoder must have engine-probe enabled and execution-probe disabled.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import subprocess
import time
from pathlib import Path

from measure_core import edit_summary, run


def edit_source(source: str, previous: int, following: int) -> str:
    before = f"if total > {previous} * 1024 * 1024 {{"
    if source.count(before) != 1:
        raise ValueError("component runtime limit anchor must occur exactly once")
    return source.replace(before, f"if total > {following} * 1024 * 1024 {{")


def validate_output(output: str, module_bytes: int) -> None:
    expected = [
        f"validated component: {module_bytes} bytes; two kernel imports; not executed",
        "Wasmtime 45 component compilation passed; not instantiated or executed",
    ]
    if output.splitlines() != expected:
        raise ValueError(
            "encoder must validate and engine-compile this exact component without execution"
        )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--encoder", required=True, type=Path)
    parser.add_argument("--edits", default=10, type=int)
    args = parser.parse_args()
    if args.edits < 10:
        parser.error("--edits must be at least 10")
    encoder = args.encoder.resolve()
    if not encoder.is_file():
        parser.error("build the engine-probe encoder first")
    repo = Path(__file__).resolve().parents[2]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    source = output / "source"
    source.mkdir()
    revision = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=repo, text=True
    ).strip()
    archive = output / "source.tar"
    with archive.open("wb") as stream:
        subprocess.run(
            ["git", "archive", revision], cwd=repo, stdout=stream, check=True
        )
    subprocess.run(["tar", "-xf", str(archive), "-C", str(source)], check=True)
    guest = source / "benchmarks/wasm-sketch/component-guest"
    guest_source = guest / "src/lib.rs"
    target = output / "target"
    artifact = target / "wasm32-unknown-unknown/release/kernal_component_probe.wasm"
    records: list[dict[str, object]] = []
    document = {
        "schema": 1,
        "status": "incomplete",
        "candidate": "component-private-probe",
        "revision": revision,
        "host": platform.platform(),
        "samples": records,
        "encoder_sha256": hashlib.sha256(encoder.read_bytes()).hexdigest(),
        "cache_mode": "soldr-disabled",
        "cache_hit_rate": None,
        "peak_compiler_rss_bytes": None,
        "limitations": [
            "shared host, not the controlled reference-host latency gate",
            "private generated API, not the same public facade as core",
            "cache hit rate and compiler memory are not collected",
            "includes encoding and engine compilation, no instantiation or execution",
            "encoder optimization profile is caller supplied; retain its build log",
        ],
    }
    previous = 64
    for index in range(args.edits + 2):
        label = "cold" if index == 0 else "noop" if index == 1 else f"edit-{index - 1}"
        if index >= 2:
            following = previous + 1
            guest_source.write_text(
                edit_source(
                    guest_source.read_text(encoding="utf-8"), previous, following
                ),
                encoding="utf-8",
            )
            previous = following
        component = output / f"{label}.wasm"
        started = time.monotonic_ns()
        build_ns, _ = run(
            [
                "soldr",
                "--no-cache",
                "cargo",
                "build",
                "--locked",
                "--release",
                "--target",
                "wasm32-unknown-unknown",
                "--target-dir",
                str(target),
                "-j",
                "1",
            ],
            guest,
            output / f"{label}-build.log",
        )
        encode_ns, result = run(
            [str(encoder), str(artifact), str(component)],
            source,
            output / f"{label}-encode.log",
        )
        module_bytes = component.stat().st_size
        validate_output(result, module_bytes)
        elapsed = time.monotonic_ns() - started
        records.append(
            {
                "label": label,
                "wall_ns": elapsed,
                "build_ns": build_ns,
                "encode_and_compile_command_ns": encode_ns,
                "module_bytes": module_bytes,
                "source_sha256": hashlib.sha256(guest_source.read_bytes()).hexdigest(),
                "module_sha256": hashlib.sha256(component.read_bytes()).hexdigest(),
            }
        )
        (output / "result.json").write_text(
            json.dumps(document, indent=2) + "\n", encoding="utf-8"
        )
        print(f"{label}: {elapsed / 1_000_000_000:.3f}s", flush=True)
    document["edit_summary"] = edit_summary(records[2:])
    document["status"] = "complete-diagnostic-only"
    (output / "result.json").write_text(
        json.dumps(document, indent=2) + "\n", encoding="utf-8"
    )


if __name__ == "__main__":
    main()
