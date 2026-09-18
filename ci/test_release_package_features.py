"""Release verification must exercise the registry package, not only patches."""

import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class ReleasePackageFeatures(unittest.TestCase):
    def test_registry_package_verifies_all_advertised_features(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        commands = [line.strip() for line in workflow.splitlines()]
        # The release packages through ci/crate_release.py, without
        # `--no-verify`, so Cargo builds the extracted package itself...
        self.assertIn(
            "run: uv run --no-project --python 3.12 ci/crate_release.py package",
            commands,
        )
        # ...with every advertised feature.
        script = (ROOT / "ci/crate_release.py").read_text()
        self.assertIn(
            'PACKAGE = ["soldr", "cargo", "package", "--locked", "--all-features", "--allow-dirty"]',
            script,
        )


if __name__ == "__main__":
    unittest.main()
