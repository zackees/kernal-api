"""PreToolUse hook that keeps all Rust tooling behind soldr."""

from __future__ import annotations

import json
import sys

RUST_TOOLS = frozenset(
    {
        "cargo",
        "cargo-clippy",
        "cargo-fmt",
        "clippy-driver",
        "rustc",
        "rustfmt",
        "rustup",
    }
)
SHELL_TOOLS = frozenset({"Bash", "Shell", "PowerShell", "exec_command"})
UV_OPTIONS_WITH_VALUES = frozenset(
    {
        "--directory",
        "--env-file",
        "--group",
        "--project",
        "--python",
        "--with",
        "--with-editable",
        "--with-requirements",
        "-m",
        "-p",
    }
)


def _command_words(segment: str) -> list[str]:
    words = segment.strip().split()
    while words:
        word = words[0].lower()
        if word == "command":
            words.pop(0)
            while words and words[0].startswith("-"):
                words.pop(0)
        elif word == "env":
            words.pop(0)
            _discard_options(words, {"-u", "--unset", "-c", "--chdir", "-s", "--split-string"})
        elif word == "sudo":
            words.pop(0)
            _discard_options(
                words,
                {
                    "-c", "--close-from", "-d", "--chdir", "-g", "--group",
                    "-h", "--host", "-p", "--prompt", "-r", "--role",
                    "-t", "--type", "-u", "--user",
                },
            )
        elif "=" in words[0] and not words[0].startswith(("=", "-")):
            words.pop(0)
        else:
            break
    return words


def _discard_options(words: list[str], options_with_values: set[str]) -> None:
    while words and words[0].startswith("-"):
        option = words.pop(0).lower()
        if option in options_with_values and words:
            words.pop(0)


def _normalize_executable(word: str) -> str:
    executable = word.strip("'\"").replace("\\", "/").rsplit("/", 1)[-1].lower()
    return executable.removesuffix(".exe")


def _uv_run_target(words: list[str]) -> str:
    index = 2
    while index < len(words):
        word = words[index]
        if word == "--":
            index += 1
        elif word in UV_OPTIONS_WITH_VALUES:
            index += 2
        elif any(word.startswith(f"{option}=") for option in UV_OPTIONS_WITH_VALUES):
            index += 1
        elif word.startswith("-"):
            index += 1
        else:
            return _normalize_executable(word)
    return ""


def _segments(command: str) -> list[str]:
    """Split shell command positions without splitting quoted operators."""
    segments: list[str] = []
    current: list[str] = []
    quote = ""
    index = 0
    while index < len(command):
        character = command[index]
        if quote:
            current.append(character)
            if character == quote and (index == 0 or command[index - 1] != "\\"):
                quote = ""
            index += 1
            continue
        if character in {"'", '"'}:
            quote = character
            current.append(character)
            index += 1
            continue
        if character in {";", "\n", "\r", "|", "&"}:
            segments.append("".join(current))
            current = []
            repeated = index + 1 < len(command) and command[index + 1] == character
            index += 2 if repeated else 1
            continue
        current.append(character)
        index += 1
    segments.append("".join(current))
    return segments


def violation(command: str) -> str | None:
    """Return a denial reason when a shell segment bypasses soldr."""
    for segment in _segments(command):
        words = _command_words(segment)
        if not words:
            continue
        first = _normalize_executable(words[0])
        if first == "soldr":
            continue
        if first in RUST_TOOLS:
            return (
                f"Use `soldr {first} ...`; direct `{first}` bypasses the pinned "
                "toolchain and soldr-managed zccache path."
            )
        if (
            len(words) >= 3
            and [_normalize_executable(word) for word in words[:2]] == ["uv", "run"]
            and _uv_run_target(words) in RUST_TOOLS | {"soldr"}
        ):
            return (
                "Use the globally installed `soldr ...` command directly; "
                "do not resolve Rust tooling from the Python environment."
            )
    return None


def _command(payload: dict[str, object]) -> str:
    tool_input = payload.get("tool_input")
    if not isinstance(tool_input, dict):
        return ""
    return next(
        (
            value
            for key in ("command", "cmd", "script")
            if isinstance((value := tool_input.get(key)), str)
        ),
        "",
    )


def main() -> int:
    try:
        payload = json.load(sys.stdin)
    except (json.JSONDecodeError, UnicodeDecodeError):
        return 0
    if payload.get("tool_name") not in SHELL_TOOLS:
        return 0
    reason = violation(_command(payload))
    if reason is None:
        return 0
    print(reason, file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
