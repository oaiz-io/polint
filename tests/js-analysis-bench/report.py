"""The `report` step of the JS/TS analysis benchmark.

It stages every construct case and package outside any git checkout, reads
polint's facts through the internal fact dump (`eval::js_bench` in the polint
crate) in two capability plans, scores them against the hand labels and the
committed observed calls, measures the CLI's cost cold and warm, and writes a
JSON report and a Markdown summary.

Two plans are read:

  base     syntax, call_graph, symbols, references: what a rule that asks for
           `CallGraph`, `Symbols` and `References` gets. Its call edges are the
           rows the public `CallGraph` view serves.
  refined  adds calls, control_flow and dataflow, which turns on the refined
           call-edge layer. Refined edges have no public view; they are read
           through the same internal dump and shown in their own column.

Everything except the `cost` section is a pure function of the committed
inputs, the fetched packages and polint's analysis, so two runs on one commit
write the same scores.
"""

from __future__ import annotations

import json
import os
import platform
import re
import resource
import shutil
import statistics
import subprocess
import sys
import tempfile
import zlib
from collections import Counter, defaultdict
from pathlib import Path

import bench

sys.path.insert(0, str(bench.REPO_ROOT / "scripts" / "bench-runner"))
import bench_matrix  # noqa: E402  (the existing cost machinery: measure, directory_bytes)

REPORT_SCHEMA = "polint-js-analysis-bench-report-1"
DUMP_SCHEMA = "polint-js-bench-facts-1"
DUMP_FILTER = "eval::js_bench::tests::dump_js_bench_facts"
PLANS = {
    "base": "syntax,call_graph,symbols,references",
    "refined": "syntax,calls,control_flow,dataflow,call_graph,symbols,references",
}
RULES_TARGET_DIR = bench.REPO_ROOT / "target" / "js-analysis-bench-rules"
MB = 1_000_000
OBSERVED_KINDS = ("call", "callback", "implicit")
LABEL_RE = re.compile(r"//\s*(@\S.*)$")


# ---------------------------------------------------------------------------
# small helpers


def ratio(hit: int, total: int) -> float | None:
    return None if total == 0 else round(hit / total, 4)


def pct(value: float | None) -> str:
    return "n/a" if value is None else f"{value * 100:.0f}%"


def frac(hit: int, total: int) -> str:
    return "–" if total == 0 else f"{hit}/{total} ({pct(ratio(hit, total))})"


def line_starts(data: bytes) -> list[int]:
    starts = [0]
    for index, byte in enumerate(data):
        if byte == 0x0A:
            starts.append(index + 1)
    return starts


def line_of(starts: list[int], offset: int) -> int:
    low, high = 0, len(starts) - 1
    while low < high:
        mid = (low + high + 1) // 2
        if starts[mid] <= offset:
            low = mid
        else:
            high = mid - 1
    return low + 1


def parse_key(key: str) -> tuple[str, int, int]:
    """`path@start..end[#name]` -> (path, start, end)."""
    path, _, span = key.rpartition("@")
    span = span.split("#", 1)[0]
    start, _, end = span.partition("..")
    return path, int(start), int(end)


# ---------------------------------------------------------------------------
# building and running polint


def build(args) -> tuple[Path, Path]:
    """Build the release CLI and the release lib-test binary that holds the dump."""
    base = ["cargo", "build", "--release", "--locked", "-p", "polint", "--bin", "polint"]
    subprocess.run(base, cwd=bench.REPO_ROOT, check=True)
    completed = subprocess.run(
        [
            "cargo", "test", "-p", "polint", "--lib", "--all-features", "--locked",
            "--release", "--no-run", "--message-format=json",
        ],
        cwd=bench.REPO_ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    executable = None
    for line in completed.stdout.splitlines():
        message = json.loads(line)
        if (
            message.get("reason") == "compiler-artifact"
            and message["target"]["name"] == "polint"
            and message["profile"]["test"]
            and message.get("executable")
        ):
            executable = Path(message["executable"])
    if executable is None:
        raise SystemExit("could not find the polint lib-test binary")
    return bench.REPO_ROOT / "target" / "release" / "polint", executable


def stage(scratch: Path, inputs: dict, cases: list[dict]) -> tuple[dict, dict]:
    """Copy every analysed tree outside git; return {case id: tree}, {package: tree}."""
    case_trees = {}
    for case in cases:
        dest = scratch / "constructs" / case["id"]
        shutil.copytree(case["root"], dest)
        case_trees[case["id"]] = dest
    package_trees = {}
    for package in inputs["package"]:
        root = bench.require_fetched(package)
        analysed, _ = bench.package_files(package)
        dest = scratch / "packages" / package["name"]
        for rel in [*analysed, "package.json"]:
            (dest / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(root / rel, dest / rel)
        package_trees[package["name"]] = dest
    return case_trees, package_trees


def run_dump(test_binary: Path, tree: Path, plan: str, out: Path, timeout: int) -> dict:
    """Dump one tree's facts; return where the dump is, its wall clock and peak RSS.

    The dump is not loaded here: see `load_dump` and `run` for why."""
    manifest = out.with_suffix(".manifest")
    manifest.write_text(f"{tree}\t{out}\n")
    env = dict(os.environ)
    env["POLINT_JS_BENCH_MANIFEST"] = str(manifest)
    env["POLINT_JS_BENCH_CAPS"] = PLANS[plan]
    result = bench_matrix.measure(
        [str(test_binary), DUMP_FILTER, "--exact", "--ignored", "--test-threads", "1"],
        bench.REPO_ROOT,
        env,
        timeout,
    )
    cost = {"wall_ms": round(result.wall_ms), "peak_rss_bytes": result.peak_rss_bytes}
    if result.timed_out:
        return {"dump": None, "cost": cost, "error": f"timed out after {timeout} s"}
    if result.exit_code != 0 or not out.exists():
        return {"dump": None, "cost": cost, "error": f"dump failed: {result.stderr_tail}"}
    return {"dump": out, "cost": cost, "error": None}


def load_dump(run: dict) -> None:
    if run["dump"] is None:
        return
    dump = json.loads(run["dump"].read_text())
    if dump.get("schema") != DUMP_SCHEMA:
        raise SystemExit(f"unexpected dump schema in {run['dump']}")
    run["dump"] = dump


# ---------------------------------------------------------------------------
# facts: one plan's dump turned into lookups


class Facts:
    def __init__(self, dump: dict, plan: str):
        self.plan = plan
        self.dump = dump
        self.files = {row["path"] for row in dump["files"]}
        self.sites = {site["key"]: site for site in dump["call_sites"]}
        self.sites_by_end = defaultdict(list)
        for site in dump["call_sites"]:
            self.sites_by_end[(site["file"], site["end"])].append(site)
        self.functions = {function["key"]: function for function in dump["functions"]}
        self.functions_by_end = defaultdict(list)
        for function in dump["functions"]:
            self.functions_by_end[(function["file"], function["end"])].append(function)
        self.unresolved_sites = {row["site"] for row in dump["unresolved_calls"]}

        # Edges this plan's answer gives: site -> {target function key}.
        self.targets: dict[str, set[str]] = defaultdict(set)
        # Sites whose answer stops at an import binding (base edges only).
        self.import_stops: set[str] = set()
        # Sites this plan's answer itself marks as unresolved.
        self.marked_unresolved: set[str] = set()
        for edge in dump["base_edges"]:
            if edge["function"] is None and (edge["symbol"] or {}).get("kind") == "Import":
                self.import_stops.add(edge["site"])
        if plan == "base":
            for edge in dump["base_edges"]:
                if edge["function"] is not None:
                    self.targets[edge["site"]].add(edge["function"])
            self.marked_unresolved = set(self.unresolved_sites)
        else:
            resolved_sites = set()
            for edge in dump["refined_edges"]:
                if edge["site"] is None:
                    continue
                if edge["function"] is not None:
                    self.targets[edge["site"]].add(edge["function"])
                if edge["status"] == "Resolved":
                    resolved_sites.add(edge["site"])
                else:
                    self.marked_unresolved.add(edge["site"])
            # The refined layer answers for every site it has a row for; sites it
            # has no row for keep the base layer's unresolved marker.
            refined_sites = {edge["site"] for edge in dump["refined_edges"]}
            self.marked_unresolved |= self.unresolved_sites - refined_sites

    def site_for(self, key: str) -> dict | None:
        """Exact span first, then the polint site in the same file ending at the same byte."""
        if key in self.sites:
            return self.sites[key]
        path, _, end = parse_key(key)
        candidates = self.sites_by_end.get((path, end), [])
        return min(candidates, key=lambda site: site["end"] - site["start"]) if candidates else None

    def function_for(self, key: str, name: str) -> dict | None:
        """The innermost polint function in the same file ending at the same byte.

        Polint represents a class constructor by the class, so an observed
        `constructor` maps to the innermost polint function enclosing it.
        """
        path, start, end = parse_key(key)
        candidates = [f for f in self.functions_by_end.get((path, end), []) if not f["module"]]
        if not candidates and name == "constructor":
            candidates = [
                f
                for f in self.dump["functions"]
                if f["file"] == path and not f["module"] and f["start"] <= start and end <= f["end"]
            ]
        return min(candidates, key=lambda f: f["end"] - f["start"]) if candidates else None

    def unresolved_call_share(self) -> dict:
        total = len(self.sites)
        without = sum(1 for key in self.sites if not self.targets.get(key))
        return {
            "call_sites": total,
            "marked_unresolved": len(self.marked_unresolved & set(self.sites)),
            "marked_unresolved_share": ratio(len(self.marked_unresolved & set(self.sites)), total),
            "without_target": without,
            "without_target_share": ratio(without, total),
        }

    def reference_share(self) -> dict:
        statuses = Counter()
        for counts in self.dump["references_by_file"].values():
            statuses.update(counts)
        total = sum(statuses.values())
        unresolved = total - statuses.get("Resolved", 0)
        return {
            "references": total,
            "by_status": dict(sorted(statuses.items())),
            "not_resolved": unresolved,
            "not_resolved_share": ratio(unresolved, total),
        }


def export_answers(dump: dict) -> dict:
    """Per file: exported names from `Symbols`, and from export facts."""
    symbols: dict[str, set[str]] = defaultdict(set)
    for row in dump["exported_symbols"]:
        if row["file"] is not None:
            symbols[row["file"]].add(row["name"])
    facts: dict[str, set[str]] = defaultdict(set)
    fact_status: Counter = Counter()
    for row in dump["exports"]:
        fact_status[f"{row['kind']}/{row['status']}"] += 1
        if row["file"] is None or row["kind"] in ("StarReexport", "CommonJsModuleExports"):
            continue
        facts[row["file"]].add(row["name"])
    return {"symbols": symbols, "facts": facts, "fact_status": dict(sorted(fact_status.items()))}


def score_exports(observed: dict, dump: dict, files: list[str] | None = None) -> dict:
    answers = export_answers(dump)
    result = {"export_fact_rows": answers["fact_status"]}
    observed_files = observed["files"]
    candidates = files if files is not None else observed_files
    for source in ("symbols", "facts"):
        # Files where neither side has a name say nothing about accuracy.
        names = sorted(
            path
            for path in candidates
            if observed_files.get(path, {}).get("names") or answers[source].get(path)
        )
        predicted_total = observed_total = hit = exact = 0
        for path in names:
            expected = set(observed_files[path]["names"]) if path in observed_files else set()
            predicted = answers[source].get(path, set())
            observed_total += len(expected)
            predicted_total += len(predicted)
            hit += len(expected & predicted)
            exact += expected == predicted
        result[source] = {
            "files": len(names),
            "files_exact": exact,
            "observed_names": observed_total,
            "predicted_names": predicted_total,
            "matched_names": hit,
            "recall": ratio(hit, observed_total),
            "precision": ratio(hit, predicted_total),
        }
    return result


# ---------------------------------------------------------------------------
# constructs: hand labels


def parse_labels(text: str) -> dict:
    labels = {"exports": None, "fn": {}, "calls": set(), "callbacks": set(), "unknown": set(), "external": set()}
    for lineno, line in enumerate(text.splitlines(), 1):
        match = LABEL_RE.search(line)
        if not match:
            continue
        directive = None
        for token in match.group(1).split():
            if token.startswith("@"):
                directive = token[1:]
                if directive == "exports":
                    if lineno != 1:
                        raise SystemExit("@exports must be on the first line")
                    labels["exports"] = []
                elif directive in ("unknown", "external"):
                    labels[directive].add(lineno)
                elif directive not in ("fn", "calls", "callback"):
                    raise SystemExit(f"unknown label @{directive}")
                continue
            if directive == "exports":
                labels["exports"].append(token)
            elif directive == "fn":
                if lineno in labels["fn"]:
                    raise SystemExit(f"two @fn labels on line {lineno}")
                labels["fn"][lineno] = token
            elif directive == "calls":
                labels["calls"].add((lineno, token))
            elif directive == "callback":
                labels["callbacks"].add((lineno, token))
            else:
                raise SystemExit(f"stray label token {token!r} on line {lineno}")
    return labels


def case_labels(case: dict) -> dict:
    files = {}
    names = Counter()
    for rel in case["files"]:
        data = (case["root"] / rel).read_bytes()
        labels = parse_labels(data.decode())
        labels["starts"] = line_starts(data)
        files[rel] = labels
        names.update(labels["fn"].values())
    duplicate = [name for name, count in names.items() if count > 1]
    if duplicate:
        raise SystemExit(f"{case['id']}: @fn labels are not unique: {duplicate}")
    known = set(names)
    for rel, labels in files.items():
        for _, callee in labels["calls"] | labels["callbacks"]:
            if callee not in known:
                raise SystemExit(f"{case['id']}/{rel}: label names unknown function {callee!r}")
    return files


def label_functions(labels: dict, facts: Facts) -> tuple[dict, list[str]]:
    """Map polint function keys to labels; return the mapping and labels polint has no function for."""
    mapping, missing = {}, []
    for rel, file_labels in labels.items():
        for line, name in file_labels["fn"].items():
            candidates = [
                f for f in facts.dump["functions"] if f["file"] == rel and f["line"] == line and not f["module"]
            ]
            if not candidates and name.endswith(".constructor"):
                # Polint represents a class constructor by the class itself.
                owner = name.removesuffix(".constructor")
                candidates = [f for f in facts.dump["functions"] if f["file"] == rel and f["name"] == owner]
            if not candidates:
                missing.append(name)
                continue
            mapping[min(candidates, key=lambda f: f["end"] - f["start"])["key"]] = name
    return mapping, sorted(missing)


def score_case(case: dict, labels: dict, facts: Facts) -> dict:
    names, missing_functions = label_functions(labels, facts)
    # (file, line, callee label) -> distinct predicted sites.
    predicted: dict[tuple[str, int, str], set[str]] = defaultdict(set)
    unlabelled_targets = 0
    for site_key, targets in facts.targets.items():
        site = facts.sites.get(site_key)
        if site is None:
            continue
        for target in targets:
            name = names.get(target)
            if name is None:
                unlabelled_targets += 1
                continue
            predicted[(site["file"], site["line"], name)].add(site_key)

    expected_calls = {(rel, line, name) for rel, fl in labels.items() for line, name in fl["calls"]}
    expected_callbacks = {(rel, line, name) for rel, fl in labels.items() for line, name in fl["callbacks"]}
    calls_hit = sorted(key for key in expected_calls if predicted.get(key))
    # One line can label the same callee as a direct call and as a callback; the
    # callback counts as found only when a second site on that line reaches it.
    callbacks_hit = sorted(
        key
        for key in expected_callbacks
        if len(predicted.get(key, ())) >= (2 if key in expected_calls else 1)
    )
    true_positive = sorted(key for key in predicted if key in expected_calls | expected_callbacks)
    false_positive = sorted(key for key in predicted if key not in expected_calls | expected_callbacks)

    unknown = []
    for rel, file_labels in labels.items():
        for line in sorted(file_labels["unknown"]):
            sites = [s["key"] for s in facts.dump["call_sites"] if s["file"] == rel and s["line"] == line]
            marked = any(key in facts.marked_unresolved for key in sites)
            resolved = sorted(
                names.get(target, parse_key(target)[0] + "#" + target.split("#", 1)[-1])
                for key in sites
                for target in facts.targets.get(key, ())
            )
            if not sites:
                verdict = "silent"
                why = "no call site on the line"
            elif resolved and not marked:
                verdict = "false-complete"
                why = f"resolved to {', '.join(resolved)} with no unresolved marker"
            elif marked:
                verdict = "reported-unknown"
                why = "marked unresolved" + (f"; also resolved to {', '.join(resolved)}" if resolved else "")
            else:
                verdict = "silent"
                why = "call site with neither a target nor an unresolved marker"
            unknown.append({"file": rel, "line": line, "verdict": verdict, "detail": why})

    import_stops = sorted(
        f"{facts.sites[key]['file']}:{facts.sites[key]['line']}" for key in facts.import_stops if key in facts.sites
    )
    return {
        "expected_calls": len(expected_calls),
        "calls_found": len(calls_hit),
        "calls_missed": [f"{f}:{line} -> {name}" for f, line, name in sorted(expected_calls - set(calls_hit))],
        "call_recall": ratio(len(calls_hit), len(expected_calls)),
        "expected_callbacks": len(expected_callbacks),
        "callbacks_found": len(callbacks_hit),
        "callbacks_missed": [
            f"{f}:{line} -> {name}" for f, line, name in sorted(expected_callbacks - set(callbacks_hit))
        ],
        "predicted_labelled_edges": len(predicted),
        "true_positive_edges": len(true_positive),
        "false_positive_edges": [f"{f}:{line} -> {name}" for f, line, name in false_positive],
        "precision": ratio(len(true_positive), len(predicted)),
        "edges_to_unlabelled_functions": unlabelled_targets,
        "labelled_functions_not_extracted": missing_functions,
        "unknown_lines": unknown,
        "import_binding_stops": import_stops if facts.plan == "base" else [],
        "unresolved_calls": facts.unresolved_call_share(),
        "references": facts.reference_share(),
    }


def label_observed_agreement(case: dict, labels: dict, observed: dict) -> dict:
    """Do the hand labels agree with the calls recorded while the case ran?"""
    by_line = {rel: {line: name for line, name in fl["fn"].items()} for rel, fl in labels.items()}
    seen = set()
    for edge in observed["edges"]:
        site_path, site_start, _ = parse_key(edge["site"])
        callee_path, callee_start, _ = parse_key(edge["callee"])
        if site_path not in labels or callee_path not in labels:
            continue
        line = line_of(labels[site_path]["starts"], site_start)
        name = by_line[callee_path].get(line_of(labels[callee_path]["starts"], callee_start))
        if name is not None:
            seen.add((site_path, line, name))
    expected = {(rel, line, name) for rel, fl in labels.items() for line, name in fl["calls"] | fl["callbacks"]}
    hidden = {(rel, line) for rel, fl in labels.items() for line in fl["unknown"]}
    unlabelled = sorted(key for key in seen - expected if (key[0], key[1]) not in hidden)
    unobserved = sorted(expected - seen)
    observed_hidden = sorted(key for key in seen if (key[0], key[1]) in hidden)
    return {
        "observed_not_labelled": [f"{f}:{line} -> {name}" for f, line, name in unlabelled],
        "labelled_not_observed": [f"{f}:{line} -> {name}" for f, line, name in unobserved],
        "observed_on_unknown_lines": [f"{f}:{line} -> {name}" for f, line, name in observed_hidden],
        "failures": observed["failures"],
    }


def score_constructs(cases: list[dict], dumps: dict) -> dict:
    results = {}
    for case in cases:
        labels = case_labels(case)
        observed_calls = json.loads((bench.OBSERVED / "constructs" / f"{case['id']}.calls.json").read_text())
        observed_exports = json.loads((bench.OBSERVED / "constructs" / f"{case['id']}.exports.json").read_text())
        expected_exports = {
            "files": {
                rel: {"names": sorted(fl["exports"] or [])} for rel, fl in labels.items() if fl["exports"] is not None
            }
        }
        entry = {
            "construct": case["construct"],
            "held_out": case["held_out"],
            "description": case["description"],
            "labels_vs_observed": label_observed_agreement(case, labels, observed_calls),
            "labelled_exports_match_observed": all(
                sorted(observed_exports["files"].get(rel, {}).get("names", [])) == value["names"]
                for rel, value in expected_exports["files"].items()
            ),
        }
        for plan in PLANS:
            run = dumps[("case", case["id"], plan)]
            if run["dump"] is None:
                entry[plan] = {"error": run["error"]}
                continue
            entry[plan] = score_case(case, labels, Facts(run["dump"], plan))
        if dumps[("case", case["id"], "base")]["dump"] is not None:
            entry["exports"] = score_exports(observed_exports, dumps[("case", case["id"], "base")]["dump"])
        results[case["id"]] = entry
    return results


def construct_summary(case_results: dict) -> dict:
    """Per construct: counts summed over all its cases, with held-out cases listed apart."""
    summary = {}
    for case_id, entry in sorted(case_results.items()):
        row = summary.setdefault(
            entry["construct"],
            {"cases": [], "held_out_cases": [], **{plan: Counter() for plan in PLANS}, "exports": Counter()},
        )
        row["held_out_cases" if entry["held_out"] else "cases"].append(case_id)
        for plan in PLANS:
            scored = entry[plan]
            if "error" in scored:
                row[plan]["errors"] += 1
                continue
            counts = row[plan]
            counts["expected_calls"] += scored["expected_calls"]
            counts["calls_found"] += scored["calls_found"]
            counts["expected_callbacks"] += scored["expected_callbacks"]
            counts["callbacks_found"] += scored["callbacks_found"]
            counts["predicted_labelled_edges"] += scored["predicted_labelled_edges"]
            counts["true_positive_edges"] += scored["true_positive_edges"]
            counts["import_binding_stops"] += len(scored["import_binding_stops"])
            counts["labelled_functions_not_extracted"] += len(scored["labelled_functions_not_extracted"])
            for unknown in scored["unknown_lines"]:
                counts[f"unknown_{unknown['verdict']}"] += 1
            counts["call_sites"] += scored["unresolved_calls"]["call_sites"]
            counts["marked_unresolved"] += scored["unresolved_calls"]["marked_unresolved"]
            counts["references"] += scored["references"]["references"]
            counts["references_not_resolved"] += scored["references"]["not_resolved"]
        exports = entry.get("exports", {}).get("symbols")
        if exports:
            row["exports"]["files"] += exports["files"]
            row["exports"]["files_exact"] += exports["files_exact"]
    return {
        construct: {
            "cases": row["cases"],
            "held_out_cases": row["held_out_cases"],
            **{plan: dict(sorted(row[plan].items())) for plan in PLANS},
            "exports_symbols": dict(sorted(row["exports"].items())),
        }
        for construct, row in summary.items()
    }


# ---------------------------------------------------------------------------
# packages: observed calls


def score_package_calls(observed: dict, facts: Facts) -> dict:
    by_kind = defaultdict(Counter)
    by_syntax = defaultdict(Counter)
    outcomes = Counter()
    for edge in observed["edges"]:
        kind, syntax = edge["kind"], edge["syntax"]
        if kind == "implicit":
            # Accessors, coercions and protocol calls have a source position but
            # no call expression, so no call graph can hold them as call sites.
            by_kind[kind]["observed"] += 1
            by_kind[kind]["found"] += 0  # keeps the key so the row shows 0 found
            continue
        site = facts.site_for(edge["site"])
        callee = facts.function_for(edge["callee"], edge["callee_name"])
        if parse_key(edge["site"])[0] not in facts.files or parse_key(edge["callee"])[0] not in facts.files:
            outcome = "site or callee in a file polint does not analyse"
        elif site is None:
            outcome = "site not a polint call site"
        elif callee is None:
            outcome = "callee not a polint function"
        elif callee["key"] in facts.targets.get(site["key"], ()):
            outcome = "found"
        elif facts.plan == "base" and site["key"] in facts.import_stops:
            outcome = "stops at import binding"
        elif facts.targets.get(site["key"]):
            outcome = (
                "other targets, marked unresolved"
                if site["key"] in facts.marked_unresolved
                else "other targets, no unresolved marker"
            )
        elif site["key"] in facts.marked_unresolved:
            outcome = "no target, marked unresolved"
        else:
            outcome = "no target, no unresolved marker"
        outcomes[outcome] += 1
        for bucket in (by_kind[kind], by_syntax[syntax]):
            bucket["observed"] += 1
            bucket["found"] += outcome == "found"
    found = outcomes["found"]
    total = sum(outcomes.values())
    return {
        "observed_edges": total,
        "observed_implicit_edges": by_kind.get("implicit", Counter())["observed"],
        "found": found,
        "recall": ratio(found, total),
        "outcomes": dict(sorted(outcomes.items())),
        "by_kind": {k: {**v, "recall": ratio(v["found"], v["observed"])} for k, v in sorted(by_kind.items())},
        "by_syntax": {k: {**v, "recall": ratio(v["found"], v["observed"])} for k, v in sorted(by_syntax.items())},
    }


def score_packages(inputs: dict, dumps: dict) -> dict:
    results = {}
    for package in inputs["package"]:
        name = package["name"]
        observed_calls = json.loads((bench.OBSERVED / "packages" / f"{name}.calls.json").read_text())
        observed_exports = json.loads((bench.OBSERVED / "packages" / f"{name}.exports.json").read_text())
        analysed, exported = bench.package_files(package)
        entry = {
            "files_selected": len(analysed),
            "version": package["version"],
            "format": package["format"],
            "licence": package["licence"],
            "origin": package["origin"],
            "sha256": package["sha256"],
            "observed": {
                "instrumented_files": len(observed_calls["instrumented_files"]),
                "instrumented_functions": observed_calls["instrumented_functions"],
                "transformed_not_instrumented": observed_calls["transformed_not_instrumented"],
                "edges_by_kind": dict(sorted(Counter(e["kind"] for e in observed_calls["edges"]).items())),
                "failures": len(observed_calls["failures"]),
            },
        }
        base_dump = dumps[("package", name, "base")]["dump"]
        if base_dump is not None:
            entry["files"] = len(base_dump["files"])
            entry["source_bytes"] = sum(row["bytes"] for row in base_dump["files"])
            entry["functions"] = len(base_dump["functions"])
            entry["exports"] = score_exports(observed_exports, base_dump, exported)
        for plan in PLANS:
            run = dumps[("package", name, plan)]
            if run["dump"] is None:
                entry[plan] = {"error": run["error"]}
                continue
            facts = Facts(run["dump"], plan)
            entry[plan] = {
                **score_package_calls(observed_calls, facts),
                "unresolved_calls": facts.unresolved_call_share(),
                "references": facts.reference_share(),
            }
        results[name] = entry
    return results


# ---------------------------------------------------------------------------
# cost


def gzip_bytes(root: Path) -> int:
    """Gzip size of every file under `root`, each compressed on its own.

    Streams each file: holding a large cache file in memory would raise this
    process's resident size, which every process it starts later inherits as
    the floor of its measured peak RSS."""
    total = 0
    for path in sorted(root.rglob("*")):
        if path.is_file() and not path.is_symlink():
            compressor = zlib.compressobj(6, zlib.DEFLATED, 16 + zlib.MAX_WBITS)
            with path.open("rb") as source:
                while chunk := source.read(1 << 20):
                    total += len(compressor.compress(chunk))
            total += len(compressor.flush())
    return total


def check_env(cost_path: Path) -> dict:
    env = dict(os.environ)
    env.update(
        {
            "POLINT_RULES_TARGET_DIR": str(RULES_TARGET_DIR),
            "POLINT_RULES_PROFILE": "release",
            "POLINT_CACHE_STORE": "off",
            "POLINT_GOLDEN_COST_PATH": str(cost_path),
        }
    )
    env.pop("POLINT_CACHE_DIR", None)
    return env


def write_rule_config(tree: Path) -> None:
    (tree / ".polint.toml").write_text(f'[rules]\npaths = ["{bench.RULE_PACK}"]\n')


CHECK_ARGS = ["check", "--format", "json", "--fail-on", "none", "--color", "never"]


def one_check(polint: Path, tree: Path, scratch: Path, timeout: int) -> dict:
    cost_path = scratch / "cost.json"
    cost_path.unlink(missing_ok=True)
    result = bench_matrix.measure([str(polint), *CHECK_ARGS], tree, check_env(cost_path), timeout)
    if result.exit_code != 0:
        raise SystemExit(f"polint check failed in {tree}: {result.stderr_tail}")
    record = json.loads(cost_path.read_text())
    return {
        "process_wall_ms": result.wall_ms,
        "process_peak_rss_bytes": result.peak_rss_bytes,
        "analysis_wall_ms": record["wall_clock_ms"],
        "analysis_peak_rss_bytes": record["peak_rss_bytes"],
    }


def measure_cost(polint: Path, trees: dict, scratch: Path, runs: int, timeout: int) -> dict:
    """Cold and warm `polint check` with the census rule pack, median of `runs`."""
    # Builds the rule host once, unmeasured.
    first = next(iter(trees.values()))
    write_rule_config(first)
    one_check(polint, first, scratch, timeout)
    shutil.rmtree(first / ".polint", ignore_errors=True)

    results = {}
    for name, tree in trees.items():
        write_rule_config(tree)
        samples = {"cold": [], "warm": []}
        stored = stored_gzip = 0
        for _ in range(runs):
            shutil.rmtree(tree / ".polint", ignore_errors=True)
            samples["cold"].append(one_check(polint, tree, scratch, timeout))
            stored = bench_matrix.directory_bytes(tree / ".polint" / "cache")
            stored_gzip = gzip_bytes(tree / ".polint" / "cache")
            samples["warm"].append(one_check(polint, tree, scratch, timeout))
        shutil.rmtree(tree / ".polint", ignore_errors=True)
        (tree / ".polint.toml").unlink()
        row = {"stored_bytes": stored, "stored_gzip_bytes": stored_gzip}
        for tier, values in samples.items():
            medians = {key: statistics.median(v[key] for v in values) for key in values[0]}
            row[tier] = {
                "process_wall_ms": round(medians["process_wall_ms"]),
                "analysis_wall_ms": round(medians["analysis_wall_ms"]),
                "process_peak_rss_bytes": int(medians["process_peak_rss_bytes"]),
                "analysis_peak_rss_bytes": int(medians["analysis_peak_rss_bytes"]),
            }
        results[name] = row
    return results


def add_per_mb(row: dict, source_bytes: int) -> None:
    megabytes = source_bytes / MB
    row["source_bytes"] = source_bytes
    row["stored_bytes_per_mb"] = round(row["stored_bytes"] / megabytes)
    row["stored_gzip_bytes_per_mb"] = round(row["stored_gzip_bytes"] / megabytes)
    for tier in ("cold", "warm"):
        row[tier]["process_wall_ms_per_mb"] = round(row[tier]["process_wall_ms"] / megabytes)
        row[tier]["analysis_wall_ms_per_mb"] = round(row[tier]["analysis_wall_ms"] / megabytes)


def machine(launcher_peak_rss_bytes: int) -> dict:
    head = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=bench.REPO_ROOT, capture_output=True, text=True
    ).stdout.strip()
    return {
        "platform": platform.platform(),
        "cpus": os.cpu_count(),
        "python": platform.python_version(),
        "git_head": head,
        "launcher_peak_rss_bytes": launcher_peak_rss_bytes,
    }


# ---------------------------------------------------------------------------
# rendering


def render_markdown(report: dict) -> str:
    out = []
    add = out.append
    add("# JS/TS analysis benchmark")
    add("")
    add(f"polint {report['polint_version']}. Written by `python3 tests/js-analysis-bench/bench.py report`;")
    add("see `tests/js-analysis-bench/README.md` for the measures, the decisions and their limits.")
    add("There is no overall accuracy number on purpose: read the rows.")
    add("")
    add("- **base** is what a rule gets from `CallGraph` (capabilities `syntax, call_graph, symbols, references`).")
    add("- **refined** is the refined call-edge layer (adds `calls, control_flow, dataflow`). It has no public view;")
    add("  it is read through the internal benchmark dump only.")
    add("- Precision is measured against hand labels only. **Observed calls give recall, never precision**:")
    add("  a run sees the paths it exercised, so an edge polint reports that the run did not take is not wrong.")
    add("")

    add("## Constructs (hand labels)")
    add("")
    add("Counts are summed over each construct's cases. `held out` cases must not be used to tune analysis;")
    add("compare their rows with the other cases to see whether a change generalises.")
    add("")
    add("| construct | cases (held out) | call recall base | call recall refined | callback recall base | callback recall refined | precision base | precision refined | hidden paths base: reported / false-complete / silent | hidden paths refined | stops at import binding (base) |")
    add("|---|---|---|---|---|---|---|---|---|---|---|")
    for construct, row in sorted(report["constructs"]["summary"].items()):
        base, refined = row["base"], row["refined"]

        def hidden(counts):
            total = sum(counts.get(f"unknown_{v}", 0) for v in ("reported-unknown", "false-complete", "silent"))
            if not total:
                return "–"
            return f"{counts.get('unknown_reported-unknown', 0)} / {counts.get('unknown_false-complete', 0)} / {counts.get('unknown_silent', 0)}"

        cases = len(row["cases"]) + len(row["held_out_cases"])
        add(
            f"| {construct} | {cases} ({len(row['held_out_cases'])}) "
            f"| {frac(base.get('calls_found', 0), base.get('expected_calls', 0))} "
            f"| {frac(refined.get('calls_found', 0), refined.get('expected_calls', 0))} "
            f"| {frac(base.get('callbacks_found', 0), base.get('expected_callbacks', 0))} "
            f"| {frac(refined.get('callbacks_found', 0), refined.get('expected_callbacks', 0))} "
            f"| {frac(base.get('true_positive_edges', 0), base.get('predicted_labelled_edges', 0))} "
            f"| {frac(refined.get('true_positive_edges', 0), refined.get('predicted_labelled_edges', 0))} "
            f"| {hidden(base)} | {hidden(refined)} | {base.get('import_binding_stops', 0)} |"
        )
    add("")
    fc = report["constructs"]["false_complete"]
    add(
        f"**False-complete count** (a labelled hidden path where polint gives a resolved answer with no "
        f"unresolved marker): base {fc['base']}, refined {fc['refined']}, over {fc['hidden_lines']} labelled hidden paths."
    )
    add("")

    add("### Per case")
    add("")
    add("| case | held out | calls base | calls refined | callbacks base | callbacks refined | precision base | precision refined | hidden paths base | hidden paths refined | unresolved call share base | labels = observed |")
    add("|---|---|---|---|---|---|---|---|---|---|---|---|")
    for case_id, entry in sorted(report["constructs"]["cases"].items()):
        base, refined = entry["base"], entry["refined"]
        if "error" in base or "error" in refined:
            add(f"| {case_id} | {'yes' if entry['held_out'] else 'no'} | error: {base.get('error') or refined.get('error')} |")
            continue
        agreement = entry["labels_vs_observed"]
        agrees = not agreement["observed_not_labelled"] and not agreement["labelled_not_observed"]
        add(
            f"| {case_id} | {'yes' if entry['held_out'] else 'no'} "
            f"| {frac(base['calls_found'], base['expected_calls'])} "
            f"| {frac(refined['calls_found'], refined['expected_calls'])} "
            f"| {frac(base['callbacks_found'], base['expected_callbacks'])} "
            f"| {frac(refined['callbacks_found'], refined['expected_callbacks'])} "
            f"| {frac(base['true_positive_edges'], base['predicted_labelled_edges'])} "
            f"| {frac(refined['true_positive_edges'], refined['predicted_labelled_edges'])} "
            f"| {', '.join(u['verdict'] for u in base['unknown_lines']) or '–'} "
            f"| {', '.join(u['verdict'] for u in refined['unknown_lines']) or '–'} "
            f"| {pct(base['unresolved_calls']['marked_unresolved_share'])} "
            f"| {'yes' if agrees else 'no'} |"
        )
    add("")

    add("## Packages (observed calls: recall only)")
    add("")
    add("Recall is over observed edges at call expressions (`call` and `callback`); `implicit` calls (getters,")
    add("coercions, iterator and thenable protocol) have no call expression and are listed by kind only.")
    add("")
    add("| package | format | files analysed / selected | source KB | observed edges | recall base | recall refined | stops at import binding (base) | not mappable to a polint site or function | unresolved call share base | call sites without target base / refined | unresolved reference share |")
    add("|---|---|---|---|---|---|---|---|---|---|---|---|")
    for name, entry in report["packages"].items():
        base, refined = entry.get("base", {}), entry.get("refined", {})

        def recall(scored):
            if "error" in scored:
                return scored["error"]
            return frac(scored["found"], scored["observed_edges"])

        unmappable = sum(
            base.get("outcomes", {}).get(key, 0)
            for key in (
                "site not a polint call site",
                "callee not a polint function",
                "site or callee in a file polint does not analyse",
            )
        )
        without = (
            f"{pct(base['unresolved_calls']['without_target_share'])} / "
            + (pct(refined["unresolved_calls"]["without_target_share"]) if "unresolved_calls" in refined else "n/a")
            if "unresolved_calls" in base
            else "n/a"
        )
        add(
            f"| {name} {entry['version']} | {entry['format']} | {entry.get('files', 'n/a')} / {entry['files_selected']} "
            f"| {entry.get('source_bytes', 0) / 1000:.0f} | {base.get('observed_edges', 'n/a')} "
            f"| {recall(base)} | {recall(refined)} "
            f"| {base.get('outcomes', {}).get('stops at import binding', 0)} | {unmappable} "
            f"| {pct(base.get('unresolved_calls', {}).get('marked_unresolved_share'))} | {without} "
            f"| {pct(base.get('references', {}).get('not_resolved_share'))} |"
        )
    add("")
    add("### Package recall by observed call kind and syntax")
    add("")
    add("| package | kind / syntax | observed | base | refined |")
    add("|---|---|---|---|---|")
    for name, entry in report["packages"].items():
        base, refined = entry.get("base", {}), entry.get("refined", {})
        for group in ("by_kind", "by_syntax"):
            for key, counts in base.get(group, {}).items():
                other = refined.get(group, {}).get(key)
                add(
                    f"| {name} | {group[3:]}: {key} | {counts['observed']} | {frac(counts['found'], counts['observed'])} "
                    f"| {frac(other['found'], other['observed']) if other else 'n/a'} |"
                )
    add("")
    add("### Why observed edges were missed (base / refined)")
    add("")
    outcomes = sorted(
        {key for entry in report["packages"].values() for plan in PLANS for key in entry.get(plan, {}).get("outcomes", {})}
    )
    add("| package | " + " | ".join(outcomes) + " |")
    add("|---|" + "---|" * len(outcomes))
    for name, entry in report["packages"].items():
        cells = []
        for key in outcomes:
            cells.append(
                " / ".join(
                    "n/a" if "error" in entry.get(plan, {}) else str(entry.get(plan, {}).get("outcomes", {}).get(key, 0))
                    for plan in PLANS
                )
            )
        add(f"| {name} | " + " | ".join(cells) + " |")
    add("")

    add("## Exports (against the loaded module's export keys)")
    add("")
    add("`Symbols` is the exported-symbol answer a rule can read; export facts are the module export rows.")
    add("Both are compared with the export keys the loaded module actually has.")
    add("")
    add("| tree | files exact (Symbols) | name recall (Symbols) | name precision (Symbols) | files exact (export facts) | name recall (export facts) | name precision (export facts) | export fact rows by kind/status |")
    add("|---|---|---|---|---|---|---|---|")
    rows = [(f"package {n}", e.get("exports")) for n, e in report["packages"].items()]
    rows += [(f"case {n}", e.get("exports")) for n, e in sorted(report["constructs"]["cases"].items())]
    for name, exports in rows:
        if not exports or not exports["symbols"]["files"] + exports["facts"]["files"]:
            continue
        s, f = exports["symbols"], exports["facts"]
        statuses = ", ".join(f"{k} {v}" for k, v in exports["export_fact_rows"].items()) or "–"
        add(
            f"| {name} | {s['files_exact']}/{s['files']} | {frac(s['matched_names'], s['observed_names'])} "
            f"| {frac(s['matched_names'], s['predicted_names'])} | {f['files_exact']}/{f['files']} "
            f"| {frac(f['matched_names'], f['observed_names'])} | {frac(f['matched_names'], f['predicted_names'])} | {statuses} |"
        )
    add("")

    add("## Cost")
    add("")
    cost = report.get("cost")
    if not cost:
        add("Not measured in this run (`--no-cost`).")
    else:
        add("The only part of this report that changes between runs. Machine: "
            f"{cost['machine']['platform']}, {cost['machine']['cpus']} CPUs, commit {cost['machine']['git_head'][:12]}.")
        add("")
        add(f"`polint check` (release) with the census rule pack, which asks for `Functions`, `CallGraph`, `Symbols`, "
            f"`References`; median of {cost['runs']} runs. `analysis` is the in-process cost record "
            "(`POLINT_GOLDEN_COST_PATH`), `process` is the whole CLI as `bench_matrix.measure` sees it. "
            "Cold has no `.polint/cache`; warm reruns over the cache the cold run left. Stored bytes are "
            "`.polint/cache` after a cold run, raw and gzip-compressed. Per MB means per 1,000,000 source bytes. "
            "Process peak RSS comes from `wait4`, which on Linux counts the launching Python process's resident size "
            "at fork, so no process figure here is below the launcher's peak of "
            f"{cost['machine']['launcher_peak_rss_bytes'] / MB:.0f} MB; the analysis column has no such floor.")
        add("")
        add("| tree | source KB | cold analysis ms (per MB) | cold process ms | warm analysis ms (per MB) | warm process ms | cold peak RSS MB (analysis / process) | warm peak RSS MB | stored bytes per MB (gzip) |")
        add("|---|---|---|---|---|---|---|---|---|")
        for name, row in cost["check"].items():
            cold, warm = row["cold"], row["warm"]
            add(
                f"| {name} | {row['source_bytes'] / 1000:.0f} "
                f"| {cold['analysis_wall_ms']} ({cold['analysis_wall_ms_per_mb']}) | {cold['process_wall_ms']} "
                f"| {warm['analysis_wall_ms']} ({warm['analysis_wall_ms_per_mb']}) | {warm['process_wall_ms']} "
                f"| {cold['analysis_peak_rss_bytes'] / MB:.0f} / {cold['process_peak_rss_bytes'] / MB:.0f} "
                f"| {warm['analysis_peak_rss_bytes'] / MB:.0f} / {warm['process_peak_rss_bytes'] / MB:.0f} "
                f"| {row['stored_bytes_per_mb']} ({row['stored_gzip_bytes_per_mb']}) |"
            )
        add("")
        add("Fact dump (no cache, one tree per process; peak RSS is the process figure, with the same floor): this is")
        add("the only way to time the refined plan, because no public view asks for it.")
        add("")
        add("| tree | base ms | base peak RSS MB | refined ms | refined peak RSS MB |")
        add("|---|---|---|---|---|")
        for name, row in cost["dump"].items():
            add(
                f"| {name} | {row['base']['wall_ms']} | {row['base']['peak_rss_bytes'] / MB:.0f} "
                f"| {row['refined']['wall_ms']}{' (' + row['refined']['error'] + ')' if row['refined'].get('error') else ''} "
                f"| {row['refined']['peak_rss_bytes'] / MB:.0f} |"
            )
    add("")
    add("## Decisions recorded by this run")
    add("")
    for decision in report["decisions"]:
        add(f"- {decision}")
    add("")
    return "\n".join(out)


DECISIONS = [
    "Construct edges are identified by (file, line of the call expression's start, callee label); labels cannot "
    "express how many sites on one line call the same callee, so precision is counted per (line, callee).",
    "When one line labels a callee both as a direct call and as a callback, the callback counts as found only "
    "when a second distinct call site on that line reaches it.",
    "Predicted edges to functions without an @fn label are counted apart and are neither true nor false positives.",
    "A labelled hidden path (@unknown) is 'reported-unknown' when a call site on that line carries an unresolved "
    "marker, 'false-complete' when it has resolved targets and no marker, and 'silent' when polint has no call site "
    "there or one with neither. For the refined column the refined layer's own status decides; sites it has no row "
    "for keep the base layer's marker.",
    "A base edge whose target is an import binding rather than a function is a miss, counted as 'stops at import "
    "binding'.",
    "Observed package edges are mapped to polint by exact call-expression span (falling back to the polint call "
    "site in the same file ending at the same byte) and by callee end byte (innermost polint function in the same "
    "file ending there).",
    "Observed edges whose site or callee has no polint counterpart stay in the denominator: recall is over every "
    "observed edge.",
    "Export accuracy compares exported names per file; star re-export rows and the CommonJS whole-value "
    "`module.exports` row carry no name and are left out of the export-fact names but counted by status.",
    "Refined edges are counted when they carry a target function, whatever their status.",
    "Polint models a class as one function spanning the class body and has no separate constructor function, so "
    "a `Class.constructor` label and an observed constructor callee map to the class function.",
    "Observed implicit edges (accessors, coercions, iterator protocol) have a position but no call expression; "
    "they are reported by kind but left out of the headline package recall, which is over calls and callbacks.",
    "Packages are analysed with polint's default discovery, so its default excludes (for example `**/vendor/**`) "
    "apply; observed edges touching such files count as 'site or callee in a file polint does not analyse'.",
    "Missing call sites and functions are not worked around: functions under a unary operator, functions declared "
    "inside try, catch or if blocks, and calls in exported variable initializers are reported as misses.",
    "Process peak RSS is measured with `wait4` like the matrix runner does. Linux carries the parent's resident "
    "size into the child's peak at fork, so every process is launched before the report loads any fact dump, and "
    "the launcher's own peak is recorded with the cost section. The in-process analysis peak has no such floor.",
    "Each fact dump has a wall-clock limit (`--dump-timeout`, 600 s by default); a tree whose dump exceeds it is "
    "reported as timed out for that plan and has no scores in that column.",
]


# ---------------------------------------------------------------------------
# entry


def run(args) -> int:
    inputs = bench.load_inputs()
    cases = bench.construct_cases()
    polint, test_binary = build(args)
    version = subprocess.run([str(polint), "--version"], capture_output=True, text=True, check=True).stdout.split()[-1]

    with tempfile.TemporaryDirectory(prefix="polint-js-bench-report-") as scratch_name:
        scratch = Path(scratch_name)
        case_trees, package_trees = stage(scratch, inputs, cases)
        (scratch / "dumps").mkdir()
        dumps = {}
        trees = [("case", case_id, tree) for case_id, tree in case_trees.items()]
        trees += [("package", name, tree) for name, tree in package_trees.items()]
        for kind, name, tree in trees:
            for plan in PLANS:
                out = scratch / "dumps" / f"{kind}-{name.replace('/', '__')}-{plan}.json"
                dumps[(kind, name, plan)] = run_dump(test_binary, tree, plan, out, args.dump_timeout)
                if dumps[(kind, name, plan)]["error"]:
                    print(f"{kind} {name} {plan}: {dumps[(kind, name, plan)]['error']}", file=sys.stderr)
            if kind == "package":
                print(f"dumped {name}", file=sys.stderr)

        # Every measured process is started before any dump is loaded. A child's
        # peak RSS from wait4 starts at this process's own resident size when it
        # forks, so this process must stay small while it measures.
        check_cost = None
        if not args.no_cost:
            constructs_tree = scratch / "cost-constructs"
            for case_id, tree in case_trees.items():
                shutil.copytree(tree, constructs_tree / case_id)
            cost_trees = {"constructs (all cases)": constructs_tree, **package_trees}
            check_cost = measure_cost(polint, cost_trees, scratch, args.cost_runs, args.dump_timeout)
        launcher_peak_rss_bytes = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * 1024
        for run_entry in dumps.values():
            load_dump(run_entry)

        case_results = score_constructs(cases, dumps)
        hidden_lines = sum(len(e["base"].get("unknown_lines", [])) for e in case_results.values())
        report = {
            "schema": REPORT_SCHEMA,
            "polint_version": version,
            "plans": {plan: caps.split(",") for plan, caps in PLANS.items()},
            "constructs": {
                "summary": construct_summary(case_results),
                "false_complete": {
                    "hidden_lines": hidden_lines,
                    **{
                        plan: sum(
                            1
                            for e in case_results.values()
                            for u in e[plan].get("unknown_lines", [])
                            if u["verdict"] == "false-complete"
                        )
                        for plan in PLANS
                    },
                },
                "cases": case_results,
            },
            "packages": score_packages(inputs, dumps),
            "decisions": DECISIONS,
            "cost": None,
        }

        if check_cost is not None:
            add_per_mb(
                check_cost["constructs (all cases)"],
                sum(
                    sum(row["bytes"] for row in dumps[("case", case_id, "base")]["dump"]["files"])
                    for case_id in case_trees
                ),
            )
            for name in package_trees:
                add_per_mb(check_cost[name], report["packages"][name].get("source_bytes", 0))
            report["cost"] = {
                "machine": machine(launcher_peak_rss_bytes),
                "runs": args.cost_runs,
                "check": check_cost,
                "dump": {
                    name: {plan: {**dumps[("package", name, plan)]["cost"], "error": dumps[("package", name, plan)]["error"]} for plan in PLANS}
                    for name in package_trees
                },
            }

    out_dir = Path(args.out) if args.out else bench.BASELINE
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "report.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    (out_dir / "report.md").write_text(render_markdown(report))
    print(f"wrote {out_dir / 'report.json'} and {out_dir / 'report.md'}", file=sys.stderr)
    return 0
