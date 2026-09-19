#!/usr/bin/env python3
"""Read the sidecar-present window out of an `rssrun-timeline-2` timeline.

    overlap.py <timeline.json> --stderr <probe stderr>

The tree peak of a Go cell is a sum of two processes that are resident at the
same time, so a single peak figure cannot say how much of it was polint. This
splits the run at the Go semantic sidecar's window and reports the seven numbers
G2b and G6 bind or record: the window itself, the maximum tree total, polint RSS
and sidecar RSS inside it, the maximum polint RSS outside it, the maximum tree
total over the run, and the sample count.

The polint process is the timeline's `root_pid`. A Go semantic sidecar is any
process matching either of two rules, and the rule that matched is printed:

  argv0  its argv0 basename, after one leading `.` is stripped, starts with
         `polint-go-frontend` (`.exe` tolerated). The default path spawns
         `.polint-go-frontend-<cache key>` from the materialized source
         directory, so an equality test against `polint-go-frontend` never
         matches the process polint actually runs.
  exe    its `exe` path has a `go-sidecars/semantic/` component, the
         materialized-sidecar directory, whatever the binary is called.

Everything else -- the symbol sidecar, a `go` toolchain build, a rule host -- is
`other` and is listed by basename. Which enumeration found a process (`via`) is
never consulted for the match: a process the scan alone caught is a sidecar
sample like any other.

Exit codes:

  0  a window was found, or the sidecar cache hit and none was expected
  2  the sidecar ran but no sidecar process was classified (fails the cell)
  3  no sidecar process and no `polint.go.semantic` stage row: the provider
     never ran, so the cell is not measurable here
  4  the timeline or the arguments cannot be read
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys

TIMELINE_SCHEMA = "rssrun-timeline-2"
ENUMERATION = "tree+scan"

SIDECAR_ARGV0_PREFIX = "polint-go-frontend"
SIDECAR_EXE_COMPONENT = "go-sidecars/semantic/"

ANSI = re.compile(r"\x1b\[[0-9;]*m")
SEMANTIC_PROVIDER = re.compile(r'provider="?polint\.go\.semantic"?(?![\w.])')
SIDECAR_CACHE_HIT = "sidecar cache hit"
STAGE_DONE = "stage done"

# Per-process record layout written by .scale-envelope/rssrun.py.
PID, COMM, ARGV0, EXE, RSS, VIA = range(6)

MB = 1 << 20

REJECTED = 4
SIDECAR_UNCLASSIFIED = 2
PROVIDER_NEVER_RAN = 3


def sidecar_rule(argv0: str, exe: str) -> str | None:
    """Which sidecar rule a process matches, or None."""
    name = os.path.basename(argv0)
    if name.startswith("."):
        name = name[1:]
    if name.startswith(SIDECAR_ARGV0_PREFIX):
        return "argv0"
    if SIDECAR_EXE_COMPONENT in exe:
        return "exe"
    return None


def reject(message: str) -> int:
    print(f"overlap.py: {message}", file=sys.stderr)
    return REJECTED


def load_timeline(path: str):
    """The timeline's summary and samples, or a message saying why not."""
    try:
        with open(path, encoding="utf-8") as handle:
            document = json.load(handle)
    except (OSError, ValueError) as error:
        return None, f"cannot read {os.path.basename(path)}: {error}"
    summary = document.get("summary") or {}
    schema = summary.get("timeline_schema")
    if schema is None:
        return None, (
            "this timeline has no timeline_schema: it was written by the sampler "
            "before W9 commit 1 gave rssrun.py a per-process timeline, so it holds "
            "one tree total per sample and cannot answer this gate"
        )
    if schema != TIMELINE_SCHEMA:
        return None, f"unknown timeline schema {schema!r}, expected {TIMELINE_SCHEMA!r}"
    if summary.get("enumeration") != ENUMERATION:
        return None, (
            f"this timeline is not enumerated {ENUMERATION!r}: it was produced by a "
            "sampler that walked the main thread's children only and cannot contain "
            "the Go semantic sidecar on a Go cell"
        )
    return (summary, document.get("timeline") or []), None


def stderr_evidence(path: str) -> tuple[bool, bool, str | None]:
    """(a polint.go.semantic stage row, a sidecar cache hit line, an error)."""
    stage_row = cache_hit = False
    try:
        with open(path, errors="replace") as handle:
            for line in handle:
                line = ANSI.sub("", line)
                if not SEMANTIC_PROVIDER.search(line):
                    continue
                if SIDECAR_CACHE_HIT in line:
                    cache_hit = True
                if STAGE_DONE in line:
                    stage_row = True
    except OSError as error:
        return False, False, f"cannot read the probe stderr: {error}"
    return stage_row, cache_hit, None


def classify(samples, root_pid: int):
    """Per sample: (t, total, polint rss, sidecar rss). Plus the process census."""
    rows = []
    sidecars: dict[int, dict] = {}
    others: dict[str, int] = {}
    for entry in samples:
        polint = sidecar = 0
        for proc in entry.get("procs") or []:
            pid, rss = proc[PID], proc[RSS]
            if pid == root_pid:
                polint = max(polint, rss)
                continue
            rule = sidecar_rule(proc[ARGV0], proc[EXE])
            if rule is None:
                name = proc[ARGV0] or proc[COMM] or "?"
                others[os.path.basename(name)] = others.get(os.path.basename(name), 0) + 1
                continue
            sidecar += rss
            seen = sidecars.setdefault(pid, {"argv0": proc[ARGV0], "rule": rule, "via": set()})
            seen["via"].add(proc[VIA])
        rows.append((entry.get("t"), entry.get("total") or 0, polint, sidecar))
    return rows, sidecars, others


def megabytes(value: int) -> str:
    return str(value // MB)


def main() -> int:
    parser = argparse.ArgumentParser(add_help=True)
    # argparse's own usage exit is 2, which is a self-check verdict here.
    parser.error = lambda message: sys.exit(reject(message))  # type: ignore[method-assign]
    parser.add_argument("timeline")
    parser.add_argument("--stderr", required=True, help="the probe stderr paired with it")
    args = parser.parse_args()

    loaded, problem = load_timeline(args.timeline)
    if problem is not None:
        return reject(problem)
    summary, samples = loaded
    root_pid = summary.get("root_pid")
    if not isinstance(root_pid, int):
        return reject("this timeline records no root_pid")

    rows, sidecars, others = classify(samples, root_pid)
    inside = [index for index, row in enumerate(rows) if row[3]]

    print(
        f"schema: {TIMELINE_SCHEMA}  enumeration: {ENUMERATION}  "
        f"root_pid: {root_pid}  root_start_ticks: {summary.get('root_start_ticks')}"
    )
    print(f"samples: {len(rows)}")

    if not inside:
        stage_row, cache_hit, error = stderr_evidence(args.stderr)
        if error is not None:
            return reject(error)
        if cache_hit:
            print("window: none (sidecar cache hit)")
        elif stage_row:
            print("window: none")
            print(
                "overlap.py: sidecar ran but no sidecar process was classified",
                file=sys.stderr,
            )
            return SIDECAR_UNCLASSIFIED
        else:
            print("window: none")
            print(
                "overlap.py: no sidecar process and no polint.go.semantic stage row: "
                "the provider never ran; this cell is not measurable by overlap.py",
                file=sys.stderr,
            )
            return PROVIDER_NEVER_RAN
        for label in (
            "max tree total in window MB",
            "max polint RSS in window MB",
            "max sidecar RSS in window MB",
        ):
            print(f"{label}: n/a")
        print(f"max polint RSS outside window MB: {megabytes(max((r[2] for r in rows), default=0))}")
    else:
        first, last = inside[0], inside[-1]
        window = rows[first : last + 1]
        outside = rows[:first] + rows[last + 1 :]
        print(
            f"window: {rows[first][0]} s .. {rows[last][0]} s "
            f"({len(window)} samples, {len(inside)} with a sidecar)"
        )
        print(f"max tree total in window MB: {megabytes(max(r[1] for r in window))}")
        print(f"max polint RSS in window MB: {megabytes(max(r[2] for r in window))}")
        print(f"max sidecar RSS in window MB: {megabytes(max(r[3] for r in window))}")
        print(
            f"max polint RSS outside window MB: "
            f"{megabytes(max((r[2] for r in outside), default=0))}"
        )

    print(f"max tree total over run MB: {megabytes(max((r[1] for r in rows), default=0))}")
    for pid in sorted(sidecars):
        seen = sidecars[pid]
        via = "+".join(sorted(seen["via"]))
        print(
            f"sidecar: pid={pid} argv0={os.path.basename(seen['argv0'])} "
            f"rule={seen['rule']} via={via}"
        )
    for name in sorted(others):
        print(f"other: {name} ({others[name]} samples)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
