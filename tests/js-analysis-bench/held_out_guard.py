"""Refuse a pull request that changes analysis code and held-out bench files together.

Rule 2 of `research/package-graphs/HELD-OUT.md`: a held-out driver, or the
decision to run a package's own suite, and its recording are committed apart
from any analysis change. CI pipes the pull request's changed paths into this
script (`git diff --name-only --no-renames BASE...HEAD`) and passes the base
commit (`--base BASE`); it exits 1 and names the rule when the paths include
analysis code and a held-out file that already exists on the base. A first add
of a held-out file is not refused: a pull request that lands a stack carries
the held-out files and the analysis change together, after they were written
apart.

A held-out file is anything under the held-out driver directory, or a recorded
file under `observed/` whose name starts with a held-out package's name and a
dot. The names come from the `held_out = true` packages in `inputs.toml`.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tomllib
from collections.abc import Callable
from pathlib import Path

BENCH = "tests/js-analysis-bench"
HELD_OUT_DRIVERS = f"{BENCH}/observed/drivers/heldout/"
ANALYSIS = re.compile(r"^crates/polint/src/(ts|analysis[^/]*|graph)(/|\.rs$)")
RULE = "research/package-graphs/HELD-OUT.md rule 2 (separation of authorship)"


def held_out_names(inputs: Path) -> set[str]:
    packages = tomllib.loads(inputs.read_text())["package"]
    return {package["name"] for package in packages if package.get("held_out")}


def is_held_out(path: str, names: set[str]) -> bool:
    if path.startswith(HELD_OUT_DRIVERS):
        return True
    if not path.startswith(f"{BENCH}/observed/"):
        return False
    file_name = path.rsplit("/", 1)[-1]
    return any(file_name.startswith(f"{name}.") for name in names)


def exists_at(commit: str) -> Callable[[str], bool]:
    """Whether a path exists in `commit` of the repository in the working directory."""

    def exists(path: str) -> bool:
        probe = subprocess.run(["git", "cat-file", "-e", f"{commit}:{path}"], capture_output=True)
        return probe.returncode == 0

    return exists


def violations(
    paths: list[str], names: set[str], on_base: Callable[[str], bool] = lambda _: True
) -> tuple[list[str], list[str]]:
    """The analysis paths and the held-out paths on the base, when a change has both; empty lists otherwise."""
    analysis = [path for path in paths if ANALYSIS.match(path)]
    held_out = [path for path in paths if is_held_out(path, names) and on_base(path)]
    if analysis and held_out:
        return analysis, held_out
    return [], []


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", required=True, help="the commit the pull request merges into")
    args = parser.parse_args()
    paths = [line.strip() for line in sys.stdin if line.strip()]
    names = held_out_names(Path(__file__).resolve().parent / "inputs.toml")
    analysis, held_out = violations(paths, names, exists_at(args.base))
    if not analysis:
        print(f"held-out authorship: ok ({len(paths)} changed paths)")
        return 0
    print(
        f"{RULE}: this pull request changes analysis code and held-out bench files the base already has.",
        file=sys.stderr,
    )
    print("Change the held-out driver and its recording in their own pull request.", file=sys.stderr)
    for path in analysis:
        print(f"  analysis: {path}", file=sys.stderr)
    for path in held_out:
        print(f"  held-out: {path}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
