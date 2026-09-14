"""Keep the real six-host proofs on independent job deadlines."""

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

    def test_all_native_proof_jobs_retain_all_six_native_hosts(self):
        # Each proof keeps its own explicit deadline; the compiler proof's cold
        # component-tool build needs longer than the screenshot proof.
        for name, timeout in (
            ("wasm-compiler-native", 60),
            ("wasm-tauri-screenshot-native", 30),
        ):
            with self.subTest(job=name):
                job = self.job(name)
                pairs = re.findall(r"- os: (\S+)\s+target: (\S+)", job)
                self.assertEqual(set(pairs), HOSTS)
                self.assertEqual(len(pairs), len(HOSTS))
                self.assertIn("fail-fast: false", job)
                self.assertIn(f"timeout-minutes: {timeout}", job)
                self.assertNotIn("continue-on-error:", job)
                self.assertNotIn("needs:", job)

    def test_compiler_and_screenshot_proofs_keep_separate_budgets(self):
        compiler = self.job("wasm-compiler-native")
        screenshot = self.job("wasm-tauri-screenshot-native")
        self.assertNotIn("--lib authenticated_", screenshot)
        self.assertIn("ci/run_compiler_guest.py --native-target", compiler)
        self.assertIn("ci.test_run_compiler_guest ci.test_native_proof_jobs", compiler)
        self.assertIn("compiler-cache", compiler)
        self.assertIn("Component", compiler)
        self.assertIn("--test wasm_tauri_screenshot", screenshot)


if __name__ == "__main__":
    unittest.main()
