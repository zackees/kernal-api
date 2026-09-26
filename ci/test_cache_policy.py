"""Keep the setup-soldr cache footprint inside the 10 GB repository budget.

One pull request once offered about 23 GB of cache entries, 14.6 GB of them
~2.4 GB cook bases (#355, zackees/setup-soldr#528). These guards pin the
properties that keep the footprint bounded:

- every setup-soldr step runs one pinned revision, so two versions never
  write two copies of the same key;
- steps that cook the same graph for the same target share one
  `cache-key-suffix` (the suffix is part of the cook-base key), unless a
  listed reason says why the graphs differ;
- no step saves durable caches from a `pull_request` run, whose entries only
  that pull request can restore: every pin is v0.9.78+ with `save-cache: auto`
  unless a listed reason needs `true`;
- no step writes the per-commit cook-delta layer: one `cook-delta-v2-*`
  generation per commit accumulated ~2.8 GB on main with nothing pruning it
  (zackees/setup-soldr#528). Every step pins a release that honors
  `cook-delta` and sets it to `false`; cook bases stay on.
"""

import re
import unittest
from pathlib import Path

WORKFLOWS = Path(__file__).resolve().parents[1] / ".github/workflows"
SETUP_SOLDR = re.compile(
    r"^(?P<indent> *)- uses: zackees/setup-soldr@(?P<ref>\S+)(?P<comment>.*)$"
)

# Every setup-soldr step, keyed by (workflow, job): the target it cooks for
# and the profile it compiles. A new step must be classified here, so its
# cook base is weighed against the others before it lands. `matrix` targets
# carry `${{ matrix.target }}` in their suffix, one distinct target per entry.
COOK_SHAPES = {
    ("ci.yml", "linux"): ("x86_64-unknown-linux-gnu", "dev"),
    ("ci.yml", "build"): ("matrix", "dev"),
    ("ci.yml", "dylints"): ("dylint-nightly", "none"),
    ("release.yml", "validate-and-package"): ("x86_64-unknown-linux-gnu", "dev"),
    ("release.yml", "symbolizer-workers"): ("matrix", "none"),
    ("auto-release.yml", "publish-crates"): ("x86_64-unknown-linux-gnu", "none"),
    ("macos-x64-tests.yml", "intel-macos-tests"): ("x86_64-apple-darwin", "none"),
}

# (target, profile) pairs whose steps may keep different suffixes, with the
# reason their cook graphs genuinely differ. Empty today: every cooking pair
# on one target shares one suffix.
JUSTIFIED_SPLITS: dict[tuple[str, str], str] = {}

# setup-soldr v0.9.78 (zackees/setup-soldr#527) added `save-cache`, whose
# default `auto` skips every durable save on `pull_request`. Older pins save
# unconditionally.
SAVE_POLICY_MIN_VERSION = (0, 9, 78)

# The first setup-soldr release whose main action honors `cook-delta`
# (zackees/setup-soldr#528). Older pins always write the delta layer.
COOK_DELTA_MIN_VERSION = (0, 9, 80)

# Steps allowed to keep the cook-delta layer, with the reason (a measured
# warm-build win and a retention bound). Empty today.
COOK_DELTA_JUSTIFICATIONS: dict[tuple[str, str], str] = {}

# Steps allowed `save-cache: "true"` on pull requests, with the reason: only
# when a later job in the same run restores what this step saved. Empty
# today: no job in a pull-request run restores another job's entry (`linux`,
# each `build` target and `dylints` key their own caches; `test` compiles
# nothing).
PR_SAVE_JUSTIFICATIONS: dict[tuple[str, str], str] = {}

# What `prebuild-deps-flags` must be for a job compiling each profile. The
# flags are hashed into the cook key; cooking `--release` for a job that only
# builds the dev profile stores a base nothing reuses.
PROFILE_FLAGS = {"dev": "", "release": "--release"}
DEFAULT_COOK_FLAGS = "--release"


def workflow_triggers(text):
    block = re.search(r"(?ms)^on:\n(.*?)(?=^\S)", text)
    return set(re.findall(r"(?m)^  ([\w-]+):", block.group(1))) if block else set()


def setup_soldr_steps():
    """Yield one dict per setup-soldr step across every workflow."""
    for path in sorted(WORKFLOWS.glob("*.yml")):
        text = path.read_text(encoding="utf-8")
        triggers = workflow_triggers(text)
        lines = text.splitlines()
        job = None
        for index, line in enumerate(lines):
            header = re.match(r"^  ([\w-]+):\s*$", line)
            if header and text.find("\njobs:\n") < sum(len(x) + 1 for x in lines[:index]):
                job = header.group(1)
            match = SETUP_SOLDR.match(line)
            if not match:
                continue
            step_indent = len(match.group("indent"))
            inputs = {}
            for body in lines[index + 1 :]:
                if body.strip() and len(body) - len(body.lstrip()) <= step_indent:
                    break
                pair = re.match(r"^\s+([\w-]+):\s*(.*?)\s*$", body)
                if pair and not body.lstrip().startswith("#") and pair.group(1) != "with":
                    value = re.sub(r"\s+#.*$", "", pair.group(2))
                    inputs[pair.group(1)] = value.strip('"').strip("'")
            yield {
                "workflow": path.name,
                "job": job,
                "ref": match.group("ref"),
                "comment": match.group("comment"),
                "inputs": inputs,
                "triggers": triggers,
            }


def cooks(step):
    inputs = step["inputs"]
    return inputs.get("cache", "true") != "false" and inputs.get("prebuild-deps", "") != "none"


def pin_version(step):
    match = re.search(r"#\s*v(\d+)\.(\d+)\.(\d+)", step["comment"])
    return tuple(int(part) for part in match.groups()) if match else None


def saves_on_pull_request(step):
    """Whether the step would upload durable caches from a pull request."""
    inputs = step["inputs"]
    if all(
        inputs.get(name, "true") == "false"
        for name in ("cache", "build-cache", "solo-toolchain-cache")
    ):
        return False
    version = pin_version(step)
    if version is None or version < SAVE_POLICY_MIN_VERSION:
        return True
    return inputs.get("save-cache", "auto") not in {"auto", "false"}


class CachePolicyTests(unittest.TestCase):
    def steps(self):
        steps = list(setup_soldr_steps())
        self.assertTrue(steps, "found no setup-soldr steps")
        return steps

    def test_every_step_is_classified(self):
        """A new setup-soldr step names its target and profile here first."""
        found = {(s["workflow"], s["job"]) for s in self.steps()}
        self.assertEqual(found, set(COOK_SHAPES))

    def test_one_pinned_revision(self):
        """Mixed setup-soldr versions write duplicate keys."""
        refs = {s["ref"] for s in self.steps()}
        self.assertEqual(len(refs), 1, f"setup-soldr pins differ: {sorted(refs)}")
        (ref,) = refs
        self.assertRegex(ref, r"^[0-9a-f]{40}$", "pin setup-soldr to a full commit SHA")

    def test_cook_flags_match_the_compiled_profile(self):
        for step in self.steps():
            target, profile = COOK_SHAPES[(step["workflow"], step["job"])]
            with self.subTest(workflow=step["workflow"], job=step["job"]):
                if profile == "none":
                    self.assertFalse(cooks(step), "classified as not cooking, but cooks")
                    continue
                self.assertTrue(cooks(step), "classified as cooking, but does not")
                flags = step["inputs"].get("prebuild-deps-flags", DEFAULT_COOK_FLAGS)
                if target == "matrix":
                    self.assertIn("matrix.", step["inputs"].get("cache-key-suffix", ""))
                    continue
                self.assertEqual(flags, PROFILE_FLAGS[profile])

    def test_one_suffix_per_target_and_graph(self):
        """Each cook base is ~2.4 GB; two suffixes for one graph store it twice."""
        suffixes = {}
        for step in self.steps():
            target, profile = COOK_SHAPES[(step["workflow"], step["job"])]
            if not cooks(step) or target == "matrix":
                continue
            key = (target, profile, step["inputs"].get("toolchain", ""))
            suffixes.setdefault(key, {}).setdefault(
                step["inputs"].get("cache-key-suffix", ""), []
            ).append(f"{step['workflow']}:{step['job']}")
        for (target, profile, _), by_suffix in suffixes.items():
            if (target, profile) in JUSTIFIED_SPLITS:
                continue
            with self.subTest(target=target, profile=profile):
                self.assertEqual(len(by_suffix), 1, f"one graph, several suffixes: {by_suffix}")

    def test_justified_splits_are_live(self):
        for pair, reason in JUSTIFIED_SPLITS.items():
            self.assertTrue(reason.strip(), pair)
            self.assertIn(pair, set(COOK_SHAPES.values()))

    def test_no_step_saves_on_pull_request(self):
        """A pull-request cache is restorable only by that pull request."""
        for step in self.steps():
            if "pull_request" not in step["triggers"] or not saves_on_pull_request(step):
                continue
            where = (step["workflow"], step["job"])
            with self.subTest(workflow=where[0], job=where[1]):
                self.assertIn(
                    where,
                    PR_SAVE_JUSTIFICATIONS,
                    "pin setup-soldr v0.9.78+ and leave `save-cache` unset or `auto`",
                )

    def test_every_step_states_its_save_policy(self):
        """Every step names `save-cache`, so the policy is visible in review."""
        for step in self.steps():
            with self.subTest(workflow=step["workflow"], job=step["job"]):
                self.assertIn(step["inputs"].get("save-cache"), {"auto", "true", "false"})

    def test_pr_save_justifications_are_live(self):
        """Drop a justification once its step stops saving on pull requests."""
        for where, reason in PR_SAVE_JUSTIFICATIONS.items():
            steps = [s for s in self.steps() if (s["workflow"], s["job"]) == where]
            with self.subTest(workflow=where[0], job=where[1]):
                self.assertTrue(reason.strip())
                self.assertTrue(steps and all(saves_on_pull_request(s) for s in steps))

    def test_no_step_writes_the_cook_delta_layer(self):
        """The delta layer saves one generation per commit and is never pruned."""
        for step in self.steps():
            where = (step["workflow"], step["job"])
            if where in COOK_DELTA_JUSTIFICATIONS:
                continue
            with self.subTest(workflow=where[0], job=where[1]):
                self.assertEqual(
                    step["inputs"].get("cook-delta"),
                    "false",
                    "set `cook-delta: false` (setup-soldr#528)",
                )
                version = pin_version(step)
                self.assertIsNotNone(COOK_DELTA_MIN_VERSION)
                self.assertTrue(
                    version is not None and version >= COOK_DELTA_MIN_VERSION,
                    f"setup-soldr {version} ignores `cook-delta`",
                )

    def test_cook_delta_justifications_are_live(self):
        for where, reason in COOK_DELTA_JUSTIFICATIONS.items():
            self.assertTrue(reason.strip(), where)
            self.assertIn(where, COOK_SHAPES)


if __name__ == "__main__":
    unittest.main()
