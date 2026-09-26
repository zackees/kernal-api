import unittest
from pathlib import Path

from ci.prune_obsolete_cook_caches import (
    BUDGET_BYTES,
    Cache,
    cache_version,
    GitHub,
    prune,
    require_single_current_generation,
    stale_cook_bases,
    version_tuple,
)


class CookCacheRetentionTests(unittest.TestCase):
    def test_only_old_main_cook_bases_are_prunable(self):
        caches = [
            Cache(1, "cook-base-v2-linux-x64-soldrv0.9.22", "refs/heads/main", 3),
            Cache(2, "cook-base-v2-linux-x64-soldrv0.9.23", "refs/heads/main", 5),
            Cache(3, "cook-base-v2-linux-arm64-soldrv0.9.22", "refs/pull/99/merge", 7),
            Cache(4, "solo-toolchain-v3-linux-x64-soldrv0.9.22", "refs/heads/main", 11),
            Cache(5, "cook-base-v2-linux-x64-soldrv0.9.24", "refs/heads/main", 13),
        ]

        self.assertEqual(stale_cook_bases(caches, "0.9.23"), [caches[0]])

    def test_current_generation_guard_rejects_mixed_versions(self):
        caches = [
            Cache(1, "cook-base-v2-linux-x64-soldrv0.9.23", "refs/heads/main", 3),
            Cache(2, "cook-base-v2-linux-arm64-soldrv0.9.22", "refs/heads/main", 7),
        ]

        with self.assertRaisesRegex(ValueError, "expected only 0.9.23"):
            require_single_current_generation(caches, "0.9.23")

    def test_current_generation_guard_preserves_distinct_shapes(self):
        caches = [
            Cache(1, "cook-base-v2-linux-x64-soldrv0.9.23", "refs/heads/main", 3),
            Cache(2, "cook-base-v2-linux-arm64-soldrv0.9.23", "refs/heads/main", 7),
        ]

        require_single_current_generation(caches, "0.9.23")

    def test_unparseable_old_cache_is_never_pruned(self):
        cache = Cache(1, "cook-base-v2-linux-x64", "refs/heads/main", 3)

        with self.assertRaisesRegex(ValueError, "cannot safely classify"):
            stale_cook_bases([cache], "0.9.23")

    def test_prune_refuses_to_delete_old_generation_without_current_replacement(self):
        class FakeGitHub:
            def __init__(self):
                self.entries = [
                    Cache(1, "cook-base-v2-linux-x64-soldrv0.9.22", "refs/heads/main", 900)
                ]
                self.deletions = []

            def caches(self):
                return list(self.entries)

            def delete_cache(self, cache_id):
                self.deletions.append(cache_id)
                self.entries = [entry for entry in self.entries if entry.cache_id != cache_id]
                return True

        api = FakeGitHub()
        with self.assertRaisesRegex(ValueError, "no main cook-base exists"):
            prune(api, "0.9.23", apply=True)
        self.assertEqual(api.deletions, [])
        self.assertEqual(api.entries[0].cache_id, 1)

    def test_versions_and_budget_use_gib_units(self):
        self.assertEqual(version_tuple("v0.9.23"), (0, 9, 23))
        self.assertEqual(cache_version("cook-base-v2-linux-x64-soldrv0.9.23-xlinux"), "0.9.23")
        self.assertEqual(BUDGET_BYTES, 10_200_547_328)

    def test_prune_deletes_only_obsolete_main_cook_bases(self):
        class FakeGitHub:
            def __init__(self):
                self.entries = [
                    Cache(1, "cook-base-v2-linux-x64-soldrv0.9.22", "refs/heads/main", 900),
                    Cache(2, "cook-base-v2-linux-x64-soldrv0.9.23", "refs/heads/main", 100),
                    Cache(3, "cook-base-v2-linux-arm64-soldrv0.9.23", "refs/heads/main", 200),
                    Cache(4, "cook-base-v2-linux-x64-soldrv0.9.22", "refs/pull/99/merge", 300),
                    Cache(5, "solo-toolchain-v3-linux-x64-soldrv0.9.22", "refs/heads/main", 400),
                ]

            def caches(self):
                return list(self.entries)

            def delete_cache(self, cache_id):
                self.entries = [entry for entry in self.entries if entry.cache_id != cache_id]
                return True

            def usage_bytes(self):
                return sum(entry.size for entry in self.entries)

        api = FakeGitHub()
        deleted, reclaimed = prune(api, "0.9.23", apply=True)

        self.assertEqual((deleted, reclaimed), (1, 900))
        self.assertEqual([cache.cache_id for cache in api.entries], [2, 3, 4, 5])

    def test_concurrent_delete_404_is_idempotent_and_inventory_is_rechecked(self):
        from unittest.mock import patch

        from urllib.error import HTTPError

        api = GitHub("zackees/kernal-api", "test-token")
        missing = HTTPError("https://api.github.com/cache/1", 404, "Not Found", None, None)
        with patch("ci.prune_obsolete_cook_caches.urlopen", side_effect=missing):
            self.assertFalse(api.delete_cache(1))

        class ConcurrentGitHub:
            def __init__(self):
                self.entries = [
                    Cache(1, "cook-base-v2-linux-x64-soldrv0.9.22", "refs/heads/main", 900),
                    Cache(2, "cook-base-v2-linux-x64-soldrv0.9.23", "refs/heads/main", 10),
                ]
                self.list_count = 0

            def caches(self):
                self.list_count += 1
                return list(self.entries)

            def delete_cache(self, cache_id):
                # Another cleanup already deleted the candidate after our snapshot.
                self.entries = [entry for entry in self.entries if entry.cache_id != cache_id]
                return False

            def usage_bytes(self):
                return sum(entry.size for entry in self.entries)

        concurrent = ConcurrentGitHub()
        with patch("ci.prune_obsolete_cook_caches.time.sleep"):
            deleted, reclaimed = prune(concurrent, "0.9.23", apply=True)
        self.assertEqual((deleted, reclaimed), (1, 900))
        self.assertGreaterEqual(concurrent.list_count, 2)
        self.assertEqual([cache.cache_id for cache in concurrent.entries], [2])

    def test_live_budget_uses_larger_of_usage_and_list(self):
        class FakeGitHub:
            def __init__(self):
                self.entries = [
                    Cache(1, "cook-base-v2-linux-x64-soldrv0.9.23", "refs/heads/main", 10),
                ]

            def caches(self):
                return self.entries

            def usage_bytes(self):
                return BUDGET_BYTES + 1

        # The usage endpoint is authoritative when it is higher than the list.
        from unittest.mock import patch

        with patch("ci.prune_obsolete_cook_caches.time.sleep"):
            from ci.prune_obsolete_cook_caches import settled_usage

            usage, endpoint, listed = settled_usage(
                FakeGitHub(), "0.9.23", polls=1, interval=0
            )
        self.assertEqual((usage, endpoint, listed), (BUDGET_BYTES + 1, BUDGET_BYTES + 1, 10))

    def test_live_budget_fails_when_either_source_exceeds_cap(self):
        from unittest.mock import patch

        for endpoint, listed in (
            (BUDGET_BYTES + 1, 10),
            (10, BUDGET_BYTES + 1),
        ):
            with self.subTest(endpoint=endpoint, listed=listed):
                class FakeGitHub:
                    def caches(self):
                        return [
                            Cache(
                                1,
                                "cook-base-v2-linux-x64-soldrv0.9.23",
                                "refs/heads/main",
                                listed,
                            )
                        ]

                    def usage_bytes(self):
                        return endpoint

                with patch("ci.prune_obsolete_cook_caches.time.sleep"):
                    with self.assertRaisesRegex(RuntimeError, "exceeds"):
                        prune(FakeGitHub(), "0.9.23", apply=True)

    def test_live_budget_retries_until_deleted_key_disappears_from_listing(self):
        class FakeGitHub:
            def __init__(self):
                self.read_count = 0
                self.current = Cache(
                    2, "cook-base-v2-linux-x64-soldrv0.9.23", "refs/heads/main", 10
                )
                self.stale = Cache(
                    1, "cook-base-v2-linux-x64-soldrv0.9.22", "refs/heads/main", 900
                )

            def caches(self):
                self.read_count += 1
                if self.read_count == 1:
                    return [self.current, self.stale]
                if self.read_count == 2:
                    return [self.current, self.stale]  # deletion/list lag
                return [self.current]

            def delete_cache(self, cache_id):
                self.asserted_id = cache_id

            def usage_bytes(self):
                return 910 if self.read_count < 3 else 10

        from unittest.mock import patch

        api = FakeGitHub()
        with patch("ci.prune_obsolete_cook_caches.time.sleep"):
            deleted, reclaimed = prune(api, "0.9.23", apply=True)
        self.assertEqual((deleted, reclaimed, api.asserted_id), (1, 900, 1))

    def test_cleanup_job_runs_after_all_producers_and_uses_their_soldr_version(self):
        workflow = (Path(__file__).resolve().parents[1] / ".github/workflows/ci.yml").read_text()

        self.assertIn("soldr_version: ${{ steps.setup_soldr.outputs.soldr-version }}", workflow)
        self.assertIn("needs: [linux, build, test, dylints, full-coverage]", workflow)
        self.assertIn("github.event_name == 'push'", workflow)
        self.assertIn("github.ref == 'refs/heads/main'", workflow)
        self.assertIn("--version \"${SOLDR_VERSION}\" --apply", workflow)


if __name__ == "__main__":
    unittest.main()
