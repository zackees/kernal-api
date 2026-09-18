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


if __name__ == "__main__":
    unittest.main()
