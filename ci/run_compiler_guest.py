"""Build and execute the private compiler-cache guest on one native host."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
TEST = "wasm::compiler_dispatch::tests::compiler_actual_guest_cache_hit_restores_without_spawning_the_granted_compiler"
COMPONENT_TESTS = (
    "wasm::component_compiler::tests::actual_guest_spawns_drains_hashes_waits_and_closes",
    "wasm::component_compiler::tests::actual_guest_cache_hit_does_not_spawn_the_granted_compiler",
)
COMPONENT_ENCODER_OUTPUT = (
    "validated component: {bytes} bytes; two kernel imports; not executed\n"
    "Wasmtime 45 component compilation passed; not instantiated or executed\n"
)


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


def artifact_file(messages: str, name: str, suffix: str) -> Path:
    artifacts = []
    for line in messages.splitlines():
        message = json.loads(line)
        if message.get("reason") != "compiler-artifact" or message["target"]["name"] != name:
            continue
        artifacts.extend(
            Path(filename)
            for filename in message.get("filenames", [])
            if filename.endswith(suffix)
        )
    if len(artifacts) != 1 or not artifacts[0].is_absolute():
        raise ValueError(f"expected exactly one absolute {suffix} artifact for {name}")
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
    guest_dir = args.target_dir / "compiler-guest"
    guest_messages = run(
        [
            "soldr", "--no-cache", "cargo", "build", "--locked", "--manifest-path",
            "benchmarks/wasm-sketch/compiler-guest/Cargo.toml", "--features", "guest-proof",
            "--bin", "kernal-compiler-guest-proof", "--target", "wasm32-wasip1-threads",
            "--release", "-j1", "--target-dir", str(guest_dir), "--message-format=json",
        ],
        env=guest_env,
    )
    raw = executable(guest_messages, "kernal-compiler-guest-proof", test=False)
    admitted = raw.with_suffix(".admitted.wasm")
    shutil.copyfile(raw, admitted)
    generator_messages = run(
        [
            "soldr", "--no-cache", "cargo", "build", "--locked", "--manifest-path",
            "tools/wasm-abi-generator/Cargo.toml", "--target", args.native_target,
            "--target-dir", str(args.target_dir / "compiler-abi"), "--message-format=json", "-j1",
        ]
    )
    generator = executable(generator_messages, "kernal-api-wasm-abi-generator", test=False)
    run([str(generator), "--embed-threaded-metadata", str(admitted)])
    host_messages = run(
        [
            "soldr", "--no-cache", "cargo", "test", "--locked", "--no-default-features",
            "--features", "wasm-sketch-host", "--lib", "--no-run", "--target", args.native_target,
            "--target-dir", str(args.target_dir / "compiler-host"), "--message-format=json", "-j1",
        ]
    )
    harness = executable(host_messages, "kernal_api", test=True)
    if f"{TEST}: test" not in run([str(harness), "--list", "--ignored"]).splitlines():
        raise ValueError("compiler cache proof is missing from the native test harness")
    output = run(
        [str(harness), TEST, "--exact", "--ignored", "--nocapture"],
        env=dict(os.environ, KERNAL_COMPILER_GUEST_WASM=str(admitted)),
    )
    if not any(
        line.startswith("test result: ok. 1 passed; 0 failed; 0 ignored;")
        for line in output.splitlines()
    ):
        raise ValueError("the compiler cache proof did not execute exactly once")

    component_dir = args.target_dir / "component-compiler-guest"
    component_messages = run(
        [
            "soldr", "--no-cache", "cargo", "build", "--locked", "--manifest-path",
            "benchmarks/wasm-sketch/component-guest/Cargo.toml", "--features", "compiler-proof",
            "--target", "wasm32-unknown-unknown", "--release", "-j1", "--target-dir",
            str(component_dir), "--message-format=json",
        ],
        env=guest_env,
    )
    component_core = artifact_file(component_messages, "kernal_component_probe", ".wasm")
    encoder_messages = run(
        [
            "soldr", "--no-cache", "cargo", "build", "--locked", "--release", "--manifest-path",
            "benchmarks/wasm-sketch/component-tools/Cargo.toml", "--features", "engine-probe", "-j1",
            "--target", args.native_target, "--target-dir", str(args.target_dir / "component-tools"),
            "--message-format=json",
        ]
    )
    encoder = executable(encoder_messages, "kernal-component-tools", test=False)
    component = args.target_dir / "compiler-policy.component.wasm"
    if component.exists():
        raise ValueError("component proof output already exists; refusing a stale artifact")
    encoding_output = run([str(encoder), str(component_core), str(component)])
    expected_encoder_output = COMPONENT_ENCODER_OUTPUT.format(bytes=component.stat().st_size)
    if encoding_output != expected_encoder_output:
        raise ValueError("Component encoder did not structurally validate and engine-compile the artifact")
    component_host_messages = run(
        [
            "soldr", "--no-cache", "cargo", "test", "--locked", "--no-default-features", "--features",
            "wasm-sketch-host,wasm-component-compiler-experiment", "--lib", "--no-run", "--target",
            args.native_target, "--target-dir", str(args.target_dir / "component-host"),
            "--message-format=json", "-j1",
        ]
    )
    component_harness = executable(component_host_messages, "kernal_api", test=True)
    listed = run([str(component_harness), "--list", "--ignored"])
    for test in COMPONENT_TESTS:
        if f"{test}: test" not in listed.splitlines():
            raise ValueError(f"Component compiler proof is missing from the native test harness: {test}")
        result = run(
            [str(component_harness), test, "--exact", "--ignored", "--nocapture"],
            env=dict(os.environ, KERNAL_COMPONENT_COMPILER_WASM=str(component)),
        )
        if not any(
            line.startswith("test result: ok. 1 passed; 0 failed; 0 ignored;")
            for line in result.splitlines()
        ):
            raise ValueError(f"Component compiler proof did not execute exactly once: {test}")


if __name__ == "__main__":
    main()
