"""Keep native proofs compiled on Linux and executed on all six native hosts."""

import re
import unittest
from pathlib import Path

from ci import native_proof

WORKFLOW = Path(__file__).resolve().parents[1] / ".github/workflows/ci.yml"
HOSTS = {
    ("ubuntu-24.04", "x86_64-unknown-linux-gnu"),
    ("ubuntu-24.04-arm", "aarch64-unknown-linux-gnu"),
    ("macos-15-intel", "x86_64-apple-darwin"),
    ("macos-15", "aarch64-apple-darwin"),
    ("windows-2025", "x86_64-pc-windows-msvc"),
    ("windows-11-arm", "aarch64-pc-windows-msvc"),
}
TARGETS = {target for _, target in HOSTS}
RUN_JOBS = {"wasm-compiler-native": "compiler", "wasm-tauri-screenshot-native": "screenshot"}
# Anything that would compile Rust on the host running it.
COMPILES = re.compile(r"\bsoldr\b|\bcargo\b|\brustup\b|\brustc\b|build-guest|build-threaded-smoke")


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

    def test_every_native_proof_binary_compiles_on_linux(self):
        build = self.job("native-proof-build")
        builders = re.findall(r"- builder: (\S+)\s+target: (\S+)", build)
        self.assertEqual({target for _, target in builders}, TARGETS)
        self.assertEqual(len(builders), len(TARGETS))
        for builder, target in builders:
            self.assertTrue(builder.startswith("ubuntu-"), f"{target} compiles on {builder}")
        self.assertIn("runs-on: ${{ matrix.builder }}", build)
        self.assertIn("ci/native_proof.py build --target", build)
        self.assertIn("fail-fast: false", build)
        guests = self.job("native-proof-guests")
        self.assertIn("runs-on: ubuntu-", guests)
        self.assertIn("ci/native_proof.py guests", guests)

    def test_native_hosts_only_execute_prebuilt_binaries(self):
        for name, suite in RUN_JOBS.items():
            with self.subTest(job=name):
                job = self.job(name)
                pairs = re.findall(r"- os: (\S+)\s+target: (\S+)", job)
                self.assertEqual(set(pairs), HOSTS)
                self.assertEqual(len(pairs), len(HOSTS))
                self.assertIn("fail-fast: false", job)
                self.assertRegex(job, r"timeout-minutes: \d+")
                self.assertNotIn("continue-on-error:", job)
                self.assertIn("needs: [native-proof-guests, native-proof-build]", job)
                # One target's failed build must not skip the other hosts.
                self.assertIn("if: ${{ !cancelled() }}", job)
                self.assertIn(f"ci/native_proof.py run {suite}", job)
                for line in job.split("steps:", 1)[1].splitlines():
                    code = line.split("#", 1)[0]
                    self.assertIsNone(COMPILES.search(code), f"{name} compiles on its native host: {line.strip()}")

    def test_compiler_and_screenshot_proofs_keep_separate_budgets(self):
        compiler = self.job("wasm-compiler-native")
        screenshot = self.job("wasm-tauri-screenshot-native")
        self.assertIn("Component", compiler)
        self.assertNotIn("run compiler", screenshot)
        self.assertNotIn("run screenshot", compiler)

    def test_proof_runner_checks_run_once_on_linux(self):
        guests = self.job("native-proof-guests")
        self.assertIn("ci.test_native_proof ci.test_native_proof_jobs", guests)
        self.assertIn("tests/screenshot-target-repair.ps1", guests)

    def test_guest_building_screenshot_cli_runs_only_on_linux(self):
        # The one screenshot proof that compiles its own guest is skipped on
        # the native hosts, so it must still run somewhere: here, on Linux.
        job = self.job("screenshot-cli-guest-build")
        self.assertIn("runs-on: ubuntu-latest", job)
        self.assertLess(
            job.index("rustup target add wasm32-wasip1-threads"),
            job.index(f"{native_proof.GUEST_BUILDING_SCREENSHOT_TEST} --exact --ignored"),
            "the CLI's guest build needs its Wasm target installed first",
        )
        self.assertIn('grep -F "test result: ok. 1 passed; 0 failed;"', job)
        source = (WORKFLOW.parents[2] / "ci/native_proof.py").read_text(encoding="utf-8")
        self.assertIn('"--skip", GUEST_BUILDING_SCREENSHOT_TEST', source)

    def test_threaded_script_lane_compiles_only_on_linux(self):
        job = self.job("threaded-rust-artifact")
        self.assertIn("runs-on: ubuntu-latest", job)
        self.assertNotRegex(job, r"macos|windows")

    def test_ci_never_disables_the_soldr_cache(self):
        text = WORKFLOW.read_text(encoding="utf-8")
        self.assertNotIn("--no-cache", text)
        # The umbrella switch; individual layers such as dylints' build-cache
        # may still be tuned.
        self.assertNotRegex(text, r"(?m)^\s+cache: false\b")
        # One documented exception: a Windows-hosted `cross-targets` prepare
        # yields no archive, and setup-soldr's cache then fails the job. Those
        # supported-targets lanes move to Linux with the OpenSSL syslib
        # (zackees/soldr#3246); nothing else may disable the cache.
        self.assertEqual(
            re.findall(r"(?m)^\s+cache: (\$\{\{.*\}\})", text),
            ["${{ !startsWith(matrix.os, 'windows') }}"],
        )


if __name__ == "__main__":
    unittest.main()
