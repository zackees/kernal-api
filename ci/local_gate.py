"""Run the remote minimal Linux job through bosn's pinned act2 engine.

The first migration lane covers minimal Linux only. Full and native platform
jobs remain remote until their complete coverage has local evidence.
"""

from __future__ import annotations

import json
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path

JsonValue = str | int | float | bool | None | list["JsonValue"] | dict[str, "JsonValue"]
ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ".github/workflows/ci.yml"


@dataclass(frozen=True)
class RunProof:
    workspace: Path
    sha: str
    dirty: JsonValue
    engine: str
    act_version: str
    workflow: str
    job: str
    mode: str
    state: str
    conclusion: str
    exit_code: int
    total: int
    completed: int
    failed: int

    @classmethod
    def from_json(cls, raw: dict[str, JsonValue]) -> RunProof:
        """Validate the wire document before treating it as evidence."""
        strings = (
            "workspace",
            "sha",
            "engine",
            "act_version",
            "workflow",
            "job",
            "mode",
            "state",
            "conclusion",
        )
        if any(not isinstance(raw.get(key), str) for key in strings):
            raise ValueError("bosn record is missing identity or completion fields")
        jobs = raw.get("jobs")
        if not isinstance(jobs, dict) or "dirty" not in raw:
            raise ValueError("bosn record is missing snapshot or job evidence")
        if type(raw.get("exit_code")) is not int or any(
            type(jobs.get(key)) is not int for key in ("total", "completed", "failed")
        ):
            raise ValueError("bosn record is missing numeric completion evidence")
        return cls(
            Path(str(raw["workspace"])).resolve(),
            str(raw["sha"]),
            raw["dirty"],
            str(raw["engine"]),
            str(raw["act_version"]),
            str(raw["workflow"]),
            str(raw["job"]),
            str(raw["mode"]),
            str(raw["state"]),
            str(raw["conclusion"]),
            int(raw["exit_code"]),
            int(jobs["total"]),
            int(jobs["completed"]),
            int(jobs["failed"]),
        )


def verify_run(proof: RunProof, workspace: Path, sha: str) -> None:
    """Reject unrelated, dirty, partial, failed, or upstream-act runs."""
    if (
        proof.workspace != workspace.resolve()
        or proof.sha != sha
        or proof.dirty is not None
        or proof.engine != "act"
        or "-act2." not in proof.act_version
        or proof.workflow != WORKFLOW
        or proof.job != "linux"
        or proof.mode != "minimal"
    ):
        raise ValueError("bosn run does not prove this clean minimal Linux lane")
    if (
        proof.state != "done"
        or proof.conclusion != "success"
        or proof.exit_code != 0
        or proof.total < 1
        or proof.completed != proof.total
        or proof.failed != 0
    ):
        raise ValueError("bosn run did not complete every selected job successfully")


def output(argv: list[str]) -> str:
    """Temporary-file capture avoids pipe-held EOF and capture-then-wait."""
    with tempfile.TemporaryFile(mode="w+", encoding="utf-8") as stream:
        subprocess.run(argv, cwd=ROOT, stdout=stream, check=True)
        stream.seek(0)
        return stream.read()


def document(argv: list[str]) -> dict[str, JsonValue]:
    raw = json.loads(output(argv))
    if not isinstance(raw, dict):
        raise TypeError("expected a bosn JSON object")
    return raw


def main() -> None:
    if output(["git", "status", "--porcelain", "--untracked-files=normal"]).strip():
        raise ValueError("commit the worktree before running its local gate")
    sha = output(["git", "rev-parse", "HEAD"]).strip()
    submitted = document(
        [
            "bosn",
            "ci",
            "run",
            "--workspace",
            str(ROOT),
            "--workflow",
            WORKFLOW,
            "--job",
            "linux",
            "--trigger",
            "pr",
            "--mode",
            "minimal",
            "--sha",
            sha,
            "--timeout-secs",
            "7200",
            "--json",
        ]
    )
    run_id = submitted.get("run")
    if not isinstance(run_id, str) or not run_id:
        raise ValueError("bosn did not return a run ID")
    print(f"bosn local gate run: {run_id}", flush=True)
    subprocess.run(["bosn", "ci", "wait", run_id], cwd=ROOT, check=True)
    proof = RunProof.from_json(document(["bosn", "ci", "show", run_id, "--json"]))
    verify_run(proof, ROOT, sha)
    if (
        output(["git", "rev-parse", "HEAD"]).strip() != sha
        or output(["git", "status", "--porcelain", "--untracked-files=normal"]).strip()
    ):
        raise ValueError("worktree changed while the local gate ran")
    print(f"Passed minimal Linux on {proof.act_version}: {sha}", flush=True)


if __name__ == "__main__":
    main()
