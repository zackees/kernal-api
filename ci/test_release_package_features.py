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


if __name__ == "__main__":
    unittest.main()
