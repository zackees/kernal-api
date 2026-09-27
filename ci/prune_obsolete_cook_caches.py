"""Retire obsolete or intentionally disabled main-branch cook bases.

Cook-base keys include the Soldr version and use exact-only restore. Once all
main-branch producers use a newer Soldr release, older-version cook bases are
unreachable and only consume the repository Actions cache quota. Other cache
families, refs, active current-version shapes, and future-version keys are
never deleted. The five exact cross-target shapes in `ci.yml` are also retired
after their producer opts out of cook; the native `xlinux` cook remains as a
required current-generation sentinel.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
from dataclasses import dataclass
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

GIB = 1024**3
BUDGET_BYTES = 19 * GIB // 2  # 9.5 GiB leaves room for ordinary cache growth.
MAIN_REF = "refs/heads/main"
COOK_BASE_PREFIX = "cook-base-v2-"
VERSION_RE = re.compile(r"(?:^|-)soldrv(?P<version>\d+\.\d+\.\d+)(?:-|$)")
DISABLED_CROSS_TARGETS = frozenset(
    {
        "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
        "x86_64-pc-windows-msvc",
        "aarch64-pc-windows-msvc",
    }
)


@dataclass(frozen=True)
class Cache:
    cache_id: int
    key: str
    ref: str
    size: int


def version_tuple(value: str) -> tuple[int, int, int]:
    match = re.fullmatch(r"v?(\d+)\.(\d+)\.(\d+)", value.strip())
    if match is None:
        raise ValueError(f"invalid Soldr version: {value!r}")
    return tuple(int(part) for part in match.groups())


def cache_version(key: str) -> str | None:
    match = VERSION_RE.search(key)
    return match.group("version") if match else None


def stale_cook_bases(caches: list[Cache], current_version: str) -> list[Cache]:
    current = version_tuple(current_version)
    candidates = []
    for cache in caches:
        if cache.ref != MAIN_REF or not cache.key.startswith(COOK_BASE_PREFIX):
            continue
        found = cache_version(cache.key)
        if found is None:
            raise ValueError(
                f"cannot safely classify cook-base cache {cache.cache_id}: {cache.key}"
            )
        version = version_tuple(found)
        if version < current:
            candidates.append(cache)
    return candidates


def retired_cross_target_cook_bases(
    caches: list[Cache], current_version: str
) -> list[Cache]:
    """Select only current-version main cook shapes whose producer opted out."""
    current = version_tuple(current_version)
    candidates = []
    for cache in caches:
        if cache.ref != MAIN_REF or not cache.key.startswith(COOK_BASE_PREFIX):
            continue
        found = cache_version(cache.key)
        if found is None:
            raise ValueError(
                f"cannot safely classify cook-base cache {cache.cache_id}: {cache.key}"
            )
        if version_tuple(found) != current:
            continue
        if any(
            cache.key.endswith(f"-xbuild-{target}") for target in DISABLED_CROSS_TARGETS
        ):
            candidates.append(cache)
    return candidates


def require_current_generation_present(
    caches: list[Cache], current_version: str
) -> None:
    """Permit old-generation pruning only when a usable current generation exists."""
    current = version_tuple(current_version)
    versions = set()
    for cache in caches:
        if cache.ref != MAIN_REF or not cache.key.startswith(COOK_BASE_PREFIX):
            continue
        found = cache_version(cache.key)
        if found is None:
            raise ValueError(
                f"cannot safely classify cook-base cache {cache.cache_id}: {cache.key}"
            )
        parsed = version_tuple(found)
        if parsed > current:
            raise ValueError(
                f"future Soldr cook-base generation {found} exceeds current {current_version}"
            )
        versions.add(parsed)
    if current not in versions:
        raise ValueError(
            f"refusing to prune old cook bases: no main cook-base exists for Soldr {current_version}"
        )


def require_single_current_generation(
    caches: list[Cache], current_version: str
) -> None:
    current = version_tuple(current_version)
    versions = {
        cache_version(cache.key)
        for cache in caches
        if cache.ref == MAIN_REF and cache.key.startswith(COOK_BASE_PREFIX)
    }
    if None in versions:
        raise ValueError("an unparseable main cook-base key remains")
    parsed = {version_tuple(value) for value in versions if value is not None}
    if parsed != {current}:
        actual = sorted(".".join(map(str, version)) for version in parsed)
        raise ValueError(
            f"main cook-base generations are {actual}, expected only {current_version}"
        )


def require_retained_native_cook_base(
    caches: list[Cache], current_version: str, candidates: list[Cache]
) -> None:
    """Prove a current native Linux producer remains before deleting any cache."""
    current = version_tuple(current_version)
    candidate_ids = {cache.cache_id for cache in candidates}
    for cache in caches:
        if cache.ref != MAIN_REF or cache.cache_id in candidate_ids:
            continue
        if not cache.key.startswith(COOK_BASE_PREFIX):
            continue
        found = cache_version(cache.key)
        if found is None:
            raise ValueError(
                f"cannot safely classify cook-base cache {cache.cache_id}: {cache.key}"
            )
        if version_tuple(found) == current and cache.key.endswith("-xlinux"):
            return
    raise ValueError(
        f"refusing to prune cook bases: no retained native xlinux cook base exists for Soldr {current_version}"
    )


def require_disabled_cross_cooks_retired(
    caches: list[Cache], current_version: str
) -> None:
    remaining = retired_cross_target_cook_bases(caches, current_version)
    if remaining:
        details = ", ".join(f"{cache.cache_id}:{cache.key}" for cache in remaining)
        raise ValueError(
            f"disabled cross-target cook bases remain in inventory: {details}"
        )


class GitHub:
    def __init__(self, repository: str, token: str):
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
            raise ValueError(f"invalid repository: {repository!r}")
        self.base = f"https://api.github.com/repos/{repository}"
        self.token = token

    def request(self, path: str, *, method: str = "GET") -> dict:
        request = Request(
            self.base + path,
            method=method,
            headers={
                "Accept": "application/vnd.github+json",
                "Authorization": f"Bearer {self.token}",
                "X-GitHub-Api-Version": "2022-11-28",
            },
        )
        try:
            with urlopen(request, timeout=30) as response:
                payload = response.read()
        except HTTPError as exc:
            if method == "DELETE" and exc.code == 404:
                return {"already_absent": True}
            raise RuntimeError(f"GitHub API {method} {path} failed: {exc}") from exc
        except URLError as exc:
            raise RuntimeError(f"GitHub API {method} {path} failed: {exc}") from exc
        if not payload:
            return {}
        value = json.loads(payload)
        if not isinstance(value, dict):
            raise TypeError(f"GitHub API returned unexpected data for {path}")
        return value

    def caches(self) -> list[Cache]:
        result = []
        page = 1
        while True:
            response = self.request(f"/actions/caches?per_page=100&page={page}")
            entries = response.get("actions_caches", [])
            if not isinstance(entries, list):
                raise TypeError("GitHub API cache listing had no actions_caches array")
            for entry in entries:
                result.append(
                    Cache(
                        cache_id=int(entry["id"]),
                        key=str(entry["key"]),
                        ref=str(entry["ref"]),
                        size=int(entry["size_in_bytes"]),
                    )
                )
            if len(entries) < 100:
                return result
            page += 1

    def usage_bytes(self) -> int:
        response = self.request("/actions/cache/usage")
        return int(response["active_caches_size_in_bytes"])

    def delete_cache(self, cache_id: int) -> bool:
        response = self.request(f"/actions/caches/{cache_id}", method="DELETE")
        return not response.get("already_absent", False)


def settled_usage(
    api: GitHub,
    current_version: str,
    *,
    polls: int = 6,
    interval: int = 10,
) -> tuple[int, int, int]:
    """Return (max usage, endpoint usage, listed usage) after deletion settles."""
    last = (0, 0, 0)
    last_policy_error: ValueError | None = None
    for attempt in range(polls):
        endpoint = api.usage_bytes()
        caches = api.caches()
        listed = sum(cache.size for cache in caches)
        last = (max(endpoint, listed), endpoint, listed)
        try:
            require_single_current_generation(caches, current_version)
            require_retained_native_cook_base(caches, current_version, [])
            require_disabled_cross_cooks_retired(caches, current_version)
            last_policy_error = None
        except ValueError as exc:
            last_policy_error = exc
        if last[0] <= BUDGET_BYTES and last_policy_error is None:
            return last
        if attempt + 1 < polls:
            time.sleep(interval)
    if last_policy_error is not None:
        raise last_policy_error
    return last


def prune(api: GitHub, current_version: str, *, apply: bool) -> tuple[int, int]:
    before = api.caches()
    require_current_generation_present(before, current_version)
    candidates = stale_cook_bases(
        before, current_version
    ) + retired_cross_target_cook_bases(before, current_version)
    require_retained_native_cook_base(before, current_version, candidates)
    reclaimed = sum(cache.size for cache in candidates)
    for cache in candidates:
        if apply:
            deleted = api.delete_cache(cache.cache_id)
            state = "deleted" if deleted else "already absent"
        else:
            state = "would delete"
        print(
            f"{state} id={cache.cache_id} ref={cache.ref} "
            f"size={cache.size} key={cache.key}"
        )

    if apply:
        usage, endpoint, listed = settled_usage(api, current_version)
    else:
        remaining = [c for c in before if c not in candidates]
        require_single_current_generation(remaining, current_version)
        require_retained_native_cook_base(remaining, current_version, [])
        require_disabled_cross_cooks_retired(remaining, current_version)
        endpoint = api.usage_bytes()
        listed = sum(cache.size for cache in before)
        usage = max(endpoint - reclaimed, listed - reclaimed)
        print(
            f"dry-run current: max(endpoint={endpoint}, listed={listed}) bytes; "
            f"projected after deletion={usage} bytes"
        )
    print(
        f"cache budget: max(endpoint={endpoint}, listed={listed})={usage} bytes; "
        f"limit={BUDGET_BYTES} bytes; retired={len(candidates)} entries/{reclaimed} bytes"
    )
    if usage > BUDGET_BYTES:
        raise RuntimeError(
            f"Actions cache usage {usage} exceeds {BUDGET_BYTES}-byte budget"
        )
    return len(candidates), reclaimed


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--version", required=True, help="Soldr version from the producer action output"
    )
    parser.add_argument(
        "--apply", action="store_true", help="actually delete obsolete main cook bases"
    )
    args = parser.parse_args()
    token = os.environ.get("GITHUB_TOKEN", "")
    repository = os.environ.get("GITHUB_REPOSITORY", "")
    if not token or not repository:
        parser.error("GITHUB_TOKEN and GITHUB_REPOSITORY must be set")
    try:
        prune(GitHub(repository, token), args.version, apply=args.apply)
    except (RuntimeError, TypeError, ValueError, KeyError, json.JSONDecodeError) as exc:
        print(f"::error::{exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
