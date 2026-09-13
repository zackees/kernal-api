"""Fail-closed artifact selection and execution checks for the six-host proof."""

import json
import unittest
from pathlib import Path
from unittest.mock import patch

from ci import run_extension2_guest as proof


def artifact(name: str, *, test: bool = False, absolute: bool = True) -> str:
    path = Path.cwd() / name if absolute else Path(name)
    return json.dumps(
        {
            "reason": "compiler-artifact",
            "target": {"name": name},
            "profile": {"test": test},
            "executable": str(path),
        }
    )


class ProofRunnerTests(unittest.TestCase):
    def test_selects_only_named_executable_with_matching_profile(self):
        messages = "\n".join(
            [
                artifact("other"),
                artifact("kernal_api"),
                artifact("kernal_api", test=True),
            ]
        )
        self.assertEqual(
            proof.executable(messages, "kernal_api", test=True),
            Path.cwd() / "kernal_api",
        )

    def test_missing_ambiguous_relative_and_malformed_artifacts_fail(self):
        for messages in [
            "",
            artifact("guest") + "\n" + artifact("guest"),
            artifact("guest", absolute=False),
            "not json",
        ]:
            with self.subTest(messages=messages), self.assertRaises(ValueError):
                proof.executable(messages, "guest", test=False)

    def invoke(self, listing: str, result: str):
        answers = [
            "host: native-test\n",
            artifact("kernal-extension2-guest-proof"),
            artifact("kernal-api-wasm-abi-generator"),
            "",
            artifact("kernal_api", test=True),
            listing,
            result,
        ]
        with (
            patch(
                "sys.argv",
                [
                    "proof",
                    "--native-target",
                    "native-test",
                    "--target-dir",
                    str(Path.cwd()),
                ],
            ),
            patch.object(proof, "run", side_effect=answers) as run,
            patch.object(proof.shutil, "copyfile"),
        ):
            proof.main()
            return run.call_args_list

    def test_requires_the_ignored_test_to_exist(self):
        with self.assertRaisesRegex(ValueError, "missing"):
            self.invoke("0 tests", "")

    def test_zero_executed_tests_is_failure(self):
        with self.assertRaisesRegex(ValueError, "exactly once"):
            self.invoke(f"{proof.TEST}: test", "0 passed; 0 failed; 0 ignored;")

    def test_native_execution_uses_exact_test_and_fresh_artifact_env(self):
        calls = self.invoke(
            f"{proof.TEST}: test", "test result: ok. 1 passed; 0 failed; 0 ignored;"
        )
        command = calls[-1].args[0]
        self.assertIn("--exact", command)
        self.assertIn("--ignored", command)
        self.assertIn(proof.TEST, command)
        self.assertTrue(
            calls[-1]
            .kwargs["env"]["KERNAL_EXTENSION2_STREAM_WASM"]
            .endswith(".admitted.wasm")
        )

    def test_cross_compiler_is_rejected_before_building(self):
        with (
            patch(
                "sys.argv",
                [
                    "proof",
                    "--native-target",
                    "foreign",
                    "--target-dir",
                    str(Path.cwd()),
                ],
            ),
            patch.object(proof, "run", return_value="host: actual") as run,
        ):
            with self.assertRaisesRegex(ValueError, "native Rust host"):
                proof.main()
            self.assertEqual(run.call_count, 1)


if __name__ == "__main__":
    unittest.main()
