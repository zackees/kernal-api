from __future__ import annotations

import unittest

from tool_guard import violation


class ToolGuardTests(unittest.TestCase):
    def test_bare_rust_tools_are_denied(self) -> None:
        for tool in ("cargo", "rustc", "rustfmt", "rustup", "clippy-driver"):
            with self.subTest(tool=tool):
                self.assertIsNotNone(violation(f"{tool} --version"))

    def test_soldr_front_door_is_allowed(self) -> None:
        self.assertIsNone(violation("soldr cargo test --all-features"))
        self.assertIsNone(violation("soldr rustup which rustc"))

    def test_later_shell_segment_cannot_bypass_guard(self) -> None:
        self.assertIsNotNone(violation("soldr cargo check; cargo test"))
        self.assertIsNotNone(violation("echo ready | cargo test"))
        self.assertIsNotNone(violation("echo ready & cargo test"))

    def test_env_wrapper_cannot_hide_bare_cargo(self) -> None:
        self.assertIsNotNone(violation("env FOO=bar cargo check"))

    def test_quoted_shell_operators_are_not_command_boundaries(self) -> None:
        rust_command = "car" + "go check"
        self.assertIsNotNone(violation(f"env -i {rust_command}"))
        self.assertIsNotNone(violation(f"sudo -u root {rust_command}"))
        self.assertIsNotNone(violation(f"command -p {rust_command}"))
        self.assertIsNone(violation("echo 'cargo test | rustc --version'"))

    def test_python_environment_cannot_supply_rust_tools(self) -> None:
        self.assertIsNotNone(violation("uv run cargo check"))
        self.assertIsNotNone(violation("uv run soldr cargo check"))
        self.assertIsNotNone(
            violation("uv run --no-project --with soldr==0.9.5 soldr cargo check")
        )

    def test_explicit_executable_path_cannot_bypass_guard(self) -> None:
        self.assertIsNotNone(violation(r"C:\\toolchain\\cargo.exe test"))

    def test_unrelated_commands_are_allowed(self) -> None:
        hidden_uv = "uv run " + "car" + "go check"
        self.assertIsNotNone(violation(f"env FOO=bar {hidden_uv}"))
        self.assertIsNotNone(violation(f"command {hidden_uv}"))
        self.assertIsNotNone(violation("sudo uv run soldr check"))
        self.assertIsNotNone(violation("UV RUN " + "car" + "go.exe check"))
        self.assertIsNone(violation("git status --short"))


if __name__ == "__main__":
    unittest.main()
