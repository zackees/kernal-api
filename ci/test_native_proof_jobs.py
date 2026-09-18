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
# `cargo-nextest` is the replay driver, not a compiler: it runs binaries out of
# a prebuilt archive. `.cargo/bin` is a path. Everything else named here would
# compile on the host running it.
COMPILES = re.compile(
    r"\bsoldr\b|(?<!\.)\bcargo\b(?!-nextest)|\brustup\b|\brustc\b"
    r"|build-guest|build-threaded-smoke"
)


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

    def test_every_test_binary_is_built_on_linux(self):
        """One compile for six hosts (#270).

        The archive job is where every test binary for every supported host is
        linked. If one of these lanes moves to a native builder, the scarce
        Apple and Windows runners go back to holding a compiler.
        """
        linux = self.job("test-archive-linux")
        job = self.job("test-archive")
        builders = re.findall(r"- builder: (\S+)\s+target: (\S+)", job)
        covered = {target for _, target in builders} | {"x86_64-unknown-linux-gnu"}
        self.assertEqual(covered, TARGETS, "every supported target is archived")
        self.assertEqual(len(builders) + 1, len(TARGETS), "no target is archived twice")
        for builder, target in builders:
            self.assertTrue(builder.startswith("ubuntu-"), f"{target} compiles on {builder}")
        for stage in (linux, job):
            self.assertIn("ci-tests: true", stage)
            self.assertIn("cargo nextest archive", stage)

    def test_every_other_platform_waits_for_linux(self):
        """A red Linux run must not cost an Apple or Windows runner.

        Linux compiles the same sources every other target does, so its
        failure condemns them. These four jobs are where a run spends most of
        its runner minutes; each waits for the Linux replay to pass.
        """
        text = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("needs: test-archive-linux", text)
        for gated in ("test-archive", "native-proof-build", "dylints", "supported-targets"):
            with self.subTest(job=gated):
                self.assertIn("needs: test-run-linux", self.job(gated))

    def test_a_failure_cancels_the_rest_of_the_run(self):
        """`fail-fast` stops a matrix's siblings; this stops everything else."""
        for sentinel in ("cancel-on-linux-failure", "cancel-on-cross-target-failure"):
            with self.subTest(job=sentinel):
                job = self.job(sentinel)
                self.assertIn("if: failure() && github.event_name == 'pull_request'", job)
                self.assertIn("gh run cancel ${{ github.run_id }}", job)
                self.assertIn("actions: write", job)
        # The two archive/replay matrices stop their own siblings.
        for matrix in ("test-archive", "test-run"):
            with self.subTest(job=matrix):
                self.assertIn("fail-fast: true", self.job(matrix))

    def test_the_replay_lanes_never_compile(self):
        """The point of the archive is that these hosts only execute."""
        job = self.job("test-run")
        pairs = re.findall(r"- os: (\S+)\s+target: (\S+)", job)
        self.assertEqual(
            set(pairs) | {("ubuntu-24.04", "x86_64-unknown-linux-gnu")},
            HOSTS,
            "every supported host replays, Linux through its own gating job",
        )
        for line in job.split("steps:", 1)[1].splitlines():
            code = line.split("#", 1)[0]
            self.assertIsNone(
                COMPILES.search(code), f"test-run compiles on its host: {line.strip()}"
            )
        # Silence is not success: each host proves it ran its own platform tree.
        self.assertIn("did not execute its own code", job)

    def test_the_windows_webview_proof_runs_a_prebuilt_binary(self):
        """WebView2 needs a Windows host; it does not need a Windows compile."""
        job = self.job("windows-webview-smoke")
        self.assertIn("download-artifact", job)
        for line in job.split("steps:", 1)[1].splitlines():
            code = line.split("#", 1)[0]
            self.assertIsNone(
                COMPILES.search(code), f"the webview proof compiles: {line.strip()}"
            )

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


if __name__ == "__main__":
    unittest.main()
