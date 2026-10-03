"""The CI host-facts facade used to refuse emulated native attestations."""

import platform
from dataclasses import dataclass


@dataclass(frozen=True)
class NativeHost:
    system: str
    machine: str
    docker_platform: str


def native_host(docker_platform: str) -> NativeHost:
    return NativeHost(platform.system(), platform.machine(), docker_platform)
