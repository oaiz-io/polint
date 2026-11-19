"""Self-test of the report's tier measures over a synthetic package.

Run by `python3 tests/js-analysis-bench/bench.py selftest`. It needs no fetched
package, no polint build and no sandbox: it writes a three-call source file to a
temporary directory, describes it with hand-written base and refined dumps and
an observed run, and checks what the report counts and renders.

The cases are the ones the tier rules exist for:

- a site answered only by a `Heuristic` edge counts in the apart view and in no
  other, and is open;
- a typed-hierarchy edge that dispatches to an implementation counts as
  conservative, and one that claims exact stops the report;
- a single exact target the run never called there is a wrong exact target;
- a budget event is reported with its reason;
- every tier section is rendered, and removing one fails the test;
- an outside tool's positions (1-based UTF-16 columns) convert to the same
  byte offsets on a line with non-ASCII and astral characters before a call;
- the outside scorer matches an observed edge to the same site and callee
  keys as polint's scorer, because both use `report.Spans`;
- driver coverage counts each census function once, leaves classes out, and
  flags a recording under the floor;
- the held-out gate leaves a flagged package out of recall and keeps its wrong
  exact targets, and a held-out score keeps no cause and no missed-edge example;
- a typed plan that finds fewer observed edges than refined in an exact or
  conservative view, on a tune or a held-out package, is reported;
- the authorship guard refuses analysis code and held-out files in one change,
  and accepts either alone and recorder or tune-set files that only share
  letters with a held-out name (`held_out_guard_test.py` runs it against a git
  base, where only held-out files the base already has count).
"""

from __future__ import annotations

import copy
import sys
import tempfile
from pathlib import Path

import held_out_guard
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
            # A typed candidate: conservative precision, conservative claim.
            refined_edge(call_f, b["key"], "Conservative", "TypeHierarchy", dispatch="implementation"),
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


# Two- and four-byte UTF-8 characters, one of them two UTF-16 units, before a call.
WIDE_SOURCE = "// caf\u00e9 \U0001d4b3\nfunction a() {}\nconst s = '\u00fc\U0001d4b3'; a();\n"


def utf16_position(text: str, offset: int) -> tuple[int, int]:
    """1-based line and 1-based UTF-16 column of character `offset`, as Jelly writes them."""
    line = text.count("\n", 0, offset) + 1
    line_start = text.rfind("\n", 0, offset) + 1
    units = len(text[line_start:offset].encode("utf-16-le")) // 2
    return line, units + 1


def jelly_loc(text: str, start: int, end: int) -> str:
    return "0:%d:%d:%d:%d" % (*utf16_position(text, start), *utf16_position(text, end))


def check_outside(failures: list[str]) -> None:
    sys.path.insert(0, str(Path(__file__).resolve().parent / "outside"))
    import score_outside

    data = WIDE_SOURCE.encode()
    positions = score_outside.JellyPositions(data)
    function_start = WIDE_SOURCE.index("function a() {}")
    call_start = WIDE_SOURCE.rindex("a()")
    for offset in (function_start, function_start + len("function a() {}"), call_start, call_start + len("a()")):
        got = positions.byte_offset(*utf16_position(WIDE_SOURCE, offset))
        want = len(WIDE_SOURCE[:offset].encode())
        check(got == want, f"outside position of character {offset}: byte {got}, expected {want}", failures)

    with tempfile.TemporaryDirectory(prefix="polint-js-bench-selftest-") as root:
        (Path(root) / FILE).write_text(SOURCE)
        base_dump, _ = dumps()
        facts = report.Facts(base_dump, "base")
        # The synthetic package as Jelly writes it: functions, then the module.
        located = {"function a() {}": 0, "function b() {}": 1, "function run(f) { f(); }": 2}
        document = {
            "files": [FILE],
            "functions": {
                **{str(index): jelly_loc(SOURCE, *span(text)) for text, index in located.items()},
                "3": jelly_loc(SOURCE, 0, len(SOURCE)),
            },
            "calls": {str(index): jelly_loc(SOURCE, *span(text)) for index, text in enumerate(("a()", "f()", "run(b)"))},
            # The edge to the module entry is not a wrong target: modules are not callees.
            "call2fun": [[0, 0], [0, 3], [1, 1], [2, 2]],
        }
        spans, targets, counts = score_outside.load_jelly(document, Path(root))
    check(counts["functions"] == 3, f"outside function count leaves the module out: {counts}", failures)
    for edge in observed()["edges"]:
        polint_site, tool_site = facts.site_for(edge["site"]), spans.site_for(edge["site"])
        polint_callee = facts.function_for(edge["callee"], edge["callee_name"])
        tool_callee = spans.function_for(edge["callee"], edge["callee_name"])
        check(polint_site is not None and tool_site is not None and polint_site["key"] == tool_site["key"],
              f"outside site match differs at {edge['site']}", failures)
        check(polint_callee is not None and tool_callee is not None and polint_callee["key"] == tool_callee["key"],
              f"outside callee match differs for {edge['callee']}", failures)
    scored = score_outside.score(observed(), spans, targets)
    check(scored["found"] == 3 and scored["wrong_targets"] == {"multi_target": 0, "single_target": 0},
          f"outside score of the synthetic package: {scored}", failures)


def census_row(text: str, kind: str) -> dict:
    start, end = span(text)
    return {"start": start, "end": end, "kind": kind, "context": None, "class": None}


def check_held_out(entry: dict, failures: list[str]) -> None:
    census = {
        "syntax_census": {
            FILE: {
                "functions": [
                    census_row("function a() {}", "declaration"),
                    census_row("function b() {}", "declaration"),
                    census_row("function run(f) { f(); }", "declaration"),
                    census_row("a();", "class"),
                ],
                "calls": [],
            }
        }
    }
    entered = [
        {"function": key("function a() {}"), "name": "a"},
        {"function": key("function a() {}"), "name": "a"},
        {"function": key("function run(f) { f(); }"), "name": "run"},
    ]
    row = report.driver_coverage({"entered_functions": entered, "instrumented_functions": 3}, census)
    check((row["census_functions"], row["entered_census_functions"], row["flagged"]) == (3, 2, False),
          f"driver coverage: {row}", failures)
    row = report.driver_coverage({"entered_functions": entered[:1], "instrumented_functions": 3}, census)
    check(row["flagged"], f"coverage under the floor not flagged: {row}", failures)

    scored = copy.deepcopy(entry["refined"])
    scored["outcome_examples"]["callee not reached"] = [{}]
    scored["outcome_examples"][report.FALSE_COMPLETE] = [{}]
    report.withhold_miss_attribution(scored)
    check("causes" not in scored and list(scored["outcome_examples"]) == [report.FALSE_COMPLETE],
          f"held-out score keeps miss attribution: {sorted(scored)} {sorted(scored['outcome_examples'])}", failures)

    counted = {**copy.deepcopy(entry), "coverage": {"flagged": False}}
    flagged = {**copy.deepcopy(entry), "coverage": {"flagged": True}}
    for plan in ("base", "refined"):
        flagged[plan]["tiers"]["found"] = {view: 0 for view, _ in report.TIER_VIEWS}
    gate = report.held_out_gate({"synthetic": entry}, {"counted": counted, "flagged": flagged}, ["base", "refined"])
    check(gate["counted"] == ["counted"] and gate["flagged"] == ["flagged"], f"gate sets: {gate}", failures)
    check(all(row["within"] for plan in gate["plans"].values() for row in plan["tiers"].values()),
          f"a flagged package moved held-out recall: {gate['plans']}", failures)
    check(gate["plans"]["refined"]["wrong_exact_targets"] == 2,
          f"held-out wrong exact targets: {gate['plans']['refined']['wrong_exact_targets']}", failures)

    package = {"version": "1.0.0", "format": "cjs", "files": 1, "files_selected": 1, "source_bytes": len(SOURCE),
               "coverage": report.driver_coverage({"entered_functions": entered, "instrumented_functions": 3}, census)}
    lines: list[str] = []
    report.render_held_out(
        {
            "plans": {"base": [], "refined": []},
            "held_out": {"synthetic": {**copy.deepcopy(entry), **package}},
            "held_out_gate": gate,
        },
        lines.append,
    )
    text = "\n".join(lines)
    for heading in ("## Held-out packages", "### Held-out gate", *(f"{heading} (held-out)" for heading in SECTIONS)):
        check(f"\n{heading}\n" in f"\n{text}\n", f"held-out section missing: {heading}", failures)


def check_guard(failures: list[str]) -> None:
    names = held_out_guard.held_out_names(Path(__file__).resolve().parent / "inputs.toml")
    check({"co", "pako", "acorn", "node-forge"} <= names, f"held-out names: {sorted(names)}", failures)
    analysis = "crates/polint/src/analysis_neutral/points_to/mod.rs"
    cases = [
        ([analysis, "tests/js-analysis-bench/observed/drivers/heldout/co.mjs"], True),
        (["crates/polint/src/ts/mir.rs", "tests/js-analysis-bench/observed/packages/node-forge.calls.json"], True),
        (["crates/polint/src/graph.rs", "tests/js-analysis-bench/observed/packages/acorn.exports.json"], True),
        ([analysis], False),
        (["tests/js-analysis-bench/observed/packages/pako.calls.json"], False),
        ([analysis, "tests/js-analysis-bench/observed/recorder.mjs"], False),
        ([analysis, "tests/js-analysis-bench/observed/packages/commander.calls.json"], False),
        ([analysis, "tests/js-analysis-bench/observed/constructs/computed-member.calls.json"], False),
        (["crates/polint-eval/src/harness/js_bench.rs", "tests/js-analysis-bench/observed/drivers/heldout/co.mjs"], False),
    ]
    for paths, refused in cases:
        analysis_paths, _ = held_out_guard.violations(paths, names)
        check(bool(analysis_paths) == refused, f"guard on {paths}: refused {bool(analysis_paths)}", failures)


def check_exports(failures: list[str]) -> None:
    """A file that fails to load is counted, and left out of both answers."""
    observed_exports = {"files": {
        "a.js": {"system": "cjs", "names": ["x", "y"]},
        "b.js": {"error": "jQuery is not defined"},
    }}
    dump = {
        "exported_symbols": [{"file": "a.js", "name": "x"}, {"file": "b.js", "name": "z"}],
        "exports": [{"file": "a.js", "name": "x", "kind": "Named", "status": "Resolved"}],
    }
    scored = report.score_exports(observed_exports, dump, ["a.js", "b.js"])
    check(scored["load_errors"] == 1, f"export load errors: {scored['load_errors']}", failures)
    check((scored["symbols"]["files"], scored["symbols"]["predicted_names"], scored["symbols"]["matched_names"]) == (1, 1, 1),
          f"a failed load is scored: {scored['symbols']}", failures)


def check_stage_times(failures: list[str]) -> None:
    """Stage lines are summed per provider, other log lines and colour codes ignored."""
    stderr = "\n".join([
        '2026-01-01T00:00:00Z  INFO polint::kernel::stage: stage done provider="polint.ts.syntax" elapsed_ms=7 rss_mb=30 peak_rss_mb=29 facts=1',
        '\x1b[2m2026-01-01T00:00:01Z\x1b[0m  INFO polint::kernel::stage: stage done provider="polint.semantic_graph" elapsed_ms=40 peak_rss_mb=90',
        '2026-01-01T00:00:02Z  INFO polint::kernel::stage: stage done provider="polint.semantic_graph" elapsed_ms=2 peak_rss_mb=95',
        '2026-01-01T00:00:03Z  INFO polint::kernel: phase: ts.syntax done',
        "thread 'main' panicked",
    ])
    stages = report.stage_times(stderr)
    expected = {
        "polint.ts.syntax": {"elapsed_ms": 7, "peak_rss_mb": 29},
        "polint.semantic_graph": {"elapsed_ms": 42, "peak_rss_mb": 95},
    }
    check(stages == expected, f"stage times: {stages}", failures)


def check_solver_solves(failures: list[str]) -> None:
    """Each `points_to solve` line is one row, constraint kinds split out, in log order."""
    stderr = "\n".join([
        '2026-01-01T00:00:00Z  INFO polint::kernel::stage: stage done provider="polint.solver" elapsed_ms=7',
        '\x1b[2m2026-01-01T00:00:01Z\x1b[0m  INFO polint::points_to: points_to solve pass="type_value_alias" '
        'constraints=copy:3,field_load:2 calls=0 variables=9 objects=4 slots=1 copy_edges=3 pops=12 largest_set=2 '
        'sets_over_64=0 sets_over_1024=0 initialize_ms=1 run_ms=2 result_ms=3 rss_delta_mb=0 peak_rss_mb=50 '
        'budget=WithinBudget',
        '2026-01-01T00:00:02Z  INFO polint::points_to: points_to solve pass="call_graph" constraints= calls=5 '
        'pops=1 budget=BudgetExceeded',
    ])
    solves = report.solver_solves(stderr)
    check([solve["pass"] for solve in solves] == ["type_value_alias", "call_graph"], f"solve passes: {solves}", failures)
    check(solves[0]["constraints"] == {"copy": 3, "field_load": 2}, f"constraint kinds: {solves[0]}", failures)
    check(solves[0]["pops"] == 12 and solves[0]["run_ms"] == 2, f"solve counts: {solves[0]}", failures)
    check(solves[1]["constraints"] == {} and solves[1]["calls"] == 5 and solves[1]["variables"] == 0,
          f"sparse solve line: {solves[1]}", failures)
    check(solves[1]["budget"] == "BudgetExceeded", f"budget: {solves[1]}", failures)


def check_sequence_callee(failures: list[str]) -> None:
    """A comma-sequence callee reads as its last operand, nested ones too; other
    parenthesised text is left alone."""
    cases = {"(0,f)": "f", "(0,(0,f))": "f", "(0,_m.default)": "_m.default", "(a)(b)": "(a)(b)", "f": "f"}
    for text, expected in cases.items():
        got = report.sequence_last_operand(text)
        check(got == expected, f"sequence operand of {text}: {got}", failures)


def check_stripped_span(failures: list[str]) -> None:
    """An observed TypeScript function that ends before polint's, as a stripped
    type assertion does, maps to the function with its start; a JavaScript one
    does not."""
    def row(file: str, start: int, end: int) -> dict:
        return {"key": f"{file}@{start}..{end}", "file": file, "start": start, "end": end, "module": False}
    spans = report.Spans([], [row("a.ts", 10, 40), row("a.ts", 10, 60), row("a.ts", 0, 90), row("a.js", 10, 40)])
    got = spans.function_for("a.ts@10..32", "f")
    check(got is not None and got["end"] == 40, f"stripped ts span: {got}", failures)
    got = spans.function_for("a.js@10..32", "f")
    check(got is None, f"js span with no end match: {got}", failures)


def check_types_never_lose(failures: list[str]) -> None:
    """The typed plan may move an observed edge between tiers only upward; a loss in any view fails the run."""

    def plans(refined: dict, typed: dict) -> dict:
        return {"refined": {"tiers": {"found": refined}}, report.TYPED_PLAN: {"tiers": {"found": typed}}}

    even = {"exact_complete": 2, "exact": 3, "exact_conservative": 4, "apart_only": 1}
    gained = {"exact_complete": 2, "exact": 3, "exact_conservative": 5, "apart_only": 0}
    demoted = {"exact_complete": 1, "exact": 2, "exact_conservative": 4, "apart_only": 1}
    check(report.types_never_lose({"tune": plans(even, gained)}, {"held": plans(even, even)}) == [],
          "a typed plan that only gains was reported as losing", failures)
    lost = report.types_never_lose({"tune": plans(even, even)}, {"held": plans(even, demoted)})
    check(lost == ["held exact_complete: refined 2, typed 1", "held exact: refined 3, typed 2"],
          f"a held-out loss in the exact views was not reported: {lost}", failures)
    deleted = {**even, "exact_conservative": 3}
    check(report.types_never_lose({"tune": plans(even, deleted)}, {}) == ["tune exact_conservative: refined 4, typed 3"],
          "a deleted edge was not reported", failures)


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

    for dispatch in sorted(report.TYPED_CANDIDATE_DISPATCH):
        candidate = refined_edge(key("f()"), key("function b() {}"), "SetupAware", "TypeHierarchy", dispatch=dispatch)
        try:
            report.edge_tier(candidate)
            check(False, f"a typed {dispatch} candidate claiming exact was scored", failures)
        except SystemExit:
            pass

    check_outside(failures)
    check_types_never_lose(failures)
    check_held_out(entry, failures)
    check_guard(failures)
    check_exports(failures)
    check_stage_times(failures)
    check_solver_solves(failures)
    check_sequence_callee(failures)
    check_stripped_span(failures)

    for failure in failures:
        print(f"selftest: {failure}", file=sys.stderr)
    print(f"selftest: {'FAILED' if failures else 'ok'} ({len(SECTIONS)} sections)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
