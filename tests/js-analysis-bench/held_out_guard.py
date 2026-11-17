"""Refuse a pull request that changes analysis code and held-out bench files together.

Rule 2 of `research/package-graphs/HELD-OUT.md`: a held-out driver, or the
decision to run a package's own suite, and its recording are committed apart
from any analysis change. CI pipes the pull request's changed paths into this
script (`git diff --name-only BASE...HEAD`); it exits 1 and names the rule when
the paths include both kinds.

A held-out file is anything under the held-out driver directory, or a recorded
file under `observed/` whose name starts with a held-out package's name and a
dot. The names come from the `held_out = true` packages in `inputs.toml`.
"""

from __future__ import annotations

import re
import sys
import tomllib
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


def violations(paths: list[str], names: set[str]) -> tuple[list[str], list[str]]:
    """The analysis paths and the held-out paths, when a change has both; empty lists otherwise."""
    analysis = [path for path in paths if ANALYSIS.match(path)]
    held_out = [path for path in paths if is_held_out(path, names)]
    if analysis and held_out:
        return analysis, held_out
    return [], []


def main() -> int:
    paths = [line.strip() for line in sys.stdin if line.strip()]
    names = held_out_names(Path(__file__).resolve().parent / "inputs.toml")
    analysis, held_out = violations(paths, names)
    if not analysis:
        print(f"held-out authorship: ok ({len(paths)} changed paths)")
        return 0
    print(f"{RULE}: this pull request changes analysis code and held-out bench files together.", file=sys.stderr)
    print("Commit the held-out driver and its recording in their own pull request.", file=sys.stderr)
    for path in analysis:
        print(f"  analysis: {path}", file=sys.stderr)
    for path in held_out:
        print(f"  held-out: {path}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
