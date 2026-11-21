"""The authorship guard, run the way CI runs it, on a scratch git repository.

A pull request that adds held-out files for the first time together with
analysis code passes; one that changes, renames or deletes a held-out file the
base already has, together with analysis code, is refused.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

GUARD = Path(__file__).resolve().parent / "held_out_guard.py"
ANALYSIS = "crates/polint/src/analysis_neutral/points_to/mod.rs"
DRIVER = "tests/js-analysis-bench/observed/drivers/heldout/co.mjs"
RECORDING = "tests/js-analysis-bench/observed/packages/co.calls.json"


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-c", "user.name=guard", "-c", "user.email=guard@example.invalid", *args],
        cwd=repo, check=True, capture_output=True, text=True,
    ).stdout.strip()


def write(repo: Path, path: str, text: str) -> None:
    (repo / path).parent.mkdir(parents=True, exist_ok=True)
    (repo / path).write_text(text)


def commit(repo: Path, message: str) -> str:
    git(repo, "add", "--all")
    git(repo, "commit", "--quiet", "--allow-empty", "-m", message)
    return git(repo, "rev-parse", "HEAD")


def guard(repo: Path, base: str) -> int:
    """Exit code of the guard on the changes from `base` to HEAD, as CI pipes them."""
    paths = git(repo, "diff", "--name-only", "--no-renames", f"{base}...HEAD")
    run = subprocess.run(
        [sys.executable, str(GUARD), "--base", base], input=paths, cwd=repo, capture_output=True, text=True
    )
    return run.returncode


def case(name: str, base_files: dict[str, str], change, expected: int, failures: list[str]) -> None:
    with tempfile.TemporaryDirectory(prefix="held-out-guard-") as scratch:
        repo = Path(scratch)
        git(repo, "init", "--quiet", "--initial-branch=main")
        for path, text in base_files.items():
            write(repo, path, text)
        base = commit(repo, "base")
        git(repo, "checkout", "--quiet", "-b", "change")
        change(repo)
        commit(repo, "change")
        code = guard(repo, base)
        if code != expected:
            failures.append(f"{name}: exit {code}, expected {expected}")


def main() -> int:
    failures: list[str] = []
    analysis_only = {ANALYSIS: "// analysis\n"}
    with_held_out = {ANALYSIS: "// analysis\n", DRIVER: "// driver\n", RECORDING: "{}\n"}

    def touch_analysis(repo: Path) -> None:
        write(repo, ANALYSIS, "// analysis, changed\n")

    def first_add(repo: Path) -> None:
        touch_analysis(repo)
        write(repo, DRIVER, "// driver\n")
        write(repo, RECORDING, "{}\n")

    def change_driver(repo: Path) -> None:
        touch_analysis(repo)
        write(repo, DRIVER, "// driver, changed\n")

    def change_driver_alone(repo: Path) -> None:
        write(repo, DRIVER, "// driver, changed\n")

    def rename_recording(repo: Path) -> None:
        touch_analysis(repo)
        git(repo, "mv", RECORDING, RECORDING.replace("packages/", "packages/renamed-"))

    def delete_recording(repo: Path) -> None:
        touch_analysis(repo)
        git(repo, "rm", "--quiet", RECORDING)

    case("first add with analysis code", analysis_only, first_add, 0, failures)
    case("later change with analysis code", with_held_out, change_driver, 1, failures)
    case("later change alone", with_held_out, change_driver_alone, 0, failures)
    case("rename of a recorded file with analysis code", with_held_out, rename_recording, 1, failures)
    case("deletion of a recorded file with analysis code", with_held_out, delete_recording, 1, failures)
    case("analysis code alone", analysis_only, touch_analysis, 0, failures)
    for failure in failures:
        print(f"held-out guard: {failure}", file=sys.stderr)
    if failures:
        return 1
    print("held-out guard test: ok (6 cases)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
