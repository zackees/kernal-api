"""The local gate accepts only completed, source-bound act2 evidence."""

import unittest
from dataclasses import replace
from pathlib import Path

from ci.local_gate import RunProof, verify_run


class RunProofTests(unittest.TestCase):
    def setUp(self):
        self.workspace = Path("/repo").resolve()
        self.sha = "a" * 40
        self.proof = RunProof(
            self.workspace,
            self.sha,
            None,
            "act",
            "0.2.89-act2.1",
            ".github/workflows/ci.yml",
            "linux",
            "minimal",
            "done",
            "success",
            0,
            1,
            1,
            0,
        )

    def test_completed_matching_run_passes(self):
        verify_run(self.proof, self.workspace, self.sha)

    def test_incomplete_wrong_tree_and_upstream_act_fail(self):
        for changes in (
            {"workspace": Path("/another")},
            {"sha": "b" * 40},
            {"dirty": True},
            {"state": "running"},
            {"conclusion": "failure"},
            {"act_version": "0.2.89"},
            {"engine": "host"},
            {"workflow": "other.yml"},
            {"job": "test"},
            {"mode": "full"},
            {"exit_code": 1},
            {"total": 0},
            {"completed": 0},
            {"failed": 1},
        ):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                verify_run(replace(self.proof, **changes), self.workspace, self.sha)

    def test_missing_boundary_fields_are_rejected(self):
        with self.assertRaises(ValueError):
            RunProof.from_json({})


if __name__ == "__main__":
    unittest.main()
