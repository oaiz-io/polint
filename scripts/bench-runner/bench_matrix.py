#!/usr/bin/env python3
"""Measure polint wall-clock, peak RSS, and output identity on public corpora.

Every repository measured here is public open source, pinned to the commit SHA
its suite manifest declares, with the license recorded beside the numbers.
The target list below is an explicit allowlist: suites are never discovered by
globbing, so a local-only manifest cannot leak into a published run.

Two workloads are measured and they are never averaged together:

  syntactic  `polint facts sample --cap file_metrics` - whole-repo file walk,
             parse, and per-file metrics. The floor cost of pointing polint at
             a repository.
  deep       `polint inspect unknowns` - whole-repo analysis over every public
             unknown capability (resolved_imports, symbols, references, events,
             calls, control_flow, dataflow). The deepest pipeline reachable from
             the shipped CLI without a repo-local rule pack, so it is a
             capability-gated run, not a `polint check` run.

Three cache tiers per workload, three runs per tier, strictly sequential:

  warm      cache primed by an unmeasured run, then measured with it present
  cold      `<repo>/.polint/cache` deleted before every measured run
  no-cache  cache deleted and `--no-cache` passed, so nothing is read or written

Wall clock is `time.monotonic_ns` around the child. Peak RSS is the child's own
`rusage.ru_maxrss` from `os.wait4`, which is the field `/usr/bin/time -v` prints
as "Maximum resident set size"; taking it in-process keeps CI and a laptop on
one code path instead of depending on GNU time being installed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import signal
import statistics
import subprocess
import sys
import threading
import time
import tomllib
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

SCHEMA_VERSION = "polint-bench-perf-1"

# Physical-line counting rules, kept identical to the committed scale-corpus
# artifact so LOC printed here is comparable with LOC printed there.
LOC_EXTENSIONS = {"go", "ts", "tsx", "js", "jsx", "mjs", "cjs"}
LOC_SKIP_DIR_NAMES = {
    ".git",
    "node_modules",
    "vendor",
    "dist",
    "build",
    "target",
    ".next",
    "coverage",
}


@dataclass(frozen=True)
class Workload:
    id: str
    label: str
    argv: tuple[str, ...]
    no_cache_argv: tuple[str, ...]
    note: str


WORKLOADS = {
    "syntactic": Workload(
        id="syntactic",
        label="syntactic (whole-repo file metrics)",
        argv=("facts", "sample", "--cap", "file_metrics", "--limit", "1", "--format", "json"),
        no_cache_argv=("--no-cache",),
        note=(
            "`polint facts sample --cap file_metrics`. Walks and parses the whole "
            "repository and computes per-file metrics; `--limit 1` bounds only how many "
            "rows are printed, not how much is analysed."
        ),
    ),
    "deep": Workload(
        id="deep",
        label="deep capability-gated (7 public capabilities incl. dataflow)",
        argv=("inspect", "unknowns", "--format", "json"),
        no_cache_argv=("--no-cache",),
        note=(
            "`polint inspect unknowns`. Plans resolved_imports, symbols, references, "
            "events, calls, control_flow and dataflow over the whole repository. This is "
            "a capability-gated kernel run, NOT `polint check` with a rule pack: it "
            "excludes rule-host compilation and rule execution."
        ),
    ),
}

TIERS = ("warm", "cold", "no-cache")


@dataclass(frozen=True)
class Target:
    id: str
    suite_manifest: str
    group: str
    languages: str
    workloads: tuple[str, ...]
    note: str = ""


# Public suites only. `group` decides default inclusion:
#   accuracy-corpus  the two pinned call-graph oracle clones, also real codebases
#   scale            the pinned large public repositories (opt in with --scale)
#   scale-heavy      too large for a hosted runner; local only (--scale-heavy)
TARGETS: tuple[Target, ...] = (
    Target(
        id="jelly",
        suite_manifest="research/evaluation-harness/suites/jelly-callgraph-micro.toml",
        group="accuracy-corpus",
        languages="JavaScript/TypeScript",
        workloads=("syntactic", "deep"),
    ),
    Target(
        id="golang-tools",
        suite_manifest="research/evaluation-harness/suites/go-x-tools-rta-callgraph.toml",
        group="accuracy-corpus",
        languages="Go",
        workloads=("syntactic", "deep"),
    ),
    Target(
        id="excalidraw",
        suite_manifest="research/evaluation-harness/suites/excalidraw-excalidraw-scale.toml",
        group="scale",
        languages="TypeScript",
        workloads=("syntactic", "deep"),
    ),
    Target(
        id="hugo",
        suite_manifest="research/evaluation-harness/suites/gohugoio-hugo-scale.toml",
        group="scale",
        languages="Go",
        workloads=("syntactic", "deep"),
    ),
    Target(
        id="grafana",
        suite_manifest="research/evaluation-harness/suites/grafana-grafana-scale.toml",
        group="scale-heavy",
        languages="Go + TypeScript",
        workloads=("syntactic", "deep"),
        note="1.5M LOC. Excluded from CI: it does not fit a hosted runner's disk or memory.",
    ),
)


# A benchmark may only publish numbers from a public repository under a license
# that has already been reviewed. `license-review-needed` is not a license, and a
# manifest with no upstream URL is not a public corpus.
REJECTED_LICENSES = {"proprietary", "license-review-needed", "unknown", ""}


def load_pin(repo_root: Path, target: Target) -> dict[str, object]:
    manifest_path = repo_root / target.suite_manifest
    with manifest_path.open("rb") as handle:
        suite = tomllib.load(handle)
    license_name = str(suite.get("license") or "")
    source_url = str(suite.get("source_url") or "")
    if license_name.lower() in REJECTED_LICENSES:
        raise SystemExit(
            f"{manifest_path}: license `{license_name or 'missing'}` may not be benchmarked; "
            "this runner publishes numbers only from reviewed public licenses"
        )
    if not source_url.startswith("https://github.com/"):
        raise SystemExit(
            f"{manifest_path}: source_url `{source_url or 'missing'}` is not a public "
            "upstream; this runner benchmarks public repositories only"
        )
    checkout = suite.get("checkout") or {}
    return {
        "suite_id": suite.get("id"),
        "suite_name": suite.get("name"),
        "source_url": suite.get("source_url"),
        "source_commit": suite.get("source_commit"),
        "license": suite.get("license"),
        "checkout_path": checkout.get("path"),
    }


def count_loc(root: Path) -> tuple[int, int]:
    """Physical source lines and file count under `root` (LOC_* rules)."""
    lines = 0
    files = 0
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [name for name in dirnames if name not in LOC_SKIP_DIR_NAMES]
        for name in filenames:
            extension = name.rsplit(".", 1)[-1].lower() if "." in name else ""
            if extension not in LOC_EXTENSIONS:
                continue
            path = Path(dirpath) / name
            try:
                with path.open("rb") as handle:
                    lines += sum(1 for _ in handle)
            except OSError:
                continue
            files += 1
    return lines, files


def directory_bytes(root: Path) -> int:
    total = 0
    for dirpath, _dirnames, filenames in os.walk(root):
        for name in filenames:
            try:
                total += (Path(dirpath) / name).lstat().st_size
            except OSError:
                continue
    return total


@dataclass
class RunResult:
    index: int
    wall_ms: float
    peak_rss_bytes: int
    exit_code: int | None
    signal_name: str | None
    timed_out: bool
    stdout_md5: str
    stdout_bytes: int
    stderr_tail: str
    cache_bytes_after: int
    # The whole of stderr, for callers that parse log lines from it.
    stderr: str = ""


def measure(
    argv: list[str], cwd: Path, env: dict[str, str], timeout_seconds: int
) -> RunResult:
    """Run `argv` once and return its wall clock, peak RSS, and output digest."""
    digest = hashlib.md5()  # noqa: S324 - output identity monitoring, not security
    stdout_bytes = 0
    timed_out = threading.Event()
    stderr_chunks: list[bytes] = []

    proc = subprocess.Popen(
        argv,
        cwd=str(cwd),
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    started = time.monotonic_ns()

    def kill_group() -> None:
        timed_out.set()
        try:
            os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            pass

    watchdog = threading.Timer(timeout_seconds, kill_group)
    watchdog.start()

    def drain_stderr() -> None:
        assert proc.stderr is not None
        stderr_chunks.append(proc.stderr.read())

    stderr_reader = threading.Thread(target=drain_stderr)
    stderr_reader.start()

    # Drain stdout so a large output cannot deadlock the child, and hash it as it
    # streams rather than holding tens of megabytes of JSON in memory.
    assert proc.stdout is not None
    while chunk := proc.stdout.read(1 << 20):
        digest.update(chunk)
        stdout_bytes += len(chunk)
    proc.stdout.close()
    stderr_reader.join()
    assert proc.stderr is not None
    proc.stderr.close()

    try:
        _, status, usage = os.wait4(proc.pid, 0)
        peak_rss_kb = usage.ru_maxrss
        returncode = os.waitstatus_to_exitcode(status)
    except ChildProcessError:
        # Already reaped elsewhere; the exit status is still recoverable.
        returncode = proc.wait()
        peak_rss_kb = 0
    elapsed_ns = time.monotonic_ns() - started
    watchdog.cancel()
    # Tell Popen the child is gone so it never waits on a reaped pid.
    proc.returncode = returncode

    exit_code: int | None = returncode
    signal_name = None
    if returncode < 0:
        signal_name = signal.Signals(-returncode).name
        exit_code = None

    stderr_text = b"".join(chunk for chunk in stderr_chunks if chunk).decode("utf-8", "replace")
    return RunResult(
        index=-1,
        wall_ms=elapsed_ns / 1e6,
        # ru_maxrss is kibibytes on Linux and bytes on macOS.
        peak_rss_bytes=peak_rss_kb * (1 if sys.platform == "darwin" else 1024),
        exit_code=exit_code,
        signal_name=signal_name,
        # Only a child the watchdog actually killed counts as timed out; the timer
        # can also fire in the instant between wait4 returning and cancel().
        timed_out=timed_out.is_set() and returncode < 0,
        stdout_md5=digest.hexdigest(),
        stdout_bytes=stdout_bytes,
        stderr_tail="\n".join(stderr_text.splitlines()[-8:]),
        cache_bytes_after=0,
        stderr=stderr_text,
    )


def median(values: list[float]) -> float | None:
    return statistics.median(values) if values else None


def run_cell(
    *,
    polint_bin: Path,
    checkout: Path,
    workload: Workload,
    tier: str,
    runs: int,
    timeout_seconds: int,
    env: dict[str, str],
) -> dict[str, object]:
    cache_dir = checkout / ".polint" / "cache"
    argv = [str(polint_bin), *workload.argv]
    if tier == "no-cache":
        argv += list(workload.no_cache_argv)

    if tier == "warm":
        # One unmeasured priming run so the first measured run sees a full cache.
        shutil.rmtree(cache_dir, ignore_errors=True)
        prime = measure(argv, checkout, env, timeout_seconds)
        if prime.exit_code != 0:
            return {
                "workload": workload.id,
                "tier": tier,
                "status": "timeout" if prime.timed_out else "failed",
                "detail": (
                    f"cache-priming run failed (exit={prime.exit_code} "
                    f"signal={prime.signal_name} timed_out={prime.timed_out}); "
                    f"stderr tail: {prime.stderr_tail}"
                ),
                "runs": [],
                "wall_ms_median": None,
                "peak_rss_bytes_median": None,
                "cache_bytes_median": None,
                "stdout_md5": None,
                "stdout_md5_stable": None,
            }

    results: list[RunResult] = []
    for index in range(runs):
        if tier != "warm":
            shutil.rmtree(cache_dir, ignore_errors=True)
        result = measure(argv, checkout, env, timeout_seconds)
        result.index = index
        result.cache_bytes_after = directory_bytes(cache_dir) if cache_dir.is_dir() else 0
        results.append(result)
        print(
            f"    {workload.id}/{tier} run {index + 1}/{runs}: "
            f"{result.wall_ms / 1000:.2f}s, "
            f"{result.peak_rss_bytes / 1024 / 1024:.0f} MiB peak RSS, "
            f"exit={result.exit_code} signal={result.signal_name}",
            file=sys.stderr,
        )
        if result.exit_code != 0:
            # Stop the cell on the first failure: repeating a crash burns runner
            # minutes and the failure is what has to be reported, not a median.
            break

    ok = [r for r in results if r.exit_code == 0]
    digests = {r.stdout_md5 for r in ok}
    failures = [r for r in results if r.exit_code != 0]
    if failures:
        first = failures[0]
        status = "timeout" if first.timed_out else ("killed" if first.signal_name else "failed")
        detail = (
            f"run {first.index + 1} exit={first.exit_code} signal={first.signal_name} "
            f"timed_out={first.timed_out}; stderr tail: {first.stderr_tail}"
        )
    else:
        status = "ok"
        detail = None

    return {
        "workload": workload.id,
        "tier": tier,
        "status": status,
        "detail": detail,
        "runs": [
            {
                "index": r.index,
                "wall_ms": round(r.wall_ms, 1),
                "peak_rss_bytes": r.peak_rss_bytes,
                "exit_code": r.exit_code,
                "signal": r.signal_name,
                "timed_out": r.timed_out,
                "stdout_md5": r.stdout_md5,
                "stdout_bytes": r.stdout_bytes,
                "cache_bytes_after": r.cache_bytes_after,
            }
            for r in results
        ],
        "wall_ms_median": round(median([r.wall_ms for r in ok]) or 0.0, 1) if ok else None,
        "peak_rss_bytes_median": int(median([r.peak_rss_bytes for r in ok]) or 0) if ok else None,
        "cache_bytes_median": int(median([r.cache_bytes_after for r in ok]) or 0) if ok else None,
        "stdout_md5": sorted(digests)[0] if len(digests) == 1 else None,
        "stdout_md5_stable": len(digests) == 1 if ok else None,
    }


def selected_targets(scale: bool, scale_heavy: bool, only: list[str]) -> list[Target]:
    groups = {"accuracy-corpus"}
    if scale:
        groups.add("scale")
    if scale_heavy:
        groups.update({"scale", "scale-heavy"})
    chosen = [target for target in TARGETS if target.group in groups]
    if only:
        chosen = [target for target in chosen if target.id in only]
    return chosen


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, required=True)
    parser.add_argument("--polint-bin", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True, help="Write perf.json here")
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--timeout-seconds", type=int, default=1200)
    parser.add_argument("--scale", action="store_true", help="Add the scale-tier repositories")
    parser.add_argument(
        "--scale-heavy",
        action="store_true",
        help="Add the scale repositories too large for a hosted runner (local only)",
    )
    parser.add_argument(
        "--workloads",
        default="syntactic,deep",
        help="Comma-separated workload ids to measure (default: syntactic,deep)",
    )
    parser.add_argument(
        "--deep-targets",
        default="jelly",
        help=(
            "Comma-separated target ids that run the `deep` workload, or `all`. "
            "The deep workload costs minutes per run on a large repository, so the "
            "default keeps a hosted-runner budget bounded (default: jelly)"
        ),
    )
    parser.add_argument(
        "--only",
        default="",
        help="Comma-separated target ids to restrict the run to",
    )
    args = parser.parse_args()

    repo_root = args.repo_root.resolve()
    polint_bin = args.polint_bin.resolve()
    if not polint_bin.is_file():
        raise SystemExit(f"polint binary not found: {polint_bin}")

    wanted_workloads = [name for name in args.workloads.split(",") if name]
    for name in wanted_workloads:
        if name not in WORKLOADS:
            raise SystemExit(f"unknown workload `{name}`; known: {', '.join(WORKLOADS)}")

    only = [name for name in args.only.split(",") if name]
    targets = selected_targets(args.scale, args.scale_heavy, only)
    deep_targets = (
        {target.id for target in TARGETS}
        if args.deep_targets.strip() == "all"
        else {name for name in args.deep_targets.split(",") if name}
    )

    # Keep the child deterministic and colour-free; the digest is the point.
    env = dict(os.environ)
    env["NO_COLOR"] = "1"
    env.pop("POLINT_CACHE_DIR", None)

    entries: list[dict[str, object]] = []
    report: dict[str, object] = {
        "schema_version": SCHEMA_VERSION,
        "started_at_utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "runs_per_cell": args.runs,
        "timeout_seconds": args.timeout_seconds,
        "deep_targets": sorted(deep_targets),
        "measurement": {
            "wall_clock": "time.monotonic_ns around the child process",
            "peak_rss": "child rusage.ru_maxrss via os.wait4 (same field as /usr/bin/time -v)",
            "median": "statistics.median over the successful runs in the cell",
            "output_identity": "md5 of the child's stdout, compared across runs and tiers",
        },
        "workloads": [
            {
                "id": workload.id,
                "label": workload.label,
                "command": " ".join(["polint", *workload.argv]),
                "no_cache_command": " ".join(
                    ["polint", *workload.argv, *workload.no_cache_argv]
                ),
                "note": workload.note,
            }
            for name, workload in WORKLOADS.items()
            if name in wanted_workloads
        ],
        "targets": entries,
        "excluded": [
            {
                "id": target.id,
                "group": target.group,
                "license": load_pin(repo_root, target)["license"],
                "reason": target.note or f"group `{target.group}` not selected for this run",
            }
            for target in TARGETS
            if target not in targets
        ],
    }

    for target in targets:
        pin = load_pin(repo_root, target)
        checkout = repo_root / str(pin["checkout_path"])
        cells: list[dict[str, object]] = []
        entry: dict[str, object] = {
            "id": target.id,
            "group": target.group,
            "languages": target.languages,
            **pin,
            "present": checkout.is_dir(),
            "cells": cells,
        }
        entries.append(entry)

        if not checkout.is_dir():
            entry["skip_reason"] = f"checkout missing at {pin['checkout_path']}"
            print(f"  {target.id}: SKIPPED, {entry['skip_reason']}", file=sys.stderr)
            continue

        head = subprocess.run(
            ["git", "-C", str(checkout), "rev-parse", "HEAD"],
            check=False,
            capture_output=True,
            text=True,
        )
        entry["checkout_commit"] = head.stdout.strip() or None
        loc, file_count = count_loc(checkout)
        entry["loc"] = loc
        entry["source_file_count"] = file_count
        print(f"  {target.id}: {loc} LOC across {file_count} source files", file=sys.stderr)

        skipped_workloads: list[dict[str, str]] = []
        entry["skipped_workloads"] = skipped_workloads
        for name in wanted_workloads:
            if name not in target.workloads:
                continue
            if name == "deep" and target.id not in deep_targets:
                skipped_workloads.append(
                    {
                        "workload": name,
                        "reason": (
                            "not selected by --deep-targets; the deep workload costs "
                            "minutes per run on a repository this size, so it is opt-in "
                            "(`make bench-run DEEP_TARGETS=all`)"
                        ),
                    }
                )
                print(f"    {name}: not selected by --deep-targets", file=sys.stderr)
                continue
            workload = WORKLOADS[name]
            for tier in TIERS:
                cell = run_cell(
                    polint_bin=polint_bin,
                    checkout=checkout,
                    workload=workload,
                    tier=tier,
                    runs=args.runs,
                    timeout_seconds=args.timeout_seconds,
                    env=env,
                )
                cells.append(cell)
                if cell["status"] != "ok":
                    # A workload that cannot complete on this machine is reported
                    # once, not re-attempted per tier.
                    print(
                        f"    {name}: {cell['status']} - skipping remaining tiers",
                        file=sys.stderr,
                    )
                    break

        # Output identity across cache tiers: the same binary on the same commit
        # must print the same bytes whether or not a cache was involved.
        by_workload: dict[str, set[str]] = {}
        for cell in cells:
            if cell["stdout_md5"]:
                by_workload.setdefault(str(cell["workload"]), set()).add(str(cell["stdout_md5"]))
        entry["output_identity"] = {
            workload_id: {
                "digests": sorted(digests),
                "identical_across_tiers": len(digests) == 1,
            }
            for workload_id, digests in by_workload.items()
        }
        # Leave the checkout without a benchmark cache so the next run is honest.
        shutil.rmtree(checkout / ".polint" / "cache", ignore_errors=True)

    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2)
        handle.write("\n")
    print(f"wrote {args.out}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
