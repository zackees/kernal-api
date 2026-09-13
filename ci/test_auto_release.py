"""Contract checks for autonomous release orchestration."""

import unittest
from pathlib import Path
from unittest.mock import patch

from auto_release import main, release_tag, should_release

ROOT = Path(__file__).resolve().parents[1]


class AutoReleaseTests(unittest.TestCase):
    def test_version_detection(self):
        self.assertEqual(release_tag('[package]\nversion = "0.1.0"'), "v0.1.0")
        for version in ["0.0.0", "bad", "0.1.0\nmalicious"]:
            with self.assertRaises(ValueError):
                release_tag(f'[package]\nversion = "{version}"')
        self.assertTrue(should_release("v0.1.0", "v0.0.0", False))
        self.assertFalse(should_release("v0.1.0", "v0.1.0", False))
        self.assertFalse(should_release("v0.1.0", None, False))
        self.assertTrue(should_release("v0.1.0", "v0.1.0", True))

    def test_workflow_has_safe_automatic_and_manual_entrypoints(self):
        workflow = (ROOT / ".github/workflows/auto-release.yml").read_text()
        self.assertIn("branches: [main]", workflow)
        self.assertIn("workflow_dispatch:", workflow)
        self.assertIn("default: true", workflow)
        self.assertIn("cancel-in-progress: false", workflow)
        self.assertIn("uses: ./.github/workflows/release.yml", workflow)
        self.assertIn("startsWith(github.ref, 'refs/tags/v')", workflow)

    def test_registry_publish_is_explicitly_opt_in(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn("workflow_call:", workflow)
        self.assertNotIn("github.event.release.tag_name", workflow)
        self.assertIn("vars.PUBLISH_CRATES_IO == 'true'", workflow)
        self.assertIn("vars.PUBLISH_PYPI == 'true'", workflow)
        self.assertIn(
            "needs: [release-guard, validate-and-package, release-assets]", workflow
        )
        self.assertIn("needs: [validate-and-package, release-assets]", workflow)
        self.assertIn("!inputs.dry_run", workflow)
        self.assertNotIn("--clobber", workflow)
        self.assertIn("path: registry-packages/*", workflow)
        self.assertIn(
            "cp target/package/kernal-api-*.crate dist/* registry-packages/", workflow
        )

    def verify_source(self, tag="v0.1.0", sha="a" * 40, tagged_sha=None):
        env = {"RELEASE_TAG": tag, "RELEASE_SHA": sha, "GITHUB_SHA": "a" * 40}
        results = ["a" * 40, "" if tagged_sha is None else tag, tagged_sha]
        with (
            patch("sys.argv", ["auto_release.py", "--verify-source"]),
            patch.dict("os.environ", env),
            patch.object(Path, "read_text", return_value='[package]\nversion="0.1.0"'),
            patch("subprocess.check_output", side_effect=results),
        ):
            main()

    def test_source_guard_accepts_absent_or_matching_tag(self):
        self.verify_source()
        self.verify_source(tagged_sha="a" * 40)

    def test_source_guard_rejects_version_commit_and_existing_tag_mismatch(self):
        for arguments in [
            {"tag": "v0.2.0"},
            {"sha": "b" * 40},
            {"tagged_sha": "b" * 40},
        ]:
            with self.subTest(arguments=arguments), self.assertRaises(ValueError):
                self.verify_source(**arguments)


if __name__ == "__main__":
    unittest.main()
