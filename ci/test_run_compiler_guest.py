"""Fail-closed selection and invocation tests for the compiler guest runner."""

import json
import unittest
from pathlib import Path
from unittest.mock import patch

from ci import run_compiler_guest as proof


def artifact(name: str, *, test: bool = False) -> str:
    return json.dumps(
        {
            "reason": "compiler-artifact",
            "target": {"name": name},
            "profile": {"test": test},
            "executable": str(Path.cwd() / name),
        }
    )


class CompilerProofRunnerTests(unittest.TestCase):
    def test_runs_the_cache_round_trip_with_a_fresh_admitted_guest(self):
        answers = [
            "host: native-test\n",
            artifact("kernal-compiler-guest-proof"),
            artifact("kernal-api-wasm-abi-generator"),
            "",
            artifact("kernal_api", test=True),
            f"{proof.TEST}: test\n",
            "test result: ok. 1 passed; 0 failed; 0 ignored;\n",
        ]
        with (
            patch("sys.argv", ["proof", "--native-target", "native-test", "--target-dir", str(Path.cwd())]),
            patch.object(proof, "run", side_effect=answers) as run,
            patch.object(proof.shutil, "copyfile") as copyfile,
        ):
            proof.main()
        copyfile.assert_called_once()
        command = run.call_args_list[-1].args[0]
        self.assertEqual(command[1], proof.TEST)
        self.assertIn("--exact", command)
        self.assertIn("--ignored", command)
        self.assertTrue(run.call_args_list[-1].kwargs["env"]["KERNAL_COMPILER_GUEST_WASM"].endswith(".admitted.wasm"))
        guest_build = run.call_args_list[1].args[0]
        self.assertIn("kernal-compiler-guest-proof", guest_build)
        self.assertIn("benchmarks/wasm-sketch/compiler-guest/Cargo.toml", guest_build)

    def test_rejects_zero_executed_cache_proof(self):
        answers = [
            "host: native-test\n",
            artifact("kernal-compiler-guest-proof"),
            artifact("kernal-api-wasm-abi-generator"),
            "",
            artifact("kernal_api", test=True),
            f"{proof.TEST}: test\n",
            "test result: ok. 0 passed; 0 failed; 0 ignored;\n",
        ]
        with (
            patch("sys.argv", ["proof", "--native-target", "native-test", "--target-dir", str(Path.cwd())]),
            patch.object(proof, "run", side_effect=answers),
            patch.object(proof.shutil, "copyfile"),
            self.assertRaisesRegex(ValueError, "exactly once"),
        ):
            proof.main()


if __name__ == "__main__":
    unittest.main()
