#!/usr/bin/env python3
"""JS/TS analysis benchmark: fetch inputs, record observed calls, write the report.

Subcommands:

  fetch            download each pinned package tarball once, verify both
                   content hashes, and unpack it under the gitignored
                   research/evaluation-harness/repos/js-analysis-bench/.
  record-observed  run every construct case and package driver under the
                   runtime recorder inside a throwaway container (no network,
                   no credentials, read-only inputs) and write the observed
                   calls and exports under observed/. This executes third-party
                   code; it is a separate, explicit command and never part of
                   `cargo test`. The recorded files are committed.
  outside          build Jelly at its pinned commit and run it on every
                   tune-set package inside the same sandbox, writing its call
                   graphs gzipped under baseline/outside/. Explicit, needs
                   docker, never part of `cargo test`; the outputs are
                   committed and `report` scores them without docker.
  report           analyse every construct case and package with polint,
                   score the facts against hand labels and observed calls,
                   measure cost, and write baseline/report.json and
                   baseline/report.md (or --out DIR).

See README.md in this directory for the measures and their limits.
"""

from __future__ import annotations

import argparse
import base64
import gzip
import hashlib
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import urllib.request
from pathlib import Path

BENCH_DIR = Path(__file__).resolve().parent
REPO_ROOT = BENCH_DIR.parent.parent
INPUTS = BENCH_DIR / "inputs.toml"
CONSTRUCTS = BENCH_DIR / "constructs"
OBSERVED = BENCH_DIR / "observed"
RULE_PACK = BENCH_DIR / "rule-pack"
BASELINE = BENCH_DIR / "baseline"
PACKAGES_DIR = REPO_ROOT / "research" / "evaluation-harness" / "repos" / "js-analysis-bench"
SOURCE_SUFFIXES = (".js", ".mjs", ".cjs", ".ts", ".mts", ".cts")
FETCH_MARKER = ".polint-js-bench-fetched"
SANDBOX_MEMORY = "4g"
JELLY_IMAGE = "polint-js-bench-jelly"
JELLY_TIMEOUT_S = 900


def load_inputs() -> dict:
    with INPUTS.open("rb") as handle:
        data = tomllib.load(handle)
    if data.get("schema_version") != "polint-js-analysis-bench-inputs-1":
        raise SystemExit(f"unsupported schema in {INPUTS}")
    return data


def select_packages(inputs: dict, only: str) -> list[dict]:
    packages = inputs["package"]
    wanted = [name for name in only.split(",") if name]
    if not wanted:
        return packages
    known = {package["name"] for package in packages}
    unknown = [name for name in wanted if name not in known]
    if unknown:
        raise SystemExit(f"--only names unknown package(s): {', '.join(unknown)}")
    return [package for package in packages if package["name"] in wanted]


def package_dir(package: dict) -> Path:
    return PACKAGES_DIR / f"{package['name']}-{package['version']}"


def glob_regex(pattern: str) -> re.Pattern[str]:
    """Translate a `/`-separated glob with `*` and `**` to an anchored regex."""
    parts = []
    for index, segment in enumerate(pattern.split("/")):
        if segment == "**":
            last = index == len(pattern.split("/")) - 1
            parts.append(".*" if last else "(?:[^/]+/)*")
            continue
        piece = "".join("[^/]*" if char == "*" else re.escape(char) for char in segment)
        parts.append(piece + ("/" if index < len(pattern.split("/")) - 1 else ""))
    return re.compile("^" + "".join(parts) + "$")


def matching_files(root: Path, patterns: list[str]) -> list[str]:
    regexes = [glob_regex(pattern) for pattern in patterns]
    found = []
    for path in root.rglob("*"):
        if not path.is_file():
            continue
        rel = path.relative_to(root).as_posix()
        if rel.split("/")[0] == "node_modules":
            continue
        if any(regex.match(rel) for regex in regexes):
            found.append(rel)
    return sorted(found)


# Paths that are a package's tests or benchmarks rather than its shipped source.
# Some packages publish them (zod's tarball carries `src/**/tests/`), but the
# library never imports them and no driver runs them.
NOT_SHIPPED_RE = re.compile(r"(^|/)(tests?|__tests__|benchmarks?)/|\.(test|spec)\.[cm]?[jt]sx?$")


def is_not_shipped(rel: str) -> bool:
    return bool(NOT_SHIPPED_RE.search(rel))


def package_files(package: dict, shipped_only: bool = True) -> tuple[list[str], list[str]]:
    """Return (analysed files, files whose exports are recorded).

    The analysed set of every package is its shipped source: the `analysed`
    globs, without test and benchmark paths. `shipped_only=False` gives the
    set the globs alone select, which the report keeps for one comparison
    with reports written before the rule."""
    root = package_dir(package)
    analysed = matching_files(root, package["analysed"])
    if shipped_only:
        analysed = [rel for rel in analysed if not is_not_shipped(rel)]
    skipped = set(matching_files(root, package.get("exports_skip", [])))
    return analysed, [rel for rel in analysed if rel not in skipped]


def construct_cases() -> list[dict]:
    cases = []
    for manifest in sorted(CONSTRUCTS.glob("*/*/case.toml")):
        with manifest.open("rb") as handle:
            meta = tomllib.load(handle)
        root = manifest.parent
        files = sorted(
            path.relative_to(root).as_posix()
            for path in root.rglob("*")
            if path.is_file() and path.suffix in SOURCE_SUFFIXES
        )
        entry = next(name for name in ("main.js", "main.ts", "main.mjs") if (root / name).exists())
        cases.append(
            {
                "id": f"{root.parent.name}/{root.name}",
                "construct": meta["construct"],
                "held_out": bool(meta["held_out"]),
                "description": meta["description"],
                "root": root,
                "files": files,
                "entry": entry,
            }
        )
    return cases


# ---------------------------------------------------------------------------
# fetch


def fetch(args: argparse.Namespace) -> int:
    inputs = load_inputs()
    PACKAGES_DIR.mkdir(parents=True, exist_ok=True)
    for package in select_packages(inputs, args.only):
        dest = package_dir(package)
        marker = dest / FETCH_MARKER
        if marker.exists() and marker.read_text().strip() == package["sha256"]:
            print(f"ok {dest.name} (already fetched)", file=sys.stderr)
            continue
        tarball = PACKAGES_DIR / f"{dest.name}.tgz"
        if not tarball.exists():
            print(f"downloading {package['origin']}", file=sys.stderr)
            with urllib.request.urlopen(package["origin"], timeout=120) as response:
                tarball.write_bytes(response.read())
        data = tarball.read_bytes()
        sha256 = hashlib.sha256(data).hexdigest()
        algorithm, _, expected = package["integrity"].partition("-")
        integrity = base64.b64encode(hashlib.new(algorithm, data).digest()).decode()
        if sha256 != package["sha256"] or integrity != expected:
            tarball.unlink()
            raise SystemExit(f"{dest.name}: content hash mismatch; refusing to unpack")
        if dest.exists():
            shutil.rmtree(dest)
        dest.mkdir()
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
            members = []
            for member in archive.getmembers():
                head, _, rest = member.name.partition("/")
                if head != "package" or not rest:
                    continue
                member.name = rest
                members.append(member)
            archive.extractall(dest, members=members, filter="data")
        marker.write_text(package["sha256"] + "\n")
        print(f"ok {dest.name}", file=sys.stderr)
    return 0


def require_fetched(package: dict) -> Path:
    root = package_dir(package)
    marker = root / FETCH_MARKER
    if not marker.exists() or marker.read_text().strip() != package["sha256"]:
        raise SystemExit(f"{root} is missing or stale; run `bench.py fetch` first")
    return root


# ---------------------------------------------------------------------------
# record-observed


def sandbox_flags(*, tmp_size: str) -> list[str]:
    """The `docker run` flags every container of the bench runs third-party code with."""
    return [
        "docker", "run", "--rm",
        "--user", f"{os.getuid()}:{os.getgid()}",
        "--network", "none",
        "--read-only",
        "--tmpfs", f"/tmp:rw,size={tmp_size}",
        "--cap-drop", "ALL",
        "--security-opt", "no-new-privileges",
        "--pids-limit", "256",
        "--memory", SANDBOX_MEMORY,
        "-e", "HOME=/tmp",
    ]


def sandbox_run(
    inputs: dict,
    *,
    tree: Path,
    mode: str,
    files: list[str],
    out: Path,
    driver: str | None = None,
    stubs: bool = False,
) -> str:
    """Run the recorder once in a throwaway container and return its stderr."""
    out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="polint-js-bench-out-") as scratch:
        os.chmod(scratch, 0o777)
        argv = [
            *sandbox_flags(tmp_size="64m"),
            "-e", "BENCH_ROOT=/bench/pkg",
            "-e", f"BENCH_MODE={mode}",
            "-e", "BENCH_OUT=/out/result.json",
            "-e", f"BENCH_FILES={json.dumps(files)}",
            "-v", f"{tree}:/bench/pkg:ro",
            "-v", f"{OBSERVED}:/bench/observed:ro",
            "-v", f"{scratch}:/out",
        ]
        if driver is not None:
            argv += ["-e", f"BENCH_DRIVER={driver}"]
        if stubs:
            argv += ["-v", f"{OBSERVED / 'stubs' / 'node_modules'}:/bench/node_modules:ro"]
        argv += [
            "-w", "/bench/pkg",
            inputs["sandbox"]["image"],
            "node", "--expose-internals", "--import", "/bench/observed/recorder.mjs", "-e", "0",
        ]
        completed = subprocess.run(argv, capture_output=True, text=True, timeout=900)
        if completed.returncode != 0:
            # The recorder catches driver errors and records them as failures,
            # so a non-zero exit is the container or node itself failing; a
            # result written before that is not trusted.
            raise SystemExit(f"recorder exited {completed.returncode} for {tree} ({mode}):\n{completed.stderr}")
        result = Path(scratch) / "result.json"
        if not result.exists():
            raise SystemExit(f"recorder wrote nothing for {tree} ({mode}):\n{completed.stderr}")
        document = json.loads(result.read_text())
    if document.get("node") != inputs["sandbox"]["node_version"]:
        raise SystemExit(f"sandbox ran node {document.get('node')}, expected {inputs['sandbox']['node_version']}")
    out.write_text(json.dumps(document, indent=2, sort_keys=False) + "\n")
    return completed.stderr


def record_observed(args: argparse.Namespace) -> int:
    inputs = load_inputs()
    if args.target in ("all", "constructs"):
        for case in construct_cases():
            base = OBSERVED / "constructs" / case["id"]
            for mode in ("calls", "exports"):
                stderr = sandbox_run(
                    inputs,
                    tree=case["root"],
                    mode=mode,
                    files=case["files"],
                    out=base.with_name(f"{base.name}.{mode}.json"),
                    driver=f"/bench/pkg/{case['entry']}" if mode == "calls" else None,
                )
                report_stderr(case["id"], mode, stderr)
            print(f"ok {case['id']}", file=sys.stderr)
    if args.target in ("all", "packages"):
        for package in select_packages(inputs, args.only):
            tree = require_fetched(package)
            analysed, exported = package_files(package)
            drivers = OBSERVED / "drivers"
            for mode, files in (("calls", analysed), ("exports", exported)):
                argv_driver = None
                if mode == "calls":
                    argv_driver = f"/bench/pkg/.polint-js-bench-driver/{Path(package['driver']).name}"
                with tempfile.TemporaryDirectory(prefix="polint-js-bench-tree-") as staging:
                    # The driver is mounted beside the package so bare and
                    # relative specifiers resolve as they would for a consumer.
                    staged = Path(staging) / "pkg"
                    shutil.copytree(tree, staged, symlinks=True)
                    (staged / ".polint-js-bench-driver").mkdir()
                    shutil.copy(drivers / Path(package["driver"]).name, staged / ".polint-js-bench-driver")
                    make_readable(staged)
                    stderr = sandbox_run(
                        inputs,
                        tree=staged,
                        mode=mode,
                        files=files,
                        out=OBSERVED / "packages" / f"{package['name']}.{mode}.json",
                        driver=argv_driver,
                        stubs=bool(package.get("stubs")),
                    )
                report_stderr(package["name"], mode, stderr)
            print(f"ok {package['name']}", file=sys.stderr)
    return 0


# ---------------------------------------------------------------------------
# outside


def outside(args: argparse.Namespace) -> int:
    """Build the pinned Jelly image and run it on the tune set in the sandbox."""
    sys.path.insert(0, str(BENCH_DIR / "outside"))
    import score_outside

    inputs = load_inputs()
    subprocess.run(
        [
            "docker", "build", "--quiet",
            "--build-arg", f"NODE_IMAGE={inputs['sandbox']['image']}",
            "--build-arg", f"JELLY_COMMIT={score_outside.JELLY_COMMIT}",
            "-t", JELLY_IMAGE,
            "-f", str(BENCH_DIR / "outside" / "Dockerfile.jelly"),
            str(BENCH_DIR / "outside"),
        ],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    for package in select_packages(inputs, args.only):
        if package.get("held_out"):
            continue
        tree = require_fetched(package)
        files, _ = package_files(package)
        for run, flags in score_outside.JELLY_RUNS.items():
            meta = run_jelly(tree, files, flags, score_outside.raw_path(run, package["name"]))
            score_outside.meta_path(run, package["name"]).write_text(json.dumps(meta, indent=2) + "\n")
            print(f"{run} {package['name']}: exit {meta['exit_code']}, {meta['wall_s']} s", file=sys.stderr)
    return 0


def run_jelly(tree: Path, files: list[str], flags: list[str], out: Path) -> dict:
    """One Jelly run on one package tree; its call graph is written gzipped to `out`."""
    import time

    # Jelly's own analysis timeout comes first, so a slow run still writes the
    # graph it has.
    tool_argv = ["-j", "/out/cg.json", "-b", "/bench/pkg", "-i", str(JELLY_TIMEOUT_S - 60), "--ignore-unresolved",
                 "--no-print-progress", "--no-tty", *flags]
    heap_mb = int(SANDBOX_MEMORY.rstrip("g")) * 1024 - 512
    out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="polint-js-bench-outside-") as scratch:
        os.chmod(scratch, 0o777)
        argv = [
            *sandbox_flags(tmp_size="256m"),
            "-e", f"NODE_OPTIONS=--max-old-space-size={heap_mb}",
            "-v", f"{tree}:/bench/pkg:ro",
            "-v", f"{scratch}:/out",
            "-w", "/bench/pkg",
            JELLY_IMAGE, *tool_argv, *files,
        ]
        started = time.monotonic()
        completed = subprocess.run(argv, capture_output=True, text=True, timeout=JELLY_TIMEOUT_S)
        wall_s = round(time.monotonic() - started, 1)
        result = Path(scratch) / "cg.json"
        if completed.returncode != 0 or not result.exists():
            raise SystemExit(f"jelly exited {completed.returncode} for {tree}:\n{completed.stderr[-4000:]}")
        out.write_bytes(gzip.compress(result.read_bytes(), mtime=0))
    return {
        "tool": "jelly",
        "image": JELLY_IMAGE,
        "arguments": tool_argv,
        "files": len(files),
        "exit_code": completed.returncode,
        "wall_s": wall_s,
    }


def make_readable(root: Path) -> None:
    for path in [root, *root.rglob("*")]:
        if path.is_symlink():
            continue
        path.chmod(0o755 if path.is_dir() else 0o644)


def report_stderr(name: str, mode: str, stderr: str) -> None:
    lines = [line for line in stderr.splitlines() if line.strip() and "ExperimentalWarning" not in line]
    lines = [line for line in lines if "--trace-warnings" not in line]
    if lines:
        print(f"{name} ({mode}) stderr:", file=sys.stderr)
        for line in lines[:20]:
            print(f"  {line}", file=sys.stderr)


# ---------------------------------------------------------------------------
# entry point


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)

    fetch_parser = commands.add_parser("fetch", help="download and unpack the pinned packages")
    fetch_parser.add_argument("--only", default="", help="comma-separated package names")
    fetch_parser.set_defaults(run=fetch)

    record_parser = commands.add_parser("record-observed", help="record observed calls in the sandbox")
    record_parser.add_argument("--target", choices=["all", "constructs", "packages"], default="all")
    record_parser.add_argument("--only", default="", help="comma-separated package names")
    record_parser.set_defaults(run=record_observed)

    report_parser = commands.add_parser("report", help="analyse, score and measure; write the report")
    report_parser.add_argument("--out", default="", help="directory for report.json and report.md (default baseline/)")
    report_parser.add_argument("--no-cost", action="store_true", help="skip the cost measurements")
    report_parser.add_argument("--cost-runs", type=int, default=3, help="cold and warm runs per tree; the median is kept")
    report_parser.add_argument(
        "--dump-timeout",
        type=int,
        default=600,
        help="seconds one tree's fact dump or check may take before it is recorded as timed out",
    )
    report_parser.add_argument(
        "--typescript",
        default="",
        help="TypeScript package directory for the typed plan (default: $POLINT_TS_TYPESCRIPT); "
        "without one the typed plan is not run",
    )
    report_parser.add_argument("--keep-dumps", default="", help="directory to copy the raw fact dumps into")
    report_parser.add_argument("--no-types", action="store_true", help="skip the typed plan even with a compiler")
    report_parser.set_defaults(run=run_report)

    outside_parser = commands.add_parser("outside", help="run the pinned outside tools in the sandbox")
    outside_parser.add_argument("--only", default="", help="comma-separated package names")
    outside_parser.set_defaults(run=outside)

    selftest_parser = commands.add_parser("selftest", help="check the report's tier measures on a synthetic package")
    selftest_parser.set_defaults(run=run_selftest)

    args = parser.parse_args()
    return args.run(args)


def run_selftest(_args: argparse.Namespace) -> int:
    import selftest

    return selftest.main()


def run_report(args: argparse.Namespace) -> int:
    import report

    return report.run(args)


if __name__ == "__main__":
    sys.exit(main())
