"""The crate check that stands between `cargo package` and crates.io."""

import io
import tarfile
import tempfile
import unittest
from pathlib import Path

import crate_release as release

VERSION = "9.9.9"
PREFIX = f"{release.CRATE}-{VERSION}/"


def make_crate(directory: Path, files: dict[str, int]) -> Path:
    crate = directory / f"{release.CRATE}-{VERSION}.crate"
    with tarfile.open(crate, "w:gz") as archive:
        for name, size in files.items():
            info = tarfile.TarInfo(name)
            info.size = size
            archive.addfile(info, io.BytesIO(b"\0" * size))
    return crate


class CrateCheckTests(unittest.TestCase):
    def setUp(self):
        self.directory = Path(tempfile.mkdtemp())

    def problems(self, files, tracked=frozenset({"Cargo.toml", "src/lib.rs"})):
        return release.crate_problems(make_crate(self.directory, files), set(tracked), VERSION)

    def test_tracked_source_and_generated_files_pass(self):
        files = {PREFIX + "Cargo.toml": 10, PREFIX + "src/lib.rs": 10}
        files.update({PREFIX + name: 10 for name in release.GENERATED})
        self.assertEqual(self.problems(files), [])

    def test_an_untracked_file_is_rejected(self):
        """v0.1.13 packed setup-soldr's cache archives from the checkout root."""
        problems = self.problems({PREFIX + "src/lib.rs": 10, PREFIX + "target.soldr-delta.tar.zst": 10})
        self.assertEqual(len(problems), 1)
        self.assertIn("target.soldr-delta.tar.zst", problems[0])
        self.assertIn("not tracked by git", problems[0])

    def test_a_crate_over_the_upload_cap_is_rejected(self):
        # Random-free zeros compress well, so shrink the cap instead of the data.
        original = release.MAX_UPLOAD_BYTES
        release.MAX_UPLOAD_BYTES = 64
        try:
            problems = self.problems({PREFIX + "src/lib.rs": 10})
        finally:
            release.MAX_UPLOAD_BYTES = original
        self.assertEqual(len(problems), 1)
        self.assertIn("crates.io accepts at most 64", problems[0])

    def test_a_file_outside_the_package_root_is_rejected(self):
        problems = self.problems({"elsewhere/src/lib.rs": 10})
        self.assertIn("outside", problems[0])

    def test_the_real_cap_is_crates_io_s(self):
        self.assertEqual(release.MAX_UPLOAD_BYTES, 10_485_760)


class BuildCompanionTests(unittest.TestCase):
    """kernal-api-build is released from the same tag and held to the same checks."""

    SPEC = release.SPECS["kernal-api-build"]
    PREFIX = f"kernal-api-build-{VERSION}/"
    TRACKED = frozenset({"crates/kernal-api-build/Cargo.toml", "crates/kernal-api-build/src/lib.rs"})

    def setUp(self):
        self.directory = Path(tempfile.mkdtemp())

    def problems(self, files):
        crate = self.directory / f"kernal-api-build-{VERSION}.crate"
        with tarfile.open(crate, "w:gz") as archive:
            for name, size in files.items():
                info = tarfile.TarInfo(name)
                info.size = size
                archive.addfile(info, io.BytesIO(b"\0" * size))
        return release.crate_problems(crate, set(self.TRACKED), VERSION, self.SPEC)

    def test_member_files_are_checked_against_their_workspace_path(self):
        files = {self.PREFIX + "Cargo.toml": 10, self.PREFIX + "src/lib.rs": 10}
        files.update({self.PREFIX + name: 10 for name in self.SPEC.generated})
        self.assertEqual(self.problems(files), [])

    def test_the_conpty_manifest_is_not_a_companion_file(self):
        """Only the facade's release generates that manifest."""
        problems = self.problems({self.PREFIX + "conpty-sidecar.sha256.toml": 10})
        self.assertEqual(len(problems), 1)
        self.assertIn("not tracked by git", problems[0])

    def test_a_root_file_is_not_mistaken_for_a_companion_file(self):
        # src/lib.rs is tracked at the root, not under crates/kernal-api-build.
        problems = release.crate_problems(
            self.make_root_named(), {"src/lib.rs"}, VERSION, self.SPEC
        )
        self.assertEqual(len(problems), 1)

    def make_root_named(self):
        crate = self.directory / "companion.crate"
        with tarfile.open(crate, "w:gz") as archive:
            info = tarfile.TarInfo(self.PREFIX + "src/lib.rs")
            info.size = 10
            archive.addfile(info, io.BytesIO(b"\0" * 10))
        return crate

    def test_a_member_is_named_to_cargo_and_the_root_is_not(self):
        self.assertEqual(self.SPEC.cargo_selector(), ["-p", "kernal-api-build"])
        self.assertEqual(release.DEFAULT.cargo_selector(), [])

    def test_the_companion_version_comes_from_its_own_manifest(self):
        self.assertEqual(release.version(self.SPEC), release.version())


if __name__ == "__main__":
    unittest.main()
