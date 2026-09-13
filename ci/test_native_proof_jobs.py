"""Keep the two real six-host proofs on independent job deadlines."""

import re
import unittest
from pathlib import Path

WORKFLOW = Path(__file__).resolve().parents[1] / ".github/workflows/ci.yml"
HOSTS = {
    ("ubuntu-24.04", "x86_64-unknown-linux-gnu"),
    ("ubuntu-24.04-arm", "aarch64-unknown-linux-gnu"),
    ("macos-15-intel", "x86_64-apple-darwin"),
    ("macos-15", "aarch64-apple-darwin"),
    ("windows-2025", "x86_64-pc-windows-msvc"),
    ("windows-11-arm", "aarch64-pc-windows-msvc"),
}


class NativeProofJobsTests(unittest.TestCase):
    def job(self, name):
        text = WORKFLOW.read_text(encoding="utf-8")
        match = re.search(
            rf"^  {re.escape(name)}:\n(.*?)(?=^  [\w-]+:|\Z)",
            text,
            re.MULTILINE | re.DOTALL,
        )
        self.assertIsNotNone(match, f"missing independent job {name}")
        return match.group(1)

    def test_both_jobs_retain_all_six_native_hosts(self):
        for name in ("wasm-archive-native", "wasm-tauri-screenshot-native"):
            with self.subTest(job=name):
                job = self.job(name)
                pairs = re.findall(r"- os: (\S+)\s+target: (\S+)", job)
                self.assertEqual(set(pairs), HOSTS)
                self.assertEqual(len(pairs), len(HOSTS))
                self.assertIn("fail-fast: false", job)
                self.assertIn("timeout-minutes: 30", job)
                self.assertNotIn("continue-on-error:", job)
                self.assertNotIn("needs:", job)

    def test_archive_job_is_self_contained_and_screenshot_budget_is_separate(self):
        archive = self.job("wasm-archive-native")
        screenshot = self.job("wasm-tauri-screenshot-native")
        for required in (
            "toolchain: 1.95.0",
            "./examples/wasm-tauri-screenshot/build-guest.ps1 -PrepareTargetOnly",
            "./tests/screenshot-target-repair.ps1",
            "--lib authenticated_",
            "uv run --no-project ci/run_extension2_guest.py --native-target",
            "uv run --no-project -m unittest ci.test_run_extension2_guest ci.test_native_proof_jobs",
            "kernal-api-archive-build",
        ):
            self.assertIn(required, archive)
        self.assertNotIn("run_extension2_guest", screenshot)
        self.assertNotIn("--lib authenticated_", screenshot)
        self.assertNotIn("tauri-webview", archive)
        self.assertIn("--test wasm_tauri_screenshot", screenshot)


if __name__ == "__main__":
    unittest.main()
