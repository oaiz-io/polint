"""Score an outside tool's call graph against the bench's observed calls.

The raw outputs are committed under `baseline/outside/<run>/<package>.json.gz`
by `bench.py outside`, so scoring needs no docker and no network. Each run's
`<package>.meta.json` beside it records how the tool was run.

An observed edge is matched to the tool's graph through the same `Spans`
lookups `report.py` uses for polint (`site_for`, `function_for`), and wrong
targets are counted by the same `unobserved_targets`, so the columns compare
edge for edge. Recall counts the observed edges of kinds `call` and
`callback`, the bench's headline set.

Jelly writes a position as `file:line:col:line:col`: 1-based lines and
1-based, end-exclusive columns in UTF-16 code units (Babel's 0-based column
plus one). `JellyPositions` converts them to the UTF-8 byte offsets every
bench key uses.
"""

from __future__ import annotations

import gzip
import json
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

BENCH_DIR = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(BENCH_DIR))
import bench  # noqa: E402
import report  # noqa: E402  (the matching rules, imported rather than copied)

OUTSIDE = bench.BASELINE / "outside"
JELLY_COMMIT = "62e60eb4fd5b991c5bf4cfc992f018984f560ea4"
JELLY_RUNS = {
    "jelly-static": [],
    "jelly-approx": ["--approx"],
}
RECALL_KINDS = ("call", "callback")
# Babel's line terminators; Jelly's line numbers count them.
LINE_BREAK = re.compile("\r\n|[\r\n\u2028\u2029]")
# Printed verbatim under `## Outside tools`: how these columns differ from
# polint's, so no reader takes them as the same measure.
DEVIATIONS = (
    "Jelly has no tiers and no notion of a complete answer or of a heuristic edge, so every "
    "Jelly edge counts as claimed, its edges to external callbacks included. Its wrong-target "
    "counts are an upper bound and are not comparable with polint's exact tier.",
    "Jelly's per-file module entries are left out of callee matching, as polint's `module` "
    "functions are.",
    "Jelly is given the same shipped source files polint analyses, as entry files; "
    "`-b <package>` makes its paths relative to the package root.",
    "Jelly models `_interopRequireDefault` and similar helpers natively instead of analysing "
    "their bodies; polint analyses the helper as code. On async this alone accounts for most "
    "of Jelly's misses.",
    "polint's outcome classes (\"stops at import binding\", \"marked unresolved\") have no "
    "counterpart in Jelly's output.",
    "Wall time is the docker wall time of one run, container start included, on the machine "
    "that recorded the committed outputs; it is not measured with polint's cost runs.",
    "ACG (js-callgraph) is not run: its parser rejects ES2020 and drops such files silently, "
    "so it cannot analyse these packages.",
)


class JellyPositions:
    """UTF-8 byte offsets of a file's Jelly positions."""

    def __init__(self, data: bytes):
        text = data.decode("utf-8")
        # (byte offset of the line start, the line without its terminator)
        self.lines: list[tuple[int, str]] = []
        offset = 0
        position = 0
        for match in LINE_BREAK.finditer(text):
            self.lines.append((offset, text[position : match.start()]))
            offset += len(text[position : match.end()].encode("utf-8"))
            position = match.end()
        self.lines.append((offset, text[position:]))

    def byte_offset(self, line: int, column: int) -> int:
        """The byte offset of 1-based `line` and 1-based UTF-16 `column`."""
        start, text = self.lines[line - 1]
        units = column - 1
        consumed = 0
        width = 0
        for char in text:
            if consumed >= units:
                break
            consumed += 2 if ord(char) > 0xFFFF else 1
            width += len(char.encode("utf-8"))
        return start + width


def load_jelly(document: dict, root: Path) -> tuple[report.Spans, dict[str, set[str]], dict]:
    """The tool's sites and functions, its site -> targets map, and its counts.

    Jelly writes every function first and then one entry per file for the
    module itself, so the last `len(files)` function indices are modules.
    """
    files = document["files"]
    positions: dict[str, JellyPositions | None] = {}

    def span(loc: str) -> tuple[str, int, int] | None:
        index, line1, col1, line2, col2 = loc.split(":")
        if line1 == "?":
            return None
        path = files[int(index)]
        if path not in positions:
            file = root / path
            positions[path] = JellyPositions(file.read_bytes()) if file.is_file() else None
        converted = positions[path]
        if converted is None:
            return None
        return path, converted.byte_offset(int(line1), int(col1)), converted.byte_offset(int(line2), int(col2))

    first_module = len(document["functions"]) - len(files)
    functions: dict[str, dict] = {}
    function_keys: dict[str, str] = {}
    for index, loc in document["functions"].items():
        located = span(loc)
        if located is None:
            continue
        path, start, end = located
        key = f"{path}@{start}..{end}"
        function_keys[index] = key
        functions.setdefault(
            key, {"key": key, "file": path, "start": start, "end": end, "module": int(index) >= first_module}
        )
    sites: dict[str, dict] = {}
    site_keys: dict[str, str] = {}
    for index, loc in document["calls"].items():
        located = span(loc)
        if located is None:
            continue
        path, start, end = located
        key = f"{path}@{start}..{end}"
        site_keys[index] = key
        sites.setdefault(key, {"key": key, "file": path, "start": start, "end": end})
    targets: dict[str, set[str]] = defaultdict(set)
    for call, function in document["call2fun"]:
        site, target = site_keys.get(str(call)), function_keys.get(str(function))
        if site is not None and target is not None:
            targets[site].add(target)
    counts = {
        "files": len(files),
        "functions": first_module,
        "calls": len(document["calls"]),
        "edges": sum(len(found) for found in targets.values()),
    }
    return report.Spans(list(sites.values()), list(functions.values())), targets, counts


def score(observed: dict, spans: report.Spans, targets: dict[str, set[str]]) -> dict:
    found = 0
    total = 0
    outcomes = Counter()
    observed_at: dict[str, set[str]] = defaultdict(set)
    for edge in observed["edges"]:
        if edge["kind"] not in RECALL_KINDS:
            continue
        total += 1
        site = spans.site_for(edge["site"])
        callee = spans.function_for(edge["callee"], edge["callee_name"])
        if site is None:
            outcome = "site not a tool call site"
        elif callee is None:
            outcome = "callee not a tool function"
        elif callee["key"] in targets.get(site["key"], ()):
            outcome = "found"
        elif targets.get(site["key"]):
            outcome = "other targets"
        else:
            outcome = "no target"
        found += outcome == "found"
        outcomes[outcome] += 1
        if site is not None and callee is not None:
            observed_at[site["key"]].add(callee["key"])
    wrong = Counter({"single_target": 0, "multi_target": 0})
    for _, _, shape in report.unobserved_targets(observed, observed_at, targets, spans):
        wrong[shape] += 1
    widths = sorted(len(found_targets) for found_targets in targets.values() if found_targets)
    return {
        "observed_edges": total,
        "found": found,
        "recall": report.ratio(found, total),
        "outcomes": dict(sorted(outcomes.items())),
        "executed_sites": len(observed_at),
        "wrong_targets": dict(sorted(wrong.items())),
        "targets_per_site": {
            "sites": len(widths),
            "mean": round(sum(widths) / len(widths), 2) if widths else None,
            "p95": report.nearest_rank(widths, 95),
            "max": widths[-1] if widths else None,
        },
    }


def raw_path(run: str, name: str) -> Path:
    return OUTSIDE / run / f"{name}.json.gz"


def meta_path(run: str, name: str) -> Path:
    return OUTSIDE / run / f"{name}.meta.json"


def score_package(run: str, package: dict) -> dict | None:
    """One run on one package, or None when no output is committed for it."""
    name = package["name"]
    raw, meta = raw_path(run, name), meta_path(run, name)
    if not raw.exists() or not meta.exists():
        return None
    observed = json.loads((bench.OBSERVED / "packages" / f"{name}.calls.json").read_text())
    spans, targets, counts = load_jelly(json.loads(gzip.decompress(raw.read_bytes())), bench.require_fetched(package))
    return {**score(observed, spans, targets), "tool": counts, "run": json.loads(meta.read_text())}


def score_all(inputs: dict) -> dict:
    """Every committed run on every package of the set, keyed run -> package."""
    results = {}
    for run in JELLY_RUNS:
        rows = {}
        for package in inputs["package"]:
            if package.get("held_out"):
                continue
            scored = score_package(run, package)
            if scored is not None:
                rows[package["name"]] = scored
        if rows:
            results[run] = rows
    return {"runs": results, "jelly_commit": JELLY_COMMIT, "deviations": list(DEVIATIONS)}


def render(outside: dict, packages: dict, plans: list[str], add) -> None:
    """The `## Outside tools` section: Jelly's columns beside polint's."""
    runs = outside["runs"]
    add("## Outside tools")
    add("")
    if not runs:
        add("No committed outside-tool output; run `bench.py outside`.")
        add("")
        return
    add(f"Jelly at `{outside['jelly_commit']}`, run by `bench.py outside` in the bench sandbox and scored")
    add("from the committed outputs under `baseline/outside/` with the bench's site and callee matching.")
    add("Recall is over observed `call` and `callback` edges. Wrong targets are claimed targets at executed")
    add("sites that the run never called there, split by answers with one target and with several.")
    add("")
    header = ["package"] + [f"polint {plan}" for plan in plans] + [f"{run}" for run in runs]
    add("| " + " | ".join(header) + " |")
    add("|" + "---|" * len(header))
    totals = Counter()
    for name in sorted({name for rows in runs.values() for name in rows}):
        cells = [name]
        for plan in plans:
            scored = packages.get(name, {}).get(plan) or {}
            cells.append(report.pct1(scored.get("recall")))
            totals[(plan, "found")] += scored.get("found", 0)
            totals[(plan, "observed")] += scored.get("observed_edges", 0)
        for run, rows in runs.items():
            row = rows.get(name)
            cells.append(report.pct1(row["recall"]) if row else "–")
            if row:
                totals[(run, "found")] += row["found"]
                totals[(run, "observed")] += row["observed_edges"]
        add("| " + " | ".join(cells) + " |")
    cells = ["**total**"]
    for column in [*plans, *runs]:
        cells.append(f"**{report.pct1(report.ratio(totals[(column, 'found')], totals[(column, 'observed')]))}**")
    add("| " + " | ".join(cells) + " |")
    add("")
    for run, rows in runs.items():
        add(f"### {run}")
        add("")
        add("| package | observed | found | recall | functions | calls | edges | targets per site, mean / p95 / max | wrong single | wrong multi | executed sites | wall s | exit |")
        add("|---|---|---|---|---|---|---|---|---|---|---|---|---|")
        for name, row in sorted(rows.items()):
            width = row["targets_per_site"]
            add(
                f"| {name} | {row['observed_edges']} | {row['found']} | {report.pct1(row['recall'])} | "
                f"{row['tool']['functions']} | {row['tool']['calls']} | {row['tool']['edges']} | "
                f"{width['mean']} / {width['p95']} / {width['max']} | {row['wrong_targets']['single_target']} | "
                f"{row['wrong_targets']['multi_target']} | {row['executed_sites']} | {row['run']['wall_s']} | "
                f"{row['run']['exit_code']} |"
            )
        add("")
    add("Deviations from polint's scoring:")
    add("")
    for deviation in outside["deviations"]:
        add(f"- {deviation}")
    add("")
