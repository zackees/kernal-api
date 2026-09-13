"""Release verification must exercise the registry package, not only patches."""

import unittest
from pathlib import Path


class ReleasePackageFeatures(unittest.TestCase):
    def test_registry_package_verifies_all_advertised_features(self) -> None:
        workflow = (
            Path(__file__).resolve().parents[1] / ".github/workflows/release.yml"
        ).read_text()
        commands = [line.strip() for line in workflow.splitlines()]
        self.assertIn("run: soldr cargo package --locked --all-features", commands)

    def test_auto_release_dispatches_the_verified_pipeline_without_registry_credentials(self) -> None:
        root = Path(__file__).resolve().parents[1]
        automatic = (root / ".github/workflows/auto-release.yml").read_text()
        release = (root / ".github/workflows/release.yml").read_text()

        self.assertIn("name: Autonomous Release", automatic)
        self.assertIn("gh release create", automatic)
        self.assertIn("gh workflow run release.yml", automatic)
        self.assertIn("cargo == '0.0.0'", automatic)
        self.assertIn('tag_commit="$(git rev-parse "refs/tags/${tag}^{}")"', automatic)
        self.assertIn('"${tag_commit}" != "${GITHUB_SHA}"', automatic)
        self.assertIn("RELEASE_TAG", release)
        self.assertIn("github.event.release.tag_name || inputs.tag", release)
        self.assertIn("vars.ENABLE_REGISTRY_PUBLISH == 'true'", release)
        self.assertIn("vars.ENABLE_PYPI_PUBLISH == 'true'", release)


if __name__ == "__main__":
    unittest.main()
