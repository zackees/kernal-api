"""Build and execute the synthetic streaming guest on an explicit native host."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
TEST = "wasm::archive_input_guest_tests::authenticated_input_actual_guest_streams_large_zip_and_rejects_bad_tag_or_nonce"


def run(arguments: list[str], *, env: dict[str, str] | None = None) -> str:
    print("Running: " + " ".join(arguments), flush=True)
    try:
        return subprocess.run(
            arguments, cwd=REPO, env=env, check=True, text=True, stdout=subprocess.PIPE
        ).stdout
    except subprocess.CalledProcessError as error:
        print(error.stdout or "", end="", flush=True)
        raise


def executable(messages: str, name: str, *, test: bool) -> Path:
    artifacts = []
    for line in messages.splitlines():
        message = json.loads(line)
        if (
            message.get("reason") == "compiler-artifact"
            and message["target"]["name"] == name
            and message["profile"]["test"] == test
            and message.get("executable")
        ):
            artifacts.append(Path(message["executable"]))
    if len(artifacts) != 1 or not artifacts[0].is_absolute():
        raise ValueError(f"expected exactly one absolute executable for {name}")
    return artifacts[0]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-target", required=True)
    parser.add_argument("--target-dir", type=Path, required=True)
    args = parser.parse_args()
    if not args.target_dir.is_absolute():
        parser.error("--target-dir must be absolute caller-owned build storage")
    host = run(["soldr", "rustc", "-vV"])
    if f"host: {args.native_target}" not in host.splitlines():
        raise ValueError("the selected target is not this runner's native Rust host")
    guest_env = dict(os.environ, SOLDR_LINKER="default")
    guest_dir = args.target_dir / "extension2-stream"
    messages = run(
        [
            "soldr",
            "--no-cache",
            "cargo",
            "build",
            "--locked",
            "--manifest-path",
            "benchmarks/wasm-sketch/extension2-guest/Cargo.toml",
            "--features",
            "guest-proof",
            "--bin",
            "kernal-extension2-guest-proof",
            "--target",
            "wasm32-wasip1-threads",
            "--release",
            "-j1",
            "--target-dir",
            str(guest_dir),
            "--message-format=json",
        ],
        env=guest_env,
    )
    raw = executable(messages, "kernal-extension2-guest-proof", test=False)
    admitted = raw.with_suffix(".admitted.wasm")
    # Always start from the freshly compiled raw module; never relabel an old ABI.
    shutil.copyfile(raw, admitted)
    messages = run(
        [
            "soldr",
            "--no-cache",
            "cargo",
            "build",
            "--locked",
            "--manifest-path",
            "tools/wasm-abi-generator/Cargo.toml",
            "--target",
            args.native_target,
            "--target-dir",
            str(args.target_dir / "extension2-abi"),
            "--message-format=json",
            "-j1",
        ]
    )
    generator = executable(messages, "kernal-api-wasm-abi-generator", test=False)
    run([str(generator), "--embed-threaded-metadata", str(admitted)])
    messages = run(
        [
            "soldr",
            "--no-cache",
            "cargo",
            "test",
            "--locked",
            "--no-default-features",
            "--features",
            "wasm-sketch-host,archive-auth-test-support",
            "--lib",
            "--no-run",
            "--target",
            args.native_target,
            "--target-dir",
            str(args.target_dir),
            "--message-format=json",
            "-j1",
        ]
    )
    harness = executable(messages, "kernal_api", test=True)
    # Run staging and actual-guest checks from exactly the same native build.
    # A separate cache-enabled cargo test caused a second full native build
    # here and exhausted the Intel macOS job's deadline.
    staging = run([str(harness), "authenticated_", "--nocapture"])
    print(staging, end="", flush=True)
    if not re.search(
        r"^test result: ok\. [1-9][0-9]* passed; 0 failed;", staging, re.MULTILINE
    ):
        raise ValueError("authenticated staging tests did not execute successfully")
    if f"{TEST}: test" not in run([str(harness), "--list", "--ignored"]).splitlines():
        raise ValueError("streaming proof is missing from the native test harness")
    output = run(
        [str(harness), TEST, "--exact", "--ignored", "--nocapture"],
        env=dict(os.environ, KERNAL_EXTENSION2_STREAM_WASM=str(admitted)),
    )
    print(output, end="", flush=True)
    if not any(
        line.startswith("test result: ok. 1 passed; 0 failed; 0 ignored;")
        for line in output.splitlines()
    ):
        raise ValueError("the streaming proof did not execute exactly once")


if __name__ == "__main__":
    main()
