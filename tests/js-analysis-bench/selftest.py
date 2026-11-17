"""Self-test of the report's tier measures over a synthetic package.

Run by `python3 tests/js-analysis-bench/bench.py selftest`. It needs no fetched
package, no polint build and no sandbox: it writes a three-call source file to a
temporary directory, describes it with hand-written base and refined dumps and
an observed run, and checks what the report counts and renders.

The cases are the ones the tier rules exist for:

- a site answered only by a `Heuristic` edge counts in the apart view and in no
  other, and is open;
- a typed-hierarchy edge that dispatches to an implementation counts as
  conservative, not exact, although its precision is `SetupAware`;
- a single exact target the run never called there is a wrong exact target;
- a budget event is reported with its reason;
- every tier section is rendered, and removing one fails the test.
"""

from __future__ import annotations

import sys
import tempfile
from pathlib import Path

import report

SOURCE = "function a() {}\nfunction b() {}\nfunction run(f) { f(); }\na();\nrun(b);\n"
FILE = "lib.js"
SECTIONS = (
    "## Tiers",
    "## Precision by tier",
    "## Open sites",
    "## Budget events",
    "## Refined edges",
    "## Sites gained by the refined plan",
)


def span(text: str, occurrence: int = 0) -> tuple[int, int]:
    start = -1
    for _ in range(occurrence + 1):
        start = SOURCE.index(text, start + 1)
    return start, start + len(text)


def key(text: str, occurrence: int = 0) -> str:
    start, end = span(text, occurrence)
    return f"{FILE}@{start}..{end}"


def function(name: str, text: str) -> dict:
    start, end = span(text)
    return {"key": key(text), "file": FILE, "name": name, "start": start, "end": end,
            "line": SOURCE[:start].count("\n") + 1, "module": False, "exported": False}


def site(text: str, occurrence: int = 0) -> dict:
    start, end = span(text, occurrence)
    return {"key": key(text, occurrence), "file": FILE, "start": start, "end": end}


def base_edge(at: str, target: str | None, precision: str) -> dict:
    return {"site": at, "function": target, "symbol": None, "precision": precision,
            "claim_tier": report.tier_of(precision), "algorithm": "Syntactic"}


def refined_edge(at: str, target: str, precision: str, algorithm: str, dispatch: str | None = None,
                 status: str = "Resolved", reason: str | None = None) -> dict:
    return {"site": at, "function": target, "symbol": None, "status": status, "reason": reason,
            "precision": precision, "claim_tier": report.tier_of(precision), "dispatch": dispatch,
            "algorithm": algorithm}


def dumps() -> tuple[dict, dict]:
    a = function("a", "function a() {}")
    b = function("b", "function b() {}")
    run = function("run", "function run(f) { f(); }")
    call_a, call_f, call_run = key("a()", 0), key("f()"), key("run(b)")
    common = {
        "files": [{"path": FILE, "bytes": len(SOURCE.encode())}],
        "functions": [a, b, run],
        "call_sites": [site("a()", 0), site("f()"), site("run(b)")],
        "references_by_file": {},
    }
    base = {
        **common,
        "base_edges": [
            base_edge(call_a, a["key"], "SetupAware"),
            # The walker's answer: a target it guessed, which claims nothing.
            base_edge(call_f, b["key"], "Heuristic"),
            base_edge(call_run, run["key"], "SetupAware"),
        ],
        "refined_edges": [],
        "unresolved_calls": [{"site": call_f, "reason": "DynamicCallee"}],
        "solver_budget": {"status": "WithinBudget", "reasons": []},
    }
    refined = {
        **common,
        "base_edges": base["base_edges"],
        "refined_edges": [
            # One exact target the run never called from this site.
            refined_edge(call_a, b["key"], "Exact", "Syntactic"),
            # A typed candidate: SetupAware precision, conservative claim.
            refined_edge(call_f, b["key"], "SetupAware", "TypeHierarchy", dispatch="implementation"),
            refined_edge(call_run, run["key"], "Exact", "Syntactic"),
        ],
        "unresolved_calls": [],
        "solver_budget": {"status": "BudgetExceeded", "reasons": ["points_to.max_dynamic_vars"]},
    }
    return base, refined


def observed() -> dict:
    a, b, run = (key(text) for text in ("function a() {}", "function b() {}", "function run(f) { f(); }"))
    edges = [
        (key("a()", 0), a, "a"),
        (key("f()"), b, "b"),
        (key("run(b)"), run, "run"),
    ]
    return {
        "instrumented_files": [FILE],
        "edges": [
            {"site": at, "callee": callee, "callee_name": name, "kind": "call", "syntax": "call"}
            for at, callee, name in edges
        ],
    }


def check(condition: bool, message: str, failures: list[str]) -> None:
    if not condition:
        failures.append(message)


def main() -> int:
    failures: list[str] = []
    with tempfile.TemporaryDirectory(prefix="polint-js-bench-selftest-") as root:
        (Path(root) / FILE).write_text(SOURCE)
        sources = report.Sources(Path(root))
        base_dump, refined_dump = dumps()
        plans = {"base": report.Facts(base_dump, "base"), "refined": report.Facts(refined_dump, "refined")}
        entry = {}
        executed = {}
        for plan, facts in plans.items():
            dump = facts.dump
            entry[plan] = {
                **report.score_package_calls(observed(), facts, None, sources),
                "solver_budget": dump["solver_budget"],
            }
            executed[plan] = set(entry[plan].pop("executed_sites"))
            if plan != "base":
                entry[plan]["refined_edge_census"] = report.refined_edge_census(dump)
                entry[plan]["sites_gained"] = report.sites_gained(plans["base"], facts, executed[plan])

    base, refined = entry["base"]["tiers"], entry["refined"]["tiers"]
    check(base["found"] == {"exact_complete": 2, "exact": 2, "exact_conservative": 2, "apart_only": 1},
          f"base tier recall: {base['found']}", failures)
    check(base["apart_edges"] == 1, f"base apart edges: {base['apart_edges']}", failures)
    check(base["sites"]["open_by_reason"] == {"DynamicCallee": 1},
          f"base open sites: {base['sites']['open_by_reason']}", failures)
    check(refined["found"] == {"exact_complete": 1, "exact": 1, "exact_conservative": 2, "apart_only": 0},
          f"refined tier recall: {refined['found']}", failures)
    check(refined["precision"]["exact"]["single_target"] == 1,
          f"refined wrong exact targets: {refined['precision']['exact']['single_target']}", failures)
    check(refined["precision"]["conservative"]["observed"] == 1,
          f"refined conservative precision: {refined['precision']['conservative']}", failures)
    gained = entry["refined"]["sites_gained"]
    check([(row["file"], row["sites"], row["executed"]) for row in gained] == [(FILE, 1, 1)],
          f"sites gained: {gained}", failures)

    packages = {"synthetic": entry}
    lines: list[str] = []
    report.render_tier_sections(packages, ["base", "refined"], lines.append)
    text = "\n".join(lines)
    for heading in SECTIONS:
        check(f"\n{heading}\n" in f"\n{text}\n", f"section missing: {heading}", failures)
    check("points_to.max_dynamic_vars" in text, "budget reason not rendered", failures)
    check(text.count("\n## ") + text.startswith("## ") == len(SECTIONS),
          "the tier sections and SECTIONS differ; update both together", failures)

    for failure in failures:
        print(f"selftest: {failure}", file=sys.stderr)
    print(f"selftest: {'FAILED' if failures else 'ok'} ({len(SECTIONS)} sections)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
