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
            job.index("native_proof.py wasm-target"),
            job.index(f"{native_proof.GUEST_BUILDING_SCREENSHOT_TEST} --exact --ignored"),
            "the CLI's guest build needs its Wasm target installed first",
        )
        # Materialize the target through the helper that verifies the
        # libraries are on disk. A bare `rustup target add` installs nothing
        # when a restored toolchain cache lists the target as present without
        # its libraries, which failed this lane intermittently on unchanged
        # commits until the two were told apart (#285).
        self.assertIn("native_proof.py wasm-target wasm32-wasip1-threads", job)
        self.assertNotRegex(
            job,
            r"run:\s*soldr rustup target add",
            "install the Wasm target through ensure_wasm_target, not a bare rustup add",
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
        # The Windows `supported-targets` lanes were the one exception: a
        # Windows-hosted `cross-targets` prepare yielded no archive and
        # setup-soldr's cache then failed the job. They prepare on Linux now
        # (#273, with the static OpenSSL syslib from zackees/soldr#3246/#3247),
        # so no job may carry a conditional cache switch at all.
        self.assertEqual(
            re.findall(r"(?m)^\s+cache: (\$\{\{.*\}\})", text),
            [],
            "every prepare runs on Linux; a conditional cache switch means a "
            "lane moved back to a Windows-hosted prepare",
        )

    def test_dylints_lints_the_windows_selected_code_from_linux(self):
        """A Dylint pass only sees what it compiles (#147).

        The `cfg(windows)` bodies are invisible to a Linux-hosted pass unless it
        cross-targets, and no Windows runner can host the lint today
        (zackees/soldr#3274). Dropping the `--target` step would silently
        restore the gap that let a `winapi` type reach a public signature.
        """
        job = self.job("dylints")
        self.assertIn("--target x86_64-pc-windows-msvc", job)
        windows_lint = job.split("Lint the Windows-selected code from Linux", 1)[1]
        for flag in ("--all-features", "--all-targets", "--target x86_64-pc-windows-msvc"):
            with self.subTest(flag=flag):
                self.assertIn(flag, windows_lint)
        self.assertIn(
            "soldr rustup target add\n          --toolchain nightly-2026-05-28 "
            "x86_64-pc-windows-msvc",
            job,
            "the pinned Dylint toolchain needs the Windows target installed",
        )


if __name__ == "__main__":
    unittest.main()
