from __future__ import annotations

import kernal_api


def test_python_and_rust_versions_are_explicit() -> None:
    assert kernal_api.__version__ == "0.1.0"
    assert kernal_api.RUST_MSRV == "1.95.0"


def test_current_ci_host_is_supported() -> None:
    status = kernal_api.compatibility()
    assert status.supported, status
    assert status.python >= (3, 10)


def test_every_release_machine_spelling_is_case_insensitive() -> None:
    for system, machine in [
        ("Linux", "x86_64"),
        ("Linux", "aarch64"),
        ("Darwin", "x86_64"),
        ("Darwin", "arm64"),
        ("Windows", "AMD64"),
        ("Windows", "ARM64"),
    ]:
        status = kernal_api._compatibility_for(system, machine, (3, 10))
        assert status.supported, status


def test_unsupported_platform_or_old_python_fails_closed() -> None:
    assert not kernal_api._compatibility_for("Plan9", "x86_64", (3, 13)).supported
    assert not kernal_api._compatibility_for("Linux", "riscv64", (3, 13)).supported
    assert not kernal_api._compatibility_for("Linux", "x86_64", (3, 9)).supported
