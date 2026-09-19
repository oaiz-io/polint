#!/usr/bin/env python3
"""Fold a gate run directory into the committed report format (plan section 6).

    report.py <gate-out-dir> --host <label> --polint <version>@<sha> [--date YYYY-MM-DD]
              [--out <path>]

A gate run leaves `polint unknowns` stdout and a full debug-level stderr under
$POLINT_GATE_OUT. Neither may be committed: the stdout is an agent JSON report
over a consumer tree and the stderr carries scanned paths. This is the only
writer of a committed report, and it will not copy a line it cannot recognise.

Permitted content: file counts, timings, sizes, provider ids, rule ids of
built-in diagnostics, gate names. Forbidden: any scanned file path beyond a
basename, any diagnostic message text, any consumer identifier. The allowlist is
positive, not a blocklist: a line is folded only when it parses as one of

  * a `stage done` row from polint::kernel::stage,
  * the sampler's one-line JSON summary,
  * a resource-budget diagnostic count,

and any other stderr line is dropped. `--strict` turns a dropped line into an
error, which is what the unit test asserts.

Exit codes: 0 written, 2 the run directory or the arguments cannot be read,
3 a line was rejected under --strict.
"""
from __future__ import annotations

import argparse
import datetime
import json
import os
import re
import sys

ANSI = re.compile(r"\x1b\[[0-9;]*m")

STAGE_ROW = re.compile(
    r'provider="?(?P<provider>[\w.]+)"?\s+elapsed_ms=(?P<ms>\d+)\s+rss_mb=(?P<rss>\d+)'
    r'\s+rss_delta_mb=(?P<delta>\d+)\s+peak_rss_mb=(?P<peak>\d+)'
    r'(?:\s+facts=(?P<facts>\d+)\s+keys=(?P<keys>\d+)\s+key_mb=(?P<key_mb>\d+))?'
)
SAMPLER_SUMMARY = re.compile(r'^\s*\{.*"peak_rss_gb"')
BUDGET_DIAGNOSTIC = "polint/resource-budget"

REJECTED = 2
LEAKED = 3


def reject(message: str) -> int:
    print(f"report.py: {message}", file=sys.stderr)
    return REJECTED


def fold_stderr(path: str, strict: bool) -> tuple[list[dict], dict | None, int, list[str]]:
    """(stage rows, sampler summary, resource-budget mentions, dropped lines)."""
    rows: list[dict] = []
    summary: dict | None = None
    budget = 0
    dropped: list[str] = []
    with open(path, errors="replace") as handle:
        for line in handle:
            clean = ANSI.sub("", line).rstrip("\n")
            if not clean.strip():
                continue
            match = STAGE_ROW.search(clean)
            if match and "stage done" in clean:
                rows.append({key: value for key, value in match.groupdict().items()})
                continue
            if SAMPLER_SUMMARY.match(clean):
                try:
                    summary = json.loads(clean)
                except ValueError:
                    dropped.append(clean)
                continue
            if BUDGET_DIAGNOSTIC in clean:
                # The count only. The diagnostic's message text is consumer-facing.
                budget += 1
                continue
            dropped.append(clean)
    if strict and dropped:
        raise ValueError(
            f"{os.path.basename(path)}: {len(dropped)} lines are not stage rows, a sampler "
            f"summary or a resource-budget mention; first: {dropped[0][:80]!r}"
        )
    return rows, summary, budget, dropped


def cells(directory: str) -> list[str]:
    return sorted(
        name[: -len(".stderr")]
        for name in os.listdir(directory)
        if name.endswith(".stderr")
    )


def providers_with_rows(rows: list[dict]) -> int:
    return len({row["provider"] for row in rows})


def render(directory: str, host: str, polint: str, date: str, strict: bool) -> str:
    out: list[str] = []
    out.append(f"# Deep-capability gate run: {date}, {host}")
    out.append("")
    out.append(f"polint: {polint}; host: {host}; threads: 12")
    out.append("")
    out.append(
        "| Scope (file count) | cap | exit | wall s | tree peak MB (rssrun.py) | "
        "polint peak MB (stage row) | providers with rows | digest oracle | fact-row oracle |"
    )
    out.append("|---|---|---:|---:|---:|---:|---:|---|---|")

    folded: dict[str, tuple[list[dict], dict | None, int]] = {}
    for cell in cells(directory):
        rows, summary, budget, _ = fold_stderr(
            os.path.join(directory, f"{cell}.stderr"), strict
        )
        folded[cell] = (rows, summary, budget)
        wall = summary.get("wall_s") if summary else None
        code = summary.get("exit_code") if summary else None
        tree = (
            int(summary["peak_rss_bytes"]) // (1 << 20)
            if summary and "peak_rss_bytes" in summary
            else None
        )
        peak = max((int(row["peak"]) for row in rows), default=None)
        verdicts = read_verdict(directory, cell)
        out.append(
            f"| {cell} | {read_cap(directory, cell)} | {fmt(code)} | {fmt(wall)} | "
            f"{fmt(tree)} | {fmt(peak)} | {providers_with_rows(rows)} | "
            f"{verdicts.get('digest', '')} | {verdicts.get('factrows', '')} |"
        )

    for cell, (rows, _, budget) in folded.items():
        out.append("")
        out.append(f"## Stage rows ({cell})")
        out.append("")
        out.append("| provider | ms | rss MB | delta MB | peak MB | facts | keys | key MB |")
        out.append("|---|---:|---:|---:|---:|---:|---:|---:|")
        for row in rows:
            out.append(
                "| {provider} | {ms} | {rss} | {delta} | {peak} | {facts} | {keys} | {key_mb} |".format(
                    **{key: (value if value is not None else "") for key, value in row.items()}
                )
            )
        if budget:
            out.append("")
            out.append(f"Resource-budget diagnostics ({BUDGET_DIAGNOSTIC}): {budget}")

    out.append("")
    out.append("## Gate verdicts")
    out.append("")
    out.append("| Gate | pass/fail | measured | threshold |")
    out.append("|---|---|---|---|")
    for cell in folded:
        verdicts = read_verdict(directory, cell)
        verdict = verdicts.get("verdict")
        if verdict is None:
            verdict = "pass" if read_probe_exit(directory, cell) == 0 else "fail"
        out.append(f"| {cell} | {verdict} | see the row above | section 5.2 |")
    out.append("")
    return "\n".join(out)


def fmt(value) -> str:
    return "" if value is None else str(value)


def read_cap(directory: str, cell: str) -> str:
    """The capability, read off the sampler's recorded command line."""
    path = os.path.join(directory, f"{cell}.timeline.json")
    try:
        with open(path, encoding="utf-8") as handle:
            command = (json.load(handle).get("summary") or {}).get("cmd") or []
    except (OSError, ValueError):
        return ""
    for index, token in enumerate(command):
        if token == "--cap" and index + 1 < len(command):
            return command[index + 1]
    return ""


def read_probe_exit(directory: str, cell: str) -> int | None:
    path = os.path.join(directory, f"{cell}.probe")
    try:
        with open(path, errors="replace") as handle:
            for line in handle:
                if line.startswith("exit="):
                    return int(line[len("exit=") :].strip())
    except (OSError, ValueError):
        return None
    return None


def read_verdict(directory: str, cell: str) -> dict[str, str]:
    """`gate.sh`'s per-cell verdict file: `<key>: <value>` lines, or nothing.

    The gate's own stdout is a transcript for the operator. What the report folds
    is this file, so a report can be regenerated from a run directory alone.
    """
    verdicts: dict[str, str] = {}
    path = os.path.join(directory, f"{cell}.verdict")
    try:
        with open(path, errors="replace") as handle:
            for line in handle:
                if ":" in line:
                    key, value = line.split(":", 1)
                    verdicts[key.strip()] = value.strip()
    except OSError:
        pass
    return verdicts


def main() -> int:
    parser = argparse.ArgumentParser(add_help=True)
    parser.error = lambda message: sys.exit(reject(message))  # type: ignore[method-assign]
    parser.add_argument("directory")
    parser.add_argument("--host", required=True, help="a host label, never a hostname")
    parser.add_argument("--polint", required=True, help="<version> at <sha>")
    parser.add_argument("--date", default=None)
    parser.add_argument("--out", default=None)
    parser.add_argument(
        "--strict",
        action="store_true",
        help="fail instead of dropping a stderr line the allowlist does not recognise",
    )
    args = parser.parse_args()

    if not os.path.isdir(args.directory):
        return reject(f"{args.directory} is not a directory")
    date = args.date or datetime.date.today().isoformat()
    try:
        body = render(args.directory, args.host, args.polint, date, args.strict)
    except ValueError as error:
        print(f"report.py: {error}", file=sys.stderr)
        return LEAKED
    except OSError as error:
        return reject(str(error))

    if args.out:
        os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
        with open(args.out, "w", encoding="utf-8") as handle:
            handle.write(body + "\n")
        print(args.out)
    else:
        print(body)
    return 0


if __name__ == "__main__":
    sys.exit(main())
