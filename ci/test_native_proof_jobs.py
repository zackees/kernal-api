"""Keep CI to a few jobs that compile on Linux and run on all six hosts.

The workflow is six jobs: `linux` (the gate), `linux-checks`, `build` and `test`
(one runner per other target each), `dylints`, and `cancel-on-failure`. These
guards pin the properties that shape was chosen for, one test per property, so
a later edit that quietly undoes one fails here instead of on a runner bill.
"""

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
# x86_64 Linux is the gate: its builder is its host, so it builds and runs in
# `linux` / `linux-checks` rather than in the per-target matrices.
GATE_TARGET = "x86_64-unknown-linux-gnu"
EXPECTED_JOBS = {"linux", "linux-checks", "build", "test", "dylints", "cancel-on-failure"}
# `cargo-nextest` is the replay driver, not a compiler: it runs binaries out of
# a prebuilt archive. `.cargo/bin` is a path. Everything else named here would
# compile on the host running it.
COMPILES = re.compile(
    r"\bsoldr\b|(?<!\.)\bcargo\b(?!-nextest)|\brustup\b|\brustc\b"
    r"|build-guest|build-threaded-smoke"
)


class NativeProofJobsTests(unittest.TestCase):
    def text(self):
        return WORKFLOW.read_text(encoding="utf-8")

    def job(self, name):
        match = re.search(
            rf"^  {re.escape(name)}:\n(.*?)(?=^  [\w-]+:|\Z)",
            self.text(),
            re.MULTILINE | re.DOTALL,
        )
        self.assertIsNotNone(match, f"missing job {name}")
        return match.group(1)

    def step(self, job, name):
        """The body of one named step, up to the next step."""
        body = self.job(job).split(f"- name: {name}\n", 1)
        self.assertEqual(len(body), 2, f"{job} has no step {name!r}")
        return re.split(r"\n      - ", body[1], 1)[0]

    # -- the shape -----------------------------------------------------------

    def test_the_pipeline_is_six_jobs(self):
        """Checks are steps on a shared runner, not a runner each.

        This workflow was 20 job definitions and 52 runners per push. A new
        check belongs as a step of the job whose runner it needs; a new job
        needs a reason a step cannot satisfy.
        """
        jobs = set(re.findall(r"(?m)^  ([\w-]+):\n", self.text().split("\njobs:\n", 1)[1]))
        self.assertEqual(jobs, EXPECTED_JOBS)

    def test_only_python_310_is_tested(self):
        """The package is tested on its supported floor alone."""
        self.assertNotIn("3.13", self.text())
        self.assertIn("--python 3.10", self.job("linux-checks"))

    def test_every_other_platform_waits_for_linux(self):
        """A red Linux run must not cost an Apple or Windows runner."""
        self.assertNotIn("needs:", self.job("linux").split("steps:", 1)[0])
        for gated in ("build", "dylints"):
            with self.subTest(job=gated):
                self.assertIn("needs: linux", self.job(gated))
        self.assertIn("needs: [linux, build]", self.job("test"))

    def test_a_failure_cancels_the_rest_of_the_run(self):
        """`fail-fast` stops a matrix's siblings; the sentinel stops everything else."""
        sentinel = self.job("cancel-on-failure")
        self.assertIn("needs: [linux, linux-checks, build, test, dylints]", sentinel)
        self.assertIn("if: failure() && github.event_name == 'pull_request'", sentinel)
        self.assertIn("gh run cancel ${{ github.run_id }}", sentinel)
        self.assertIn("actions: write", sentinel)
        for matrix in ("build", "test"):
            with self.subTest(job=matrix):
                self.assertIn("fail-fast: true", self.job(matrix))

    # -- where things compile ------------------------------------------------

    def test_every_target_is_built_on_linux_exactly_once(self):
        """One compile per target, all on Linux builders (#270)."""
        build = self.job("build")
        builders = re.findall(r"- builder: (\S+)\s+target: (\S+)", build)
        self.assertEqual({target for _, target in builders} | {GATE_TARGET}, TARGETS)
        self.assertEqual(len(builders) + 1, len(TARGETS), "no target is built twice")
        for builder, target in builders:
            self.assertTrue(builder.startswith("ubuntu-"), f"{target} compiles on {builder}")
        for job in ("linux", "build"):
            with self.subTest(job=job):
                self.assertIn("ci-tests: true", self.job(job))
        self.assertIn("cargo nextest archive", build)
        self.assertIn("native_proof.py build --target ${{ matrix.target }}", build)

    def test_native_hosts_only_execute_prebuilt_binaries(self):
        """Every host in `test` runs what `build` produced and compiles nothing."""
        test = self.job("test")
        pairs = set(re.findall(r"- os: (\S+)\s+target: (\S+)", test))
        self.assertEqual(pairs | {("ubuntu-24.04", GATE_TARGET)}, HOSTS)
        for line in test.split("steps:", 1)[1].splitlines():
            code = line.split("#", 1)[0]
            self.assertIsNone(COMPILES.search(code), f"test compiles on its host: {line.strip()}")
        for suite in ("compiler", "screenshot"):
            with self.subTest(suite=suite):
                self.assertIn(f"ci/native_proof.py run {suite}", test)
        # Silence is not success: each host proves it ran its own platform tree.
        self.assertIn("did not execute its own code", test)

    def test_compiler_and_screenshot_proofs_keep_separate_budgets(self):
        compiler = self.step("test", "Run actual Core and Component compiler-policy guest proofs")
        screenshot = self.step(
            "test", "Run offline native Wasm screenshot, admission, and worker containment proofs"
        )
        self.assertIn("timeout-minutes: 20", compiler)
        self.assertIn("timeout-minutes: 30", screenshot)

    def test_the_gate_target_runs_its_proofs_in_place(self):
        """x86_64 Linux builds and runs its native proofs where it is built."""
        checks = self.job("linux-checks")
        self.assertIn("native_proof.py build --target x86_64-unknown-linux-gnu", checks)
        self.assertIn("native_proof.py run compiler", checks)
        self.assertIn("native_proof.py run screenshot", checks)

    def test_each_proof_suite_gets_its_own_work_directory(self):
        """The runner refuses a work directory holding another run's state.

        The compiler and screenshot suites were separate jobs, so each had a
        fresh directory. As consecutive steps they must not share one: the
        first consolidated run failed with `work directory is not empty;
        refusing stale proof state` after the compiler proofs had passed.
        """
        for job in ("test", "linux-checks"):
            with self.subTest(job=job):
                body = self.job(job)
                self.assertIn('native-proof-run-compiler"', body)
                self.assertIn('native-proof-run-screenshot"', body)
                self.assertNotIn('native-proof-run"', body)

    def test_the_windows_webview_proof_runs_a_prebuilt_binary(self):
        """WebView2 needs a Windows host; it does not need a Windows compile."""
        self.assertIn("kernal-tauri-smoke.exe close", self.job("test"))
        self.assertIn("--bin kernal-tauri-smoke", self.job("build"))

    def test_native_archive_lanes_build_for_the_host(self):
        """A lane whose runner matches its target must not name the triple.

        Passing `--target` for aarch64 Linux on an ARM runner makes Soldr treat
        it as a cross build and route linking through a zig shim the runner does
        not have. Clippy never links, so only the archive step caught it.
        """
        lint = self.step("build", "Lint this target")
        archive = self.step("build", "Archive every test binary for this target")
        self.assertNotIn("--target ${{ matrix.target }}", lint + archive)
        self.assertIn("matrix.cross && format('--target {0}', matrix.cross)", lint)
        self.assertIn('target_args=(--target "${{ matrix.cross }}")', archive)

    def test_windows_archives_ship_the_symbolizer_pdb(self):
        """`kernal-symbolize`'s tests read their own binary's PDB."""
        archive = self.step("build", "Archive every test binary for this target")
        self.assertIn("kernal_symbolize-*.pdb", archive)
        self.assertIn('on-missing = \\"error\\"', archive)
        self.assertIn("the kernal-symbolize test build produced no PDB", archive)

    def test_each_replay_host_gets_a_nextest_it_can_execute(self):
        """The bare `linux` and `windows` nextest builds are x86-64."""
        install = self.step("test", "Install cargo-nextest")
        self.assertIn('"${{ runner.os }}-${{ runner.arch }}"', install)
        for suffix in ("linux", "linux-arm", "windows", "windows-arm", "mac"):
            with self.subTest(build=suffix):
                self.assertIn(f"https://get.nexte.st/latest/{suffix} ;;", install)

    # -- Linux-only checks ---------------------------------------------------

    def test_proof_runner_checks_run_once_on_linux(self):
        checks = self.job("linux-checks")
        self.assertIn("ci.test_native_proof ci.test_native_proof_jobs", checks)
        self.assertIn("ci.test_nextest_config ci.test_deny_warnings", checks)
        self.assertIn("tests/screenshot-target-repair.ps1", checks)

    def test_guest_building_screenshot_cli_runs_only_on_linux(self):
        # The one screenshot proof that compiles its own guest is skipped on
        # the native hosts, so it must still run somewhere: here, on Linux.
        checks = self.job("linux-checks")
        self.assertLess(
            checks.index("native_proof.py wasm-target"),
            checks.index(f"{native_proof.GUEST_BUILDING_SCREENSHOT_TEST} --exact --ignored"),
            "the CLI's guest build needs its Wasm target installed first",
        )
        # Materialize the target through the helper that verifies the libraries
        # are on disk; a bare `rustup target add` can install nothing when a
        # restored toolchain cache lists the target as present (#285).
        self.assertIn("native_proof.py wasm-target wasm32-wasip1-threads", checks)
        self.assertNotRegex(checks, r"run:\s*soldr rustup target add")
        self.assertIn('grep -F "test result: ok. 1 passed; 0 failed;"', checks)
        source = (WORKFLOW.parents[2] / "ci/native_proof.py").read_text(encoding="utf-8")
        self.assertIn('"--skip", GUEST_BUILDING_SCREENSHOT_TEST', source)

    def test_threaded_script_lane_compiles_only_on_linux(self):
        self.assertIn("bash scripts/build-threaded-smoke.sh", self.job("linux-checks"))
        self.assertNotIn("build-threaded-smoke", self.job("test"))

    def test_feature_isolation_checks_share_one_runner(self):
        """36 isolated `cargo check`s are one step, not 36 runners."""
        step = self.step("linux-checks", "Check every feature in isolation")
        self.assertIn('for feature in "${features[@]}"', step)
        self.assertIn('failed+=("${feature}")', step)

    def test_ci_never_disables_the_soldr_cache(self):
        text = self.text()
        self.assertNotIn("--no-cache", text)
        # The umbrella switch; individual layers such as dylints' build-cache
        # may still be tuned.
        self.assertNotRegex(text, r"(?m)^\s+cache: false\b")
        # Every prepare runs on Linux (#273), so no job may carry a conditional
        # cache switch: one would mean a lane moved back to a Windows prepare.
        self.assertEqual(re.findall(r"(?m)^\s+cache: (\$\{\{.*\}\})", text), [])


if __name__ == "__main__":
    unittest.main()
