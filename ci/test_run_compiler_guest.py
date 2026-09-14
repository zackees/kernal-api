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


def wasm_artifact(name: str) -> str:
    return json.dumps(
        {
            "reason": "compiler-artifact",
            "target": {"name": name},
            "profile": {"test": False},
            "filenames": [str(Path.cwd() / f"{name}.wasm")],
        }
    )


def component_answers() -> list[str]:
    return [
        wasm_artifact("kernal_component_probe"),
        artifact("kernal-component-tools"),
        proof.COMPONENT_ENCODER_OUTPUT.format(bytes=17),
        artifact("kernal_api", test=True),
        "\n".join(f"{test}: test" for test in proof.COMPONENT_TESTS) + "\n",
        "test result: ok. 1 passed; 0 failed; 0 ignored;\n",
        "test result: ok. 1 passed; 0 failed; 0 ignored;\n",
    ]


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
        ] + component_answers()
        with (
            patch("sys.argv", ["proof", "--native-target", "native-test", "--target-dir", str(Path.cwd())]),
            patch.object(proof, "run", side_effect=answers) as run,
            patch.object(proof.shutil, "copyfile") as copyfile,
            patch.object(proof.Path, "exists", return_value=False),
            patch.object(proof.Path, "stat") as stat,
        ):
            stat.return_value.st_size = 17
            proof.main()
        copyfile.assert_called_once()
        command, core_call = next(
            (call.args[0], call) for call in run.call_args_list if proof.TEST in call.args[0]
        )
        self.assertEqual(command[1], proof.TEST)
        self.assertIn("--exact", command)
        self.assertIn("--ignored", command)
        self.assertTrue(core_call.kwargs["env"]["KERNAL_COMPILER_GUEST_WASM"].endswith(".admitted.wasm"))
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

    def test_runs_component_normal_and_cache_hit_proofs_from_a_fresh_component(self):
        answers = [
            "host: native-test\n",
            artifact("kernal-compiler-guest-proof"),
            artifact("kernal-api-wasm-abi-generator"),
            "",
            artifact("kernal_api", test=True),
            f"{proof.TEST}: test\n",
            "test result: ok. 1 passed; 0 failed; 0 ignored;\n",
        ] + component_answers()
        with (
            patch("sys.argv", ["proof", "--native-target", "native-test", "--target-dir", str(Path.cwd())]),
            patch.object(proof, "run", side_effect=answers) as run,
            patch.object(proof.shutil, "copyfile"),
            patch.object(proof.Path, "exists", return_value=False),
            patch.object(proof.Path, "stat") as stat,
        ):
            stat.return_value.st_size = 17
            proof.main()
        commands = [call.args[0] for call in run.call_args_list]
        self.assertTrue(
            any("benchmarks/wasm-sketch/component-guest/Cargo.toml" in command for command in commands),
            "the six-host runner must build the real Component compiler guest",
        )
        self.assertTrue(any("engine-probe" in command for command in commands))
        component_runs = [
            command for command in commands if command and command[0].endswith("kernal_api")
            and any(test in command for test in proof.COMPONENT_TESTS)
        ]
        self.assertEqual(len(component_runs), 2)


if __name__ == "__main__":
    unittest.main()
