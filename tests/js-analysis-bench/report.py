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

import functools
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

REPORT_SCHEMA = "polint-js-analysis-bench-report-2"
DUMP_SCHEMA = "polint-js-bench-facts-3"
DUMP_FILTER = "eval::js_bench::tests::dump_js_bench_facts"
PLANS = {
    "base": "syntax,call_graph,symbols,references",
    "refined": "syntax,calls,control_flow,dataflow,call_graph,symbols,references",
}
# The typed plan is the refined plan run with the TypeScript type sidecar on,
# over a copy of each tree that has a tsconfig and asks for the tier. It is added
# to `PLANS` only when a TypeScript compiler is available (see `run`), so every
# per-plan score and total covers it without a second code path.
TYPED_PLAN = "typed"
TYPED_TSCONFIG = {
    "compilerOptions": {
        "allowJs": True,
        "checkJs": False,
        "noEmit": True,
        "skipLibCheck": True,
        "target": "ES2022",
        "module": "nodenext",
        "moduleResolution": "nodenext",
        "jsx": "preserve",
    },
    "include": ["**/*"],
}
TYPED_SOURCE_SUFFIXES = {".js", ".mjs", ".cjs", ".jsx", ".ts", ".mts", ".cts", ".tsx"}
# Files the run records after their types are stripped.
TYPESCRIPT_SUFFIXES = {".ts", ".mts", ".cts", ".tsx"}
TYPED_CONFIG = "[languages.ts]\ntype_sidecar = true\n"
TYPESCRIPT_ENV = "POLINT_TS_TYPESCRIPT"
# The syntax census rides on the cheapest plan in its own run, so it never
# adds to the timed base and refined dumps.
CENSUS_CAPS = "syntax"
# Examples kept per outcome class, per package and plan. False-complete answers
# are always kept in full.
EXAMPLES_PER_OUTCOME = 5
EXAMPLES_PER_GAP = 3
FALSE_COMPLETE = "other targets, no unresolved marker"
# A held-out package whose recording entered fewer of its functions than this
# is reported and flagged, and left out of the held-out averages.
COVERAGE_FLOOR = 0.40
# The held-out gate: recall per tier within this many points of the tune set.
HELD_OUT_MARGIN_POINTS = 5
RULES_TARGET_SUBDIR = "js-analysis-bench-rules"
MB = 1_000_000
OBSERVED_KINDS = ("call", "callback", "implicit")
LABEL_RE = re.compile(r"//\s*(@\S.*)$")


# ---------------------------------------------------------------------------
# small helpers


def ratio(hit: int, total: int) -> float | None:
    return None if total == 0 else round(hit / total, 4)


def pct(value: float | None) -> str:
    return "n/a" if value is None else f"{value * 100:.0f}%"


def pct1(value: float | None) -> str:
    return "n/a" if value is None else f"{value * 100:.1f}%"


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
    """Build the release CLI and the release lib-test binary that holds the dump.

    Both paths come from cargo's own artifact messages, so a `CARGO_TARGET_DIR`
    or a configured target directory is honoured and a stale binary from
    another directory is never measured."""
    cli = cargo_executable(
        ["cargo", "build", "--release", "--locked", "-p", "polint", "--bin", "polint"],
        lambda message: "bin" in message["target"]["kind"] and not message["profile"]["test"],
        "the polint CLI binary",
    )
    test_binary = cargo_executable(
        ["cargo", "test", "-p", "polint", "--lib", "--all-features", "--locked", "--release", "--no-run"],
        lambda message: message["profile"]["test"],
        "the polint lib-test binary",
    )
    return cli, test_binary


def cargo_executable(argv: list[str], wanted, what: str) -> Path:
    """Run a cargo build and return the `executable` of the matching polint artifact."""
    completed = subprocess.run(
        [*argv, "--message-format=json"],
        cwd=bench.REPO_ROOT,
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    executable = None
    for line in completed.stdout.splitlines():
        message = json.loads(line)
        if (
            message.get("reason") == "compiler-artifact"
            and message["target"]["name"] == "polint"
            and message.get("executable")
            and wanted(message)
        ):
            executable = Path(message["executable"])
    if executable is None:
        raise SystemExit(f"could not find {what} in cargo's output")
    return executable


def stage(scratch: Path, inputs: dict, cases: list[dict]) -> tuple[dict, dict]:
    """Copy every analysed tree outside git; return {case id: tree}, {package: tree}.

    polint's file walk reads ignore files in every ancestor of a tree even
    outside a git checkout, so an ignore file above the scratch directory (a
    stray `.gitignore` in the system temp directory, say) could hide staged
    files. The `.ignore` written at the scratch root re-includes every path
    beneath it and takes precedence over any ancestor's rules; `load_dump`
    still rejects a dump that discovered no files."""
    (scratch / ".ignore").write_text("!*\n")
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


def stage_previous_sets(scratch: Path, inputs: dict) -> dict:
    """Stage each `compare_previous_set` package with its globs alone; return {package: tree}.

    Reports written before the shipped-source rule analysed these files; the
    comparison keeps one report readable against them."""
    trees = {}
    for package in inputs["package"]:
        if not package.get("compare_previous_set"):
            continue
        root = bench.require_fetched(package)
        analysed, _ = bench.package_files(package, shipped_only=False)
        dest = scratch / "previous-set" / package["name"]
        for rel in [*analysed, "package.json"]:
            (dest / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(root / rel, dest / rel)
        trees[package["name"]] = dest
    return trees


def stage_typed(scratch: Path, trees: dict) -> dict:
    """Copy each tree for the typed plan; return {key: typed tree}.

    The copy gets a tsconfig (unless the tree brings its own) so the sidecar
    has a project to load, and a `.polint.toml` that names the tier, so a
    sidecar that cannot start is a reported setup failure, not a silent skip.
    The plain trees stay untouched: the base and refined plans and the cost
    runs must not see either file."""
    typed = {}
    for key, tree in trees.items():
        dest = scratch / "typed" / tree.relative_to(scratch)
        shutil.copytree(tree, dest)
        if not (dest / "tsconfig.json").exists():
            # TypeScript's wildcard `include` never matches `*.min.js`, so the
            # sources are also listed by name: a minified bundle is still input.
            sources = sorted(
                path.relative_to(dest).as_posix()
                for path in dest.rglob("*")
                if path.is_file() and path.suffix in TYPED_SOURCE_SUFFIXES
            )
            config = {**TYPED_TSCONFIG, "files": sources}
            (dest / "tsconfig.json").write_text(json.dumps(config, indent=2) + "\n")
        (dest / ".polint.toml").write_text(TYPED_CONFIG)
        typed[key] = dest
    return typed


def find_typescript(args) -> Path | None:
    """The TypeScript package directory the typed plan hands to the sidecar."""
    if args.no_types:
        return None
    configured = args.typescript or os.environ.get(TYPESCRIPT_ENV, "")
    if not configured.strip():
        return None
    path = Path(configured.strip()).resolve()
    if not (path / "package.json").is_file():
        raise SystemExit(f"--typescript {path}: no package.json; point it at a `typescript` package directory")
    return path


def run_dump(
    test_binary: Path, tree: Path, plan: str, out: Path, timeout: int, typescript: Path | None = None
) -> dict:
    """Dump one tree's facts; return where the dump is, its wall clock and peak RSS.

    `plan` is a key of `PLANS`, or `census` for the syntax census run. The dump
    is not loaded here: see `load_dump` and `run` for why."""
    manifest = out.with_suffix(".manifest")
    manifest.write_text(f"{tree}\t{out}\n")
    env = dict(os.environ)
    env["POLINT_JS_BENCH_MANIFEST"] = str(manifest)
    if plan == "census":
        env["POLINT_JS_BENCH_CAPS"] = CENSUS_CAPS
        env["POLINT_JS_BENCH_CENSUS"] = "1"
    else:
        env["POLINT_JS_BENCH_CAPS"] = PLANS[plan]
        env.pop("POLINT_JS_BENCH_CENSUS", None)
    if plan == TYPED_PLAN:
        env[TYPESCRIPT_ENV] = str(typescript)
    else:
        # A compiler in the caller's environment must not reach the other plans.
        env.pop(TYPESCRIPT_ENV, None)
    env["RUST_LOG"] = STAGE_LOG
    env["NO_COLOR"] = "1"
    result = bench_matrix.measure(
        [str(test_binary), DUMP_FILTER, "--exact", "--ignored", "--test-threads", "1"],
        bench.REPO_ROOT,
        env,
        timeout,
    )
    cost = {
        "wall_ms": round(result.wall_ms),
        "peak_rss_bytes": result.peak_rss_bytes,
        "stages": stage_times(result.stderr),
    }
    if result.timed_out:
        return {"dump": None, "cost": cost, "error": f"timed out after {timeout} s"}
    if result.exit_code != 0 or not out.exists():
        tail = [line for line in result.stderr.splitlines() if "stage done" not in line][-8:]
        return {"dump": None, "cost": cost, "error": f"dump failed: {chr(10).join(tail)}"}
    return {"dump": out, "cost": cost, "error": None}


# The kernel's per-provider stage log: one `stage done` line per provider run.
STAGE_LOG = "polint::kernel::stage=info"
STAGE_FIELD = re.compile(r'\b(provider|elapsed_ms|peak_rss_mb)=("[^"]*"|\S+)')
ANSI_ESCAPE = re.compile(r"\x1b\[[0-9;]*m")


def stage_times(stderr: str) -> dict:
    """Each provider's summed stage wall time and the process peak RSS when it ended.

    A provider that runs more than once in a dump has its times added; the
    peak is the largest seen at the end of any of its runs."""
    stages: dict = {}
    for line in stderr.splitlines():
        line = ANSI_ESCAPE.sub("", line)
        if "stage done" not in line:
            continue
        fields = {key: value.strip('"') for key, value in STAGE_FIELD.findall(line)}
        if "provider" not in fields or "elapsed_ms" not in fields:
            continue
        row = stages.setdefault(fields["provider"], {"elapsed_ms": 0, "peak_rss_mb": 0})
        row["elapsed_ms"] += int(fields["elapsed_ms"])
        row["peak_rss_mb"] = max(row["peak_rss_mb"], int(fields.get("peak_rss_mb", 0)))
    return stages


def load_dump(run: dict) -> None:
    if run["dump"] is None:
        return
    dump = json.loads(run["dump"].read_text())
    if dump.get("schema") != DUMP_SCHEMA:
        raise SystemExit(f"unexpected dump schema in {run['dump']}")
    if not dump["files"]:
        # Every staged tree holds source files, so an empty dump means the
        # walk was hidden from them and its scores would be meaningless.
        run["dump"] = None
        run["error"] = "dump discovered no files"
        return
    run["dump"] = dump
    if run.get("typed") and not dump.get("ts_types", {}).get("callsites"):
        # The tier ran nowhere, so the dump is the refined plan again and its
        # scores would be read as what types add.
        reasons = "; ".join(dump.get("ts_types", {}).get("diagnostics", [])) or "no type rows"
        run["dump"] = None
        run["error"] = f"type tier did not run: {reasons}"


# ---------------------------------------------------------------------------
# tiers: what an edge claims


# Every `CallPrecision` and the tier it claims (research/package-graphs/TIERS.md
# (a)). The list is closed: a precision missing here stops the report, and the
# dump's own `claim_tier`, written by the engine, must agree with it.
PRECISION_TIERS = {
    "Exact": "exact",
    "SetupAware": "exact",
    "Conservative": "conservative",
    "Ambiguous": "conservative",
    "Heuristic": "apart",
    "Unknown": "none",
    "Unsupported": "none",
}
CLAIM_TIERS = ("exact", "conservative", "apart")
# Typed edges that name a candidate rather than the checker's answer: an
# implementation of a declared member, one constituent of a union receiver,
# or a declared target whose binding may hold another value. The engine
# tiers them conservative; one that claims exact is an engine defect the
# report stops on rather than scores.
TYPED_CANDIDATE_DISPATCH = {"implementation", "union_member", "inferred"}
# A refined row claims its target only with one of these statuses; an
# `Ambiguous` row names one of several candidates, so it claims no more than
# conservative.
CLAIMING_STATUS = {"Resolved": None, "Ambiguous": "conservative"}


def tier_of(precision: str) -> str:
    """The tier a precision claims; an unknown precision is an error."""
    try:
        return PRECISION_TIERS[precision]
    except KeyError:
        raise SystemExit(f"precision {precision!r} has no tier; add it to PRECISION_TIERS") from None


def edge_tier(edge: dict) -> str:
    """The tier the bench counts an edge in: its precision's tier, checked
    against the engine's, and never exact for a typed candidate."""
    tier = tier_of(edge["precision"])
    if edge.get("claim_tier") != tier:
        raise SystemExit(
            f"edge at {edge['site']}: the engine says {edge.get('claim_tier')!r} for {edge['precision']}, "
            f"the bench says {tier!r}"
        )
    if tier == "exact" and edge["algorithm"] == "TypeHierarchy" and edge.get("dispatch") in TYPED_CANDIDATE_DISPATCH:
        raise SystemExit(
            f"edge at {edge['site']}: a typed {edge.get('dispatch')} candidate claims {edge['precision']}"
        )
    return tier


def claimed_tier(edge: dict, refined: bool) -> str:
    """The tier an edge row claims for its target, `none` when it claims none."""
    tier = edge_tier(edge)
    if not refined or tier == "none":
        return tier
    if edge["status"] not in CLAIMING_STATUS:
        return "none"
    cap = CLAIMING_STATUS[edge["status"]]
    return cap if cap is not None and tier == "exact" else tier


def is_test_path(path: str) -> bool:
    return bench.is_not_shipped(path)


# ---------------------------------------------------------------------------
# facts: one plan's dump turned into lookups


class Spans:
    """Call sites and functions by span, and the bench's two matching rules.

    Every scorer of the bench matches an observed edge to a call graph through
    `site_for` and `function_for`: polint's facts and the outside tools' graphs
    alike, so their recall counts the same edges. A site row has `key`, `file`,
    `start` and `end` (UTF-8 bytes); a function row also has `module`.
    """

    def __init__(self, sites: list[dict], functions: list[dict]):
        self.sites = {site["key"]: site for site in sites}
        self.sites_by_end = defaultdict(list)
        for site in sites:
            self.sites_by_end[(site["file"], site["end"])].append(site)
        self.function_rows = functions
        self.functions = {function["key"]: function for function in functions}
        self.functions_by_end = defaultdict(list)
        for function in functions:
            self.functions_by_end[(function["file"], function["end"])].append(function)

    def site_for(self, key: str) -> dict | None:
        """Exact span first, then the site in the same file ending at the same byte."""
        if key in self.sites:
            return self.sites[key]
        path, _, end = parse_key(key)
        candidates = self.sites_by_end.get((path, end), [])
        return min(candidates, key=lambda site: site["end"] - site["start"]) if candidates else None

    def function_for(self, key: str, name: str) -> dict | None:
        """The function with the same span, else the innermost one in the same
        file ending at the same byte.

        The exact span comes first because a concise arrow returning another
        arrow, `(a) => (b) => ...`, ends at the same byte as the arrow it returns.
        Without an exact span, a candidate that starts at or before the observed
        start wins over one that starts inside it: the observed span of a
        generic arrow `<T>(a: T) => (b) => ...` starts after its type
        parameters, and the innermost function ending at the same byte is then
        the arrow it returns, not the one that was called.
        Polint represents a class constructor by the class, so an observed
        `constructor` maps to the innermost function enclosing it.
        The run records TypeScript after its types are stripped, so a concise
        arrow whose body ends in a type assertion, `(a) => a.b as any`, is
        observed ending before ` as any` while polint's span includes it. With
        no function ending at the observed byte, a function in a TypeScript
        file that starts at the observed start and ends nearest after the
        observed end is the one that was called.
        """
        path, start, end = parse_key(key)
        candidates = [f for f in self.functions_by_end.get((path, end), []) if not f["module"]]
        exact = [f for f in candidates if f["start"] == start]
        if exact:
            return exact[0]
        enclosing = [f for f in candidates if f["start"] <= start]
        if enclosing:
            return max(enclosing, key=lambda f: f["start"])
        if not candidates and Path(path).suffix in TYPESCRIPT_SUFFIXES:
            stripped = [
                f
                for f in self.function_rows
                if f["file"] == path and not f["module"] and f["start"] == start and f["end"] > end
            ]
            if stripped:
                return min(stripped, key=lambda f: f["end"])
        if not candidates and name == "constructor":
            candidates = [
                f
                for f in self.function_rows
                if f["file"] == path and not f["module"] and f["start"] <= start and end <= f["end"]
            ]
        return min(candidates, key=lambda f: f["end"] - f["start"]) if candidates else None


def unobserved_targets(observed: dict, observed_at: dict, claimed: dict, spans: Spans):
    """Yield `(site, target, shape)` for each claimed target at a site the run
    executed that the run never called from there.

    `observed_at` maps a site to the functions the run was seen calling from it,
    `claimed` a site to the targets a call graph claims there. A target counts
    when it is in an instrumented file, so the run would have recorded a call to
    it. `shape` is `single_target` when the answer at the site names one target
    and `multi_target` otherwise.
    """
    instrumented = set(observed["instrumented_files"])
    for site_key, called in sorted(observed_at.items()):
        targets = claimed.get(site_key, set())
        for target in sorted(targets - called):
            function = spans.functions.get(target)
            if function is None or function["module"] or function["file"] not in instrumented:
                continue
            yield site_key, target, "single_target" if len(targets) == 1 else "multi_target"


class Facts(Spans):
    def __init__(self, dump: dict, plan: str):
        super().__init__(dump["call_sites"], dump["functions"])
        self.plan = plan
        self.dump = dump
        self.files = {row["path"] for row in dump["files"]}
        self.unresolved_sites = {row["site"] for row in dump["unresolved_calls"]}

        # Edges this plan's answer gives: site -> {target function key}.
        self.targets: dict[str, set[str]] = defaultdict(set)
        # The subset of `targets` the answer claims: resolved and not heuristic.
        self.claimed_targets: dict[str, set[str]] = defaultdict(set)
        # Sites whose answer stops at an import binding (base edges only).
        self.import_stops: set[str] = set()
        # Sites this plan's answer itself marks as unresolved.
        self.marked_unresolved: set[str] = set()
        # Per site and tier, the targets the answer claims in that tier.
        self.tier_targets: dict[str, dict[str, set[str]]] = defaultdict(lambda: defaultdict(set))
        # Per site, the reasons its answer gives for being open.
        self.open_reasons: dict[str, set[str]] = defaultdict(set)
        for edge in dump["base_edges"]:
            if edge["function"] is None and (edge["symbol"] or {}).get("kind") == "Import":
                self.import_stops.add(edge["site"])
        if plan == "base":
            for edge in dump["base_edges"]:
                tier = claimed_tier(edge, refined=False)
                if edge["function"] is not None and tier != "none":
                    self.tier_targets[edge["site"]][tier].add(edge["function"])
            for row in dump["unresolved_calls"]:
                self.open_reasons[row["site"]].add(row["reason"])
        else:
            refined_sites = {edge["site"] for edge in dump["refined_edges"] if edge["site"] is not None}
            for edge in dump["refined_edges"]:
                if edge["site"] is None:
                    continue
                tier = claimed_tier(edge, refined=True)
                if edge["function"] is not None and tier != "none":
                    self.tier_targets[edge["site"]][tier].add(edge["function"])
                if edge["status"] != "Resolved":
                    self.open_reasons[edge["site"]].add(edge["reason"] or f"status {edge['status']}")
            for row in dump["unresolved_calls"]:
                if row["site"] not in refined_sites:
                    self.open_reasons[row["site"]].add(row["reason"])
        if plan == "base":
            for edge in dump["base_edges"]:
                if edge["function"] is not None:
                    self.targets[edge["site"]].add(edge["function"])
                    if edge["precision"] != "Heuristic":
                        self.claimed_targets[edge["site"]].add(edge["function"])
            self.marked_unresolved = set(self.unresolved_sites)
        else:
            resolved_sites = set()
            for edge in dump["refined_edges"]:
                if edge["site"] is None:
                    continue
                if edge["function"] is not None:
                    self.targets[edge["site"]].add(edge["function"])
                    if edge["status"] == "Resolved" and edge["precision"] != "Heuristic":
                        self.claimed_targets[edge["site"]].add(edge["function"])
                if edge["status"] == "Resolved":
                    resolved_sites.add(edge["site"])
                else:
                    self.marked_unresolved.add(edge["site"])
            # The refined layer answers for every site it has a row for; sites it
            # has no row for keep the base layer's unresolved marker.
            refined_sites = {edge["site"] for edge in dump["refined_edges"]}
            self.marked_unresolved |= self.unresolved_sites - refined_sites
        # A site is complete when it claims at least one exact or conservative
        # target and its answer has no unresolved row; every other site is open.
        self.complete_sites = {
            key
            for key in self.sites
            if self.claims(key, "exact") | self.claims(key, "conservative")
            and key not in self.marked_unresolved
        }

    def claims(self, site: str, tier: str) -> set[str]:
        """The targets the answer claims at `site` in `tier`."""
        tiers = self.tier_targets.get(site)
        return tiers.get(tier, set()) if tiers else set()

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
    candidates = files if files is not None else observed["files"]
    # A file that fails to load on its own has no observed names to compare
    # with; it is left out and counted, never scored as exporting nothing.
    failed = {path for path, entry in observed["files"].items() if "error" in entry}
    result["load_errors"] = len(failed & set(candidates))
    candidates = [path for path in candidates if path not in failed]
    observed_files = {path: entry for path, entry in observed["files"].items() if path not in failed}
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

    # A labelled call line answered with resolved targets that miss a labelled
    # callee, and with no unresolved marker on any call site of the line, claims
    # a complete answer that is not: the same false-complete answer as on an
    # @unknown line, for a callee polint could have named.
    incomplete = []
    for rel, file_labels in labels.items():
        wanted_by_line = defaultdict(set)
        for line, name in file_labels["calls"]:
            wanted_by_line[line].add(name)
        for line, wanted in sorted(wanted_by_line.items()):
            sites = [s["key"] for s in facts.dump["call_sites"] if s["file"] == rel and s["line"] == line]
            if any(key in facts.marked_unresolved for key in sites):
                continue
            resolved = {names.get(target) for key in sites for target in facts.targets.get(key, ())}
            missed = sorted(wanted - resolved)
            if missed and resolved - {None}:
                incomplete.append(
                    {
                        "file": rel,
                        "line": line,
                        "missed": missed,
                        "resolved": sorted(name for name in resolved if name is not None),
                    }
                )

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
        "labelled_call_lines": sum(len({line for line, _ in fl["calls"]}) for fl in labels.values()),
        "false_complete_call_lines": incomplete,
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
            counts["labelled_call_lines"] += scored["labelled_call_lines"]
            counts["false_complete_call_lines"] += len(scored["false_complete_call_lines"])
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


class Census:
    """One package's syntax census: Oxc's function and call spans with their context.

    A census function counts as extracted when polint has a function in the same
    file ending at the same byte, like `Facts.function_for` maps callees; a class
    constructor counts as extracted when its class is."""

    def __init__(self, dump: dict, facts: Facts):
        self.functions = defaultdict(list)
        self.calls = {}
        self.calls_by_end = defaultdict(list)
        for path, rows in dump["syntax_census"].items():
            self.functions[path] = rows["functions"]
            for row in rows["calls"]:
                self.calls[(path, row["start"], row["end"])] = row
                self.calls_by_end[(path, row["end"])].append(row)
        self.facts = facts

    def extracted(self, path: str, row: dict) -> bool:
        if row["kind"] == "constructor" and row["class"] is not None:
            return any(not f["module"] for f in self.facts.functions_by_end.get((path, row["class"][1]), []))
        return any(not f["module"] for f in self.facts.functions_by_end.get((path, row["end"]), []))

    def missing_around(self, path: str, start: int, end: int) -> list[dict]:
        """Census functions polint does not extract that contain the span, outermost first."""
        rows = [
            f
            for f in self.functions.get(path, [])
            if f["start"] <= start and end <= f["end"] and not self.extracted(path, f)
        ]
        return sorted(rows, key=lambda f: (f["start"] - f["end"], f["start"]))

    def around(self, path: str, start: int, end: int) -> list[dict]:
        """Census functions other than classes that contain the span, outermost first."""
        rows = [
            f
            for f in self.functions.get(path, [])
            if f["start"] <= start and end <= f["end"] and f["kind"] != "class"
        ]
        return sorted(rows, key=lambda f: (f["start"] - f["end"], f["start"]))

    def function_at(self, path: str, end: int) -> dict | None:
        rows = [f for f in self.functions.get(path, []) if f["end"] == end and f["kind"] != "class"]
        return min(rows, key=lambda f: f["end"] - f["start"]) if rows else None

    def call_at(self, path: str, start: int, end: int) -> dict | None:
        if (path, start, end) in self.calls:
            return self.calls[(path, start, end)]
        rows = self.calls_by_end.get((path, end), [])
        return min(rows, key=lambda row: row["end"] - row["start"]) if rows else None


class Sources:
    """Line numbers and short excerpts from a package's fetched sources."""

    def __init__(self, root: Path):
        self.root = root
        self.cache: dict[str, tuple[bytes, list[int]] | None] = {}

    def load(self, path: str):
        if path not in self.cache:
            file = self.root / path
            data = file.read_bytes() if file.is_file() else None
            self.cache[path] = None if data is None else (data, line_starts(data))
        return self.cache[path]

    def line(self, path: str, offset: int) -> int | None:
        loaded = self.load(path)
        return None if loaded is None else line_of(loaded[1], offset)

    def at(self, key: str) -> str:
        path, start, _ = parse_key(key)
        line = self.line(path, start)
        return f"{path}:{line}" if line is not None else path

    def excerpt(self, key: str, limit: int = 90) -> str:
        path, start, end = parse_key(key)
        loaded = self.load(path)
        if loaded is None:
            return ""
        text = " ".join(loaded[0][start:end].decode("utf-8", "replace").split())
        return text if len(text) <= limit else text[: limit - 1] + "…"


# Gaps a missed observed edge is attributed to. The exclusive attribution takes
# the first one that applies, in this order: extraction before resolution,
# because no resolution change finds an edge whose call site or callee polint
# does not extract.
GAPS = {
    "unary": "functions under a unary operator (unary IIFEs)",
    "block": "functions declared in try / catch / if blocks",
    "export": "calls in exported initializers",
    "constructor": "class constructors and `new`",
    "callback": "callbacks invoked by built-ins",
    "computed": "computed-key member calls",
    "class_method": "prototype and class methods, incl. inheritance",
}
REMAINDERS = {
    "not_analysed": "file not analysed (discovery excludes)",
    "other_extraction": "other call sites or functions not extracted",
    "import_stop": "identifier call stops at import binding",
    "other_member": "other member calls (object-literal functions, namespaces)",
    "other": "other resolution misses",
}
BLOCK_TAGS = {"try", "catch", "finally", "if"}
EXTRACTION_OUTCOMES = ("site not a polint call site", "callee not a polint function")


def extraction_gap(row: dict) -> str | None:
    """The extraction gap a census function or call's own context places it in.

    An exported declaration (`export function f`, `export class C`) carries the
    `export` tag too, but only an expression inside an exported initializer
    (`export const x = make(() => ...)`) is in that gap."""
    context = set(row["context"])
    if "unary" in context:
        return "unary"
    if context & BLOCK_TAGS:
        return "block"
    if "export" in context and not (row["kind"] in ("declaration", "class") and row["context"] == ["export"]):
        return "export"
    return None


def attribute_gap(edge: dict, outcome: str, census: Census) -> tuple[str, set[str], bool, str]:
    """(the one gap or remainder the miss is attributed to, every gap whose
    condition holds, whether a class-method miss goes through inheritance,
    the call syntax and the callee's census kind)."""
    site_path, site_start, site_end = parse_key(edge["site"])
    callee_path, callee_start, callee_end = parse_key(edge["callee"])
    site_call = census.call_at(site_path, site_start, site_end)
    callee_row = census.function_at(callee_path, callee_end)
    applies = set()
    missing = []
    # The functions around a call site polint does not have: when polint
    # extracts all of them, the outermost one in a gap decides, then the call's
    # own exported-initializer context.
    enclosing = []
    if outcome == "site not a polint call site":
        missing = census.missing_around(site_path, site_start, site_end)
        enclosing = census.around(site_path, site_start, site_end)
    elif outcome == "callee not a polint function":
        missing = census.missing_around(callee_path, callee_start, callee_end)
    for row in [*missing, *enclosing]:
        gap = extraction_gap(row)
        if gap:
            applies.add(gap)
        if row in missing and row["kind"] == "constructor":
            applies.add("constructor")
    if outcome == "site not a polint call site" and site_call and extraction_gap(site_call) == "export":
        applies.add("export")
    if edge["syntax"] == "new" or (callee_row is not None and callee_row["kind"] == "constructor"):
        applies.add("constructor")
    if edge["kind"] == "callback":
        applies.add("callback")
    if edge["syntax"] == "computed-member":
        applies.add("computed")
    class_method = callee_row is not None and (
        callee_row["kind"] in ("method", "accessor") or "prototype_assignment" in callee_row["context"]
    )
    if class_method and edge["syntax"] in ("member", "super", "computed-member"):
        applies.add("class_method")
    shape = f"{edge['syntax']} call to {callee_row['kind'] if callee_row else 'unknown'}"
    inherited = class_method and (
        edge["syntax"] == "super"
        or bool(site_call and site_call.get("this_receiver") and site_call["class"] != callee_row["class"])
    )

    if outcome == "site or callee in a file polint does not analyse":
        return "not_analysed", applies, inherited, shape
    if outcome in EXTRACTION_OUTCOMES:
        outermost = missing[0] if missing else None
        if outermost is not None:
            gap = extraction_gap(outermost) or ("constructor" if outermost["kind"] == "constructor" else None)
            return gap or "other_extraction", applies, inherited, shape
        for row in enclosing:
            if extraction_gap(row):
                return extraction_gap(row), applies, inherited, shape
        if outcome == "site not a polint call site" and site_call and extraction_gap(site_call) == "export":
            return "export", applies, inherited, shape
        return "other_extraction", applies, inherited, shape
    for gap in ("constructor", "callback", "computed", "class_method"):
        if gap in applies:
            return gap, applies, inherited, shape
    if outcome == "stops at import binding":
        return "import_stop", applies, inherited, shape
    if edge["syntax"] == "member":
        return "other_member", applies, inherited, shape
    return "other", applies, inherited, shape


# Why a missed edge was missed, read off the call's own source text: what the
# callee expression is and what its receiver is bound to. Unlike `GAPS`, which
# says which construct the callee is, a cause says what a resolver would have
# to follow to reach it. The reading is heuristic (a lexical scan of the call
# expression and its enclosing functions' parameter lists, not a parse), so a
# cause count is an estimate to rank work by, never an exact census.
CAUSES = {
    "not_analysed": "site or callee in a file polint does not analyse (vendor, node_modules)",
    "not_extracted": "site or callee not extracted by polint",
    "import_binding": "call stops at an import binding",
    "callback": "callback invoked by a built-in or library",
    "constructor": "`new` / constructor",
    "iife": "immediately invoked function",
    "this_member": "method on `this`",
    "super_member": "method on `super`",
    "computed_key": "dynamic (computed) key",
    "sequence_interop": "`(0, f)(...)` comma-sequence call",
    "call_apply_bind": "`.call` / `.apply` / `.bind`",
    "prototype_member": "member reached through `.prototype`",
    "parameter_member": "method on a parameter",
    "parameter_call": "call of a parameter (higher-order function)",
    "module_member": "member of an import / require binding (namespace object)",
    "object_literal_member": "member of a local object literal",
    "call_result_member": "method on a call result (fluent chain)",
    "local_member": "method on another local value",
    "local_call": "call of a local identifier",
    "other": "other",
}
IDENTIFIER_RE = re.compile(r"[A-Za-z_$][\w$]*")
MODULE_BINDING_RES = (
    r"import\s+\*\s+as\s+{name}\b",
    r"import\s+{name}\b",
    r"import\s*\{{[^}}]*\b{name}\b[^}}]*\}}",
    r"\b{name}\s*=\s*(?:__importStar\(|__importDefault\(|_interopRequire\w*\()?require\(",
    r"\b{name}\s*=\s*__webpack_require__\(",
    # Babel's two-step form: `var _m = require(...); var _m2 = _interopRequireDefault(_m);`.
    r"\b{name}\s*=\s*(?:__importStar|__importDefault|_interopRequire\w*)\(\s*[A-Za-z_$][\w$]*\s*\)",
)
KEYWORD_RECEIVERS = {"this", "super", "new", "typeof", "await", "void", "return"}


def callee_text(call: str) -> str:
    """The callee of a call expression's source text: the text before its last
    top-level argument list, without a trailing `?.` or type arguments."""
    text = call.strip()
    if text.startswith("new "):
        text = text[4:].lstrip()
    if not text.endswith(")"):
        return text
    depth = 0
    for index in range(len(text) - 1, -1, -1):
        char = text[index]
        if char == ")":
            depth += 1
        elif char == "(":
            depth -= 1
            if depth == 0:
                head = text[:index].rstrip()
                if head.endswith("?."):
                    head = head[:-2]
                if head.endswith(">") and "<" in head:
                    head = head[: head.rfind("<")]
                return head.rstrip()
    return text


def receiver_root(callee: str) -> str | None:
    """The identifier a member callee's receiver chain starts at."""
    match = IDENTIFIER_RE.match(callee.lstrip("(!"))
    return match.group(0) if match else None


def parameter_names(text: str) -> set[str]:
    """Names in a function's parameter list, read from its source text."""
    head = text.split("{", 1)[0] if "=>" not in text.split("{", 1)[0] else text.split("=>", 1)[0]
    if "(" in head:
        head = head[head.find("(") + 1 : head.rfind(")") if ")" in head else len(head)]
    names = set()
    for part in head.split(","):
        part = part.split("=", 1)[0]
        names.update(name for name in IDENTIFIER_RE.findall(part) if name not in ("async", "function"))
    return names


def sequence_last_operand(compact: str) -> str:
    """The last operand of a parenthesised comma sequence, unwrapped while the
    result is itself one: `(0,(0,f))` reads as `f`. Other text is returned as is."""
    while compact.startswith("(") and compact.endswith(")"):
        depth, last_comma = 0, None
        for index, char in enumerate(compact):
            if char in "([{":
                depth += 1
            elif char in ")]}":
                depth -= 1
                if depth == 0 and index != len(compact) - 1:
                    return compact
            elif char == "," and depth == 1:
                last_comma = index
        if last_comma is None:
            return compact
        compact = compact[last_comma + 1 : -1]
    return compact


def classify_cause(
    edge: dict,
    outcome: str,
    import_stop: bool,
    sources: Sources,
    census: "Census | None",
    site_kind: str | None = None,
) -> str:
    if outcome == "site or callee in a file polint does not analyse":
        return "not_analysed"
    if outcome in EXTRACTION_OUTCOMES:
        return "not_extracted"
    # Only the base outcome names an import stop; a refined site that stops at
    # the same binding is the same gap, so the cause reads the fact itself.
    if import_stop:
        return "import_binding"
    if edge["kind"] == "callback":
        return "callback"
    if edge["syntax"] == "new":
        return "constructor"
    path, start, end = parse_key(edge["site"])
    loaded = sources.load(path)
    if loaded is None:
        return "other"
    data = loaded[0]
    callee = callee_text(data[start:end].decode("utf-8", "replace"))
    compact = re.sub(r"\s+", "", callee)
    if compact.startswith(("function", "(function", "!function", "(()=>", "(async", "async(", "(()")):
        return "iife"
    if compact.startswith("(0,"):
        # A sequence callee polint reads through (its site has a callee kind)
        # is the gap of its last operand; only a site polint could not read is
        # a sequence gap.
        if site_kind in (None, "Unknown"):
            return "sequence_interop"
        compact = sequence_last_operand(compact)
    if compact.startswith("this."):
        return "this_member"
    if compact.startswith("super."):
        return "super_member"
    if re.search(r"\.(call|apply|bind)$", compact):
        return "call_apply_bind"
    if edge["syntax"] == "computed-member" or compact.endswith("]"):
        return "computed_key"
    if ".prototype." in compact:
        return "prototype_member"
    root = receiver_root(compact)
    parameters = set()
    if census is not None:
        for function in census.around(path, start, end):
            parameters |= parameter_names(data[function["start"] : function["end"]].decode("utf-8", "replace"))
    is_member = edge["syntax"] in ("member", "computed-member") or "." in compact
    if not is_member:
        return "parameter_call" if compact in parameters else "local_call"
    if root is None or root in KEYWORD_RECEIVERS:
        return "other"
    if root in parameters:
        return "parameter_member"
    text = data.decode("utf-8", "replace")
    escaped = re.escape(root)
    if any(re.search(pattern.format(name=escaped), text) for pattern in MODULE_BINDING_RES):
        return "module_member"
    receiver = compact[: compact.rfind(".")] if "." in compact else compact
    if "(" in receiver:
        return "call_result_member"
    if re.search(rf"\b(?:const|let|var)\s+{escaped}\s*=\s*\{{", text) or re.search(
        rf"\b{escaped}\s*=\s*\{{", text
    ):
        return "object_literal_member"
    return "local_member"


def describe_function(facts: Facts, key: str, sources: Sources) -> str:
    function = facts.functions.get(key)
    name = (function or {}).get("name") or "<anonymous>"
    return f"{sources.at(key)} {name}"


def describe_edge(edge: dict, outcome: str, facts: Facts, sources: Sources) -> dict:
    """One observed edge and what polint answered for it."""
    site = facts.site_for(edge["site"])
    callee = facts.function_for(edge["callee"], edge["callee_name"])
    example = {
        "site": sources.at(edge["site"]),
        "site_key": edge["site"],
        "site_text": sources.excerpt(edge["site"]),
        "kind": edge["kind"],
        "syntax": edge["syntax"],
        "observed_callee": f"{sources.at(edge['callee'])} {edge['callee_name'] or '<anonymous>'}",
        "polint_callee": callee["key"] if callee else None,
        "outcome": outcome,
        "polint_site": None,
    }
    if site is not None:
        targets = sorted(facts.targets.get(site["key"], ()))
        example["polint_site"] = {
            "key": site["key"],
            "match": "exact span" if site["key"] == edge["site"] else "same end byte",
            "text": sources.excerpt(site["key"]),
            "targets": [describe_function(facts, key, sources) for key in targets],
            "marked_unresolved": site["key"] in facts.marked_unresolved,
            "stops_at_import_binding": site["key"] in facts.import_stops,
        }
    return example


def polint_answer(example: dict) -> str:
    site = example["polint_site"]
    if site is None:
        return "no call site"
    if site["targets"]:
        answer = "targets " + "; ".join(site["targets"])
    elif site["stops_at_import_binding"]:
        answer = "import binding"
    else:
        answer = "no target"
    answer += ", marked unresolved" if site["marked_unresolved"] else ", no unresolved marker"
    if site["match"] != "exact span":
        answer += f" (site matched by {site['match']}: `{site['text']}`)"
    return answer


def unobserved_claimed_targets(observed: dict, observed_at: dict, facts: Facts, sources: Sources) -> dict:
    """Claimed targets at sites the run executed that the run never called there.

    A site counts once the run was seen calling a polint function from it. A
    claimed target (resolved, not heuristic) counts when it is in an
    instrumented file, so the run would have recorded a call to it, and the run
    never called it from that site. On a single-target answer that is a false
    precise edge: polint names one callee and every execution of the site went
    elsewhere. On a multi-target answer it is only an upper bound, because the
    run may not have taken the path that reaches the other targets.
    """
    counts = Counter({"single_target": 0, "multi_target": 0})
    examples = []
    for site_key, target, shape in unobserved_targets(observed, observed_at, facts.claimed_targets, facts):
        counts[shape] += 1
        if shape == "single_target" or len(examples) < EXAMPLES_PER_OUTCOME:
            examples.append(
                {
                    "answer": shape,
                    "site": sources.at(site_key),
                    "site_text": sources.excerpt(site_key),
                    "target": describe_function(facts, target, sources),
                    "observed_callees": [describe_function(facts, key, sources) for key in sorted(observed_at[site_key])],
                }
            )
    return {**dict(sorted(counts.items())), "examples": examples}


def tier_hits(facts: Facts, site: dict, callee: dict) -> list[str]:
    """The recall views an observed edge counts in, from the tier of its target."""
    key, target = site["key"], callee["key"]
    exact = target in facts.claims(key, "exact")
    either = exact or target in facts.claims(key, "conservative")
    hits = []
    if exact and key in facts.complete_sites:
        hits.append("exact_complete")
    if exact:
        hits.append("exact")
    if either:
        hits.append("exact_conservative")
    if not either and target in facts.claims(key, "apart"):
        hits.append("apart_only")
    return hits


def tier_precision(observed: dict, observed_at: dict, facts: Facts, sources: Sources) -> dict:
    """Per tier, the claimed targets at executed sites the run did and did not call there.

    The same rule as `unobserved_claimed_targets`, per tier: a target counts when
    it is a function in an instrumented file, at a site the run was seen calling
    a polint function from. `precision` is observed over claimed, a lower bound,
    because the run may not have taken the path that reaches a target it did not
    call. A target the run never called on a single-target answer in the exact
    tier is a wrong exact claim; every one is kept as an example."""
    instrumented = set(observed["instrumented_files"])
    result = {}
    for tier in CLAIM_TIERS:
        counts = Counter({"sites": 0, "claimed": 0, "observed": 0, "single_target": 0, "multi_target": 0})
        examples = []
        for site_key, called in sorted(observed_at.items()):
            claimed = facts.claims(site_key, tier)
            countable = sorted(
                target
                for target in claimed
                if (function := facts.functions.get(target)) is not None
                and not function["module"]
                and function["file"] in instrumented
            )
            if not countable:
                continue
            counts["sites"] += 1
            counts["claimed"] += len(countable)
            counts["observed"] += sum(target in called for target in countable)
            shape = "single_target" if len(claimed) == 1 else "multi_target"
            for target in countable:
                if target in called:
                    continue
                counts[shape] += 1
                if (tier == "exact" and shape == "single_target") or len(examples) < EXAMPLES_PER_OUTCOME:
                    examples.append(
                        {
                            "answer": shape,
                            "site": sources.at(site_key),
                            "site_text": sources.excerpt(site_key),
                            "target": describe_function(facts, target, sources),
                            "observed_callees": [describe_function(facts, key, sources) for key in sorted(called)],
                        }
                    )
        result[tier] = {
            **dict(sorted(counts.items())),
            "precision": ratio(counts["observed"], counts["claimed"]),
            "examples": examples,
        }
    return result


def nearest_rank(values: list[int], percent: int) -> int | None:
    """The nearest-rank percentile: the smallest value at or above `percent`% of them."""
    if not values:
        return None
    ordered = sorted(values)
    return ordered[max(0, -(-len(ordered) * percent // 100) - 1)]


def targets_per_site(facts: Facts) -> dict:
    """Targets per site that claims any, per tier: the width of each tier's answers."""
    result = {}
    views = {tier: (tier,) for tier in CLAIM_TIERS}
    views["exact_conservative"] = ("exact", "conservative")
    for view, tiers in views.items():
        widths = [
            len(set().union(*(facts.claims(key, tier) for tier in tiers)))
            for key in facts.sites
        ]
        widths = [width for width in widths if width]
        result[view] = {
            "sites": len(widths),
            "mean": round(sum(widths) / len(widths), 2) if widths else None,
            "p95": nearest_rank(widths, 95),
            "max": max(widths) if widths else None,
        }
    return result


def site_status(facts: Facts) -> dict:
    """Complete and open sites, and the open ones by the reason their answer gives.

    A site with several reasons counts under each; a site whose answer gives
    none is counted as `none recorded`."""
    open_sites = [key for key in facts.sites if key not in facts.complete_sites]
    reasons = Counter()
    for key in open_sites:
        given = facts.open_reasons.get(key) or {"none recorded"}
        reasons.update(given)
    return {
        "call_sites": len(facts.sites),
        "complete": len(facts.complete_sites),
        "open": len(open_sites),
        "open_with_a_claimed_target": sum(
            1 for key in open_sites if facts.claims(key, "exact") or facts.claims(key, "conservative")
        ),
        "open_by_reason": dict(sorted(reasons.items(), key=lambda item: (-item[1], item[0]))),
    }


def refined_edge_census(dump: dict) -> list[dict]:
    """Refined rows by (tier, algorithm, status, precision, has a function target)."""
    counts = Counter(
        (claimed_tier(edge, refined=True), edge["algorithm"], edge["status"], edge["precision"], edge["function"] is not None)
        for edge in dump["refined_edges"]
    )
    return [
        {"tier": tier, "algorithm": algorithm, "status": status, "precision": precision, "has_function": has_function, "rows": rows}
        for (tier, algorithm, status, precision, has_function), rows in sorted(counts.items(), key=lambda item: (-item[1], item[0]))
    ]


def apart_edges(dump: dict, plan: str) -> int:
    rows = dump["base_edges"] if plan == "base" else dump["refined_edges"]
    return sum(1 for edge in rows if edge["site"] is not None and edge_tier(edge) == "apart")


def score_package_calls(observed: dict, facts: Facts, census: Census | None, sources: Sources) -> dict:
    by_kind = defaultdict(Counter)
    by_syntax = defaultdict(Counter)
    outcomes = Counter()
    examples = defaultdict(list)
    causes = Counter()
    cause_examples = defaultdict(list)
    gaps_exclusive = Counter()
    gaps_applying = Counter()
    gap_examples = defaultdict(list)
    inherited = 0
    other_shapes = Counter()
    tier_found = Counter({view: 0 for view in ("exact_complete", "exact", "exact_conservative", "apart_only")})
    # Polint site -> the polint functions the run was seen calling from it.
    observed_at: dict[str, set[str]] = defaultdict(set)
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
                else FALSE_COMPLETE
            )
        elif site["key"] in facts.marked_unresolved:
            outcome = "no target, marked unresolved"
        else:
            outcome = "no target, no unresolved marker"
        if site is not None and callee is not None:
            observed_at[site["key"]].add(callee["key"])
            tier_found.update(tier_hits(facts, site, callee))
        outcomes[outcome] += 1
        for bucket in (by_kind[kind], by_syntax[syntax]):
            bucket["observed"] += 1
            bucket["found"] += outcome == "found"
        if outcome == FALSE_COMPLETE or len(examples[outcome]) < EXAMPLES_PER_OUTCOME:
            examples[outcome].append(describe_edge(edge, outcome, facts, sources))
        if outcome != "found":
            import_stop = site is not None and site["key"] in facts.import_stops
            cause = classify_cause(edge, outcome, import_stop, sources, census, site and site.get("kind"))
            causes[cause] += 1
            if len(cause_examples[cause]) < EXAMPLES_PER_GAP:
                cause_examples[cause].append(describe_edge(edge, outcome, facts, sources))
        if outcome != "found" and census is not None:
            gap, applying, through_inheritance, shape = attribute_gap(edge, outcome, census)
            gaps_exclusive[gap] += 1
            if gap == "other":
                other_shapes[shape] += 1
            gaps_applying.update(applying)
            inherited += gap == "class_method" and through_inheritance
            if len(gap_examples[gap]) < EXAMPLES_PER_GAP:
                gap_examples[gap].append(describe_edge(edge, outcome, facts, sources))
    wrong = unobserved_claimed_targets(observed, observed_at, facts, sources)
    found = outcomes["found"]
    total = sum(outcomes.values())
    result = {
        "observed_edges": total,
        "observed_implicit_edges": by_kind.get("implicit", Counter())["observed"],
        "found": found,
        "recall": ratio(found, total),
        "tiers": {
            "found": dict(sorted(tier_found.items())),
            "recall": {view: ratio(count, total) for view, count in sorted(tier_found.items())},
            "precision": tier_precision(observed, observed_at, facts, sources),
            "targets_per_site": targets_per_site(facts),
            "sites": site_status(facts),
            "apart_edges": apart_edges(facts.dump, facts.plan),
        },
        "executed_sites": sorted(observed_at),
        "outcomes": dict(sorted(outcomes.items())),
        "wrong_targets": wrong,
        "outcome_examples": dict(sorted(examples.items())),
        "causes": {"counts": dict(sorted(causes.items())), "examples": dict(sorted(cause_examples.items()))},
        "by_kind": {k: {**v, "recall": ratio(v["found"], v["observed"])} for k, v in sorted(by_kind.items())},
        "by_syntax": {k: {**v, "recall": ratio(v["found"], v["observed"])} for k, v in sorted(by_syntax.items())},
    }
    if census is not None:
        result["gaps"] = {
            "attributed": dict(sorted(gaps_exclusive.items())),
            "applying": dict(sorted(gaps_applying.items())),
            "class_method_through_inheritance": inherited,
            "other_by_syntax_and_callee": dict(sorted(other_shapes.items())),
            "examples": dict(sorted(gap_examples.items())),
        }
    return result


def driver_coverage(observed: dict, census_dump: dict | None) -> dict:
    """The functions a recording entered, over the functions in the analysed files.

    The denominator is the syntax census's functions other than classes and
    static blocks, which no call enters. An entered function counts once, as the
    innermost census function ending at the same byte, the rule
    `Census.function_at` and `Spans.function_for` match callees by."""
    row = {
        "entered_functions": len(observed["entered_functions"]),
        "instrumented_functions": observed["instrumented_functions"],
        "census_functions": None,
        "entered_census_functions": None,
        "coverage": None,
        "flagged": False,
    }
    if census_dump is None:
        return row
    by_end = defaultdict(list)
    total = 0
    for path, rows in census_dump["syntax_census"].items():
        for function in rows["functions"]:
            if function["kind"] in ("class", "static_block"):
                continue
            total += 1
            by_end[(path, function["end"])].append(function)
    entered = set()
    for function in observed["entered_functions"]:
        path, _, end = parse_key(function["function"])
        candidates = by_end.get((path, end), [])
        if candidates:
            innermost = min(candidates, key=lambda f: f["end"] - f["start"])
            entered.add((path, innermost["start"], innermost["end"]))
    row["census_functions"] = total
    row["entered_census_functions"] = len(entered)
    row["coverage"] = ratio(len(entered), total)
    row["flagged"] = row["coverage"] is not None and row["coverage"] < COVERAGE_FLOOR
    return row


def score_packages(inputs: dict, dumps: dict) -> dict:
    """Every package scored against its recording, the held-out ones included.

    A held-out package gets the same measures with no miss attribution: no gap,
    no cause and no example of a missed edge, so nothing in the report ranks
    work on it. Its wrong exact targets and false-complete answers keep their
    examples, because either is a defect to fix wherever it shows."""
    results = {}
    for package in inputs["package"]:
        name = package["name"]
        held_out = bool(package.get("held_out"))
        observed_calls = json.loads((bench.OBSERVED / "packages" / f"{name}.calls.json").read_text())
        observed_exports = json.loads((bench.OBSERVED / "packages" / f"{name}.exports.json").read_text())
        analysed, exported = bench.package_files(package)
        sources = Sources(bench.require_fetched(package))
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
        census_run = dumps.get(("package", name, "census"), {"dump": None, "error": "not run"})
        if census_run["dump"] is None:
            entry["census_error"] = census_run["error"]
        entry["held_out"] = held_out
        entry["coverage"] = driver_coverage(observed_calls, census_run["dump"])
        plan_facts = {}
        executed = {}
        for plan in PLANS:
            run = dumps[("package", name, plan)]
            if run["dump"] is None:
                entry[plan] = {"error": run["error"]}
                continue
            facts = Facts(run["dump"], plan)
            census = Census(census_run["dump"], facts) if census_run["dump"] is not None and not held_out else None
            entry[plan] = {
                **score_package_calls(observed_calls, facts, census, sources),
                "unresolved_calls": facts.unresolved_call_share(),
                "references": facts.reference_share(),
                "solver_budget": run["dump"]["solver_budget"],
            }
            if held_out:
                withhold_miss_attribution(entry[plan])
            executed[plan] = set(entry[plan].pop("executed_sites"))
            plan_facts[plan] = facts
            if plan != "base":
                entry[plan]["refined_edge_census"] = refined_edge_census(run["dump"])
            if plan == TYPED_PLAN:
                entry[plan]["ts_types"] = {
                    key: run["dump"]["ts_types"][key] for key in ("callsites", "callees")
                }
        if "base" in plan_facts:
            for plan, facts in plan_facts.items():
                if plan != "base":
                    entry[plan]["sites_gained"] = sites_gained(plan_facts["base"], facts, executed[plan])
        results[name] = entry
    return results


def withhold_miss_attribution(scored: dict) -> None:
    """Drop what ranks missed edges from a held-out package's score: causes and examples.

    The outcome counts stay, and so do the false-complete examples."""
    scored.pop("causes", None)
    scored["outcome_examples"] = {
        outcome: examples for outcome, examples in scored["outcome_examples"].items() if outcome == FALSE_COMPLETE
    }


def pooled_tier_recall(packages: dict, plan: str) -> dict:
    """Recall per tier view over every observed edge of `packages`, pooled as the Tiers totals are."""
    observed = 0
    found = Counter()
    for entry in packages.values():
        scored = entry.get(plan, {})
        if "tiers" not in scored:
            continue
        observed += scored["observed_edges"]
        found.update(scored["tiers"]["found"])
    return {
        "observed_edges": observed,
        **{view: ratio(found[view], observed) for view, _ in TIER_VIEWS},
    }


def held_out_gate(packages: dict, held_out: dict, plans: list[str]) -> dict:
    """The held-out gate per plan: recall per tier against the tune set, and the wrong answers.

    Flagged packages are left out of the recall comparison and kept in the wrong
    answers: a wrong exact target or a false-complete answer counts wherever it is."""
    counted = {name: entry for name, entry in held_out.items() if not entry["coverage"]["flagged"]}
    result = {
        "counted": sorted(counted),
        "flagged": sorted(name for name in held_out if name not in counted),
        "margin_points": HELD_OUT_MARGIN_POINTS,
        "plans": {},
    }
    for plan in plans:
        tune, held = pooled_tier_recall(packages, plan), pooled_tier_recall(counted, plan)
        views = {}
        for view, _ in TIER_VIEWS:
            if view == "apart_only":
                continue
            gap = None if tune[view] is None or held[view] is None else round((tune[view] - held[view]) * 100, 1)
            views[view] = {
                "tune": tune[view],
                "held_out": held[view],
                "points_below_tune": gap,
                "within": gap is not None and gap <= HELD_OUT_MARGIN_POINTS,
            }
        scored = [entry[plan] for entry in held_out.values() if "tiers" in entry.get(plan, {})]
        result["plans"][plan] = {
            "observed_edges": {"tune": tune["observed_edges"], "held_out": held["observed_edges"]},
            "tiers": views,
            "wrong_exact_targets": sum(s["tiers"]["precision"]["exact"]["single_target"] for s in scored),
            "false_complete": sum(s["outcomes"].get(FALSE_COMPLETE, 0) for s in scored),
        }
    return result


PREVIOUS_SET = "previous_set"


def previous_set_cost_key(name: str) -> str:
    """The `dump_cost` key of a package's dumps on its globs alone.

    Costs stay under `dump_cost`, so every other key of the report is the same
    from one run to the next."""
    return f"{name}, globs alone"


def set_summary(scored: dict) -> dict:
    """The rows of one plan's score the analysed-set comparison shows."""
    if "error" in scored:
        return {"error": scored["error"]}
    tiers = scored["tiers"]
    return {
        "observed_edges": scored["observed_edges"],
        "found": scored["found"],
        "recall": scored["recall"],
        "tiers_found": tiers["found"],
        "wrong_targets": {key: scored["wrong_targets"][key] for key in ("single_target", "multi_target")},
        "wrong_exact_targets": tiers["precision"]["exact"]["single_target"],
        "call_sites": tiers["sites"]["call_sites"],
    }


def score_previous_sets(inputs: dict, dumps: dict, packages: dict) -> dict:
    """Each `compare_previous_set` package scored on its previous and its shipped set.

    Kept apart from `packages`, so totals, the gap ranking and the cost runs
    see the shipped set only."""
    comparisons = {}
    for package in inputs["package"]:
        name = package["name"]
        if not package.get("compare_previous_set"):
            continue
        observed_calls = json.loads((bench.OBSERVED / "packages" / f"{name}.calls.json").read_text())
        sources = Sources(bench.require_fetched(package))
        sets = {}
        for label, kind in ((PREVIOUS_SET, PREVIOUS_SET), ("shipped", "package")):
            base_dump = dumps[(kind, name, "base")]["dump"]
            row = {
                "files": len(base_dump["files"]) if base_dump else None,
                "source_bytes": sum(file["bytes"] for file in base_dump["files"]) if base_dump else None,
            }
            for plan in PLANS:
                run_entry = dumps[(kind, name, plan)]
                if kind == "package":
                    scored = packages[name][plan]
                elif run_entry["dump"] is None:
                    scored = {"error": run_entry["error"]}
                else:
                    scored = score_package_calls(observed_calls, Facts(run_entry["dump"], plan), None, sources)
                row[plan] = set_summary(scored)
            sets[label] = row
        comparisons[name] = sets
    return comparisons


def sites_gained(base: Facts, refined: Facts, executed: set[str]) -> list[dict]:
    """Sites the refined plan claims a target at and the base plan does not, by file.

    `test_path` marks files under a test or benchmark folder, or named
    `*.test.*` / `*.spec.*`: a gain there is analysis of code the package does
    not ship. `executed` counts the gained sites the run was seen calling a
    polint function from, the only ones recall can credit."""
    def claims_any(facts: Facts, key: str) -> bool:
        return bool(facts.claims(key, "exact") or facts.claims(key, "conservative"))

    by_file = defaultdict(lambda: {"sites": 0, "executed": 0})
    for key, site in refined.sites.items():
        if claims_any(refined, key) and not claims_any(base, key):
            row = by_file[site["file"]]
            row["sites"] += 1
            row["executed"] += key in executed
    return [
        {"file": path, "test_path": is_test_path(path), **row}
        for path, row in sorted(by_file.items(), key=lambda item: (-item[1]["sites"], item[0]))
    ]


def rank_gaps(packages: dict) -> dict:
    """Missed observed edges per gap over every package, per plan, largest first.

    `attributed` counts each missed edge once, under the first gap that applies
    (see `GAPS`); `applying` counts every gap whose condition holds, so one edge
    can be in several. `recall_gain_upper_bound` is `attributed` over the plan's
    observed edges: what fixing the gap could add at most, if every edge it holds
    were then found. A plan's totals leave out packages whose dump for that plan
    failed, and say which."""
    ranking = {}
    for plan in PLANS:
        scored = {name: entry[plan] for name, entry in packages.items() if "gaps" in entry.get(plan, {})}
        observed = sum(entry["observed_edges"] for entry in scored.values())
        rows = []
        for gap, label in {**GAPS, **REMAINDERS}.items():
            per_package = {
                name: entry["gaps"]["attributed"].get(gap, 0)
                for name, entry in scored.items()
                if entry["gaps"]["attributed"].get(gap, 0)
            }
            attributed = sum(per_package.values())
            rows.append(
                {
                    "gap": gap,
                    "label": label,
                    "fixable_gap": gap in GAPS,
                    "attributed": attributed,
                    "applying": sum(entry["gaps"]["applying"].get(gap, 0) for entry in scored.values()),
                    "recall_gain_upper_bound": ratio(attributed, observed),
                    "by_package": dict(sorted(per_package.items(), key=lambda item: (-item[1], item[0]))),
                }
            )
        rows.sort(key=lambda row: (-row["attributed"], row["gap"]))
        ranking[plan] = {
            "packages": sorted(scored),
            "left_out": sorted(set(packages) - set(scored)),
            "observed_edges": observed,
            "missed_edges": sum(row["attributed"] for row in rows),
            "class_method_through_inheritance": sum(
                entry["gaps"]["class_method_through_inheritance"] for entry in scored.values()
            ),
            "other_by_syntax_and_callee": dict(
                sorted(
                    sum((Counter(entry["gaps"]["other_by_syntax_and_callee"]) for entry in scored.values()), Counter()).items(),
                    key=lambda item: (-item[1], item[0]),
                )
            ),
            "rows": rows,
        }
    return ranking


# ---------------------------------------------------------------------------
# cost


def typescript_version(typescript: Path | None) -> str | None:
    if typescript is None:
        return None
    return json.loads((typescript / "package.json").read_text()).get("version")


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


@functools.cache
def cargo_target_dir() -> Path:
    """The target directory cargo builds into for this workspace.

    Read from `cargo metadata`, so a `CARGO_TARGET_DIR` or a configured target
    directory is honoured the way `build` honours it."""
    completed = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"],
        cwd=bench.REPO_ROOT,
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    return Path(json.loads(completed.stdout)["target_directory"])


def check_env(cost_path: Path) -> dict:
    env = dict(os.environ)
    env.update(
        {
            "POLINT_RULES_TARGET_DIR": str(cargo_target_dir() / RULES_TARGET_SUBDIR),
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


def machine(launcher_peak_rss_bytes: int, load_at_start: tuple[float, float, float]) -> dict:
    head = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=bench.REPO_ROOT, capture_output=True, text=True
    ).stdout.strip()
    return {
        "platform": platform.platform(),
        "cpus": os.cpu_count(),
        "python": platform.python_version(),
        "git_head": head,
        "launcher_peak_rss_bytes": launcher_peak_rss_bytes,
        # One-, five- and fifteen-minute load averages when measuring started and ended.
        "load_average_start": [round(value, 2) for value in load_at_start],
        "load_average_end": [round(value, 2) for value in os.getloadavg()],
    }


# ---------------------------------------------------------------------------
# rendering


def cell(text: str) -> str:
    return text.replace("|", "\\|").replace("`", "'")


def render_gaps(report: dict, add) -> None:
    add("### Missed edges ranked by gap")
    add("")
    add("Every observed call or callback edge polint did not find is attributed to one gap: the first in the order")
    add("unary IIFEs, try / catch / if blocks, exported initializers, constructors, callbacks to built-ins, computed")
    add("keys, prototype and class methods, then the remainders. Extraction gaps come first because no resolution")
    add("change finds an edge whose call site or callee polint does not extract; a site or callee polint misses is")
    add("charged to the outermost function around it that polint does not extract. **attributed** counts each edge")
    add("once; **applying** counts every gap whose condition holds. The recall gain is an upper bound: it assumes")
    add("every attributed edge is found once the gap is fixed. The syntax census behind this is described in the README.")
    add("")
    ranking = report["gap_ranking"]
    base, refined = ranking["base"], ranking["refined"]
    refined_rows = {row["gap"]: row for row in refined["rows"]}
    add(f"Base: {base['missed_edges']} missed of {base['observed_edges']} observed edges. Refined: "
        f"{refined['missed_edges']} missed of {refined['observed_edges']}"
        + (f", without {', '.join(refined['left_out'])} (no refined dump)" if refined["left_out"] else "")
        + ".")
    add("")
    add("| rank | gap | attributed base | applying base | recall gain base (upper bound) | attributed refined | recall gain refined (upper bound) | base by package |")
    add("|---|---|---|---|---|---|---|---|")
    for rank, row in enumerate(base["rows"], 1):
        other = refined_rows.get(row["gap"], {})
        label = row["label"] if row["fixable_gap"] else f"_{row['label']}_"
        by_package = ", ".join(f"{name} {count}" for name, count in row["by_package"].items()) or "–"
        add(
            f"| {rank} | {label} | {row['attributed']} | {row['applying']} | {pct1(row['recall_gain_upper_bound'])} "
            f"| {other.get('attributed', 'n/a')} | {pct1(other.get('recall_gain_upper_bound'))} | {by_package} |"
        )
    add("")
    add("Rows in italics are remainders, not one of the named extraction or resolution gaps. Of the base")
    add(f"prototype and class method misses, {base['class_method_through_inheritance']} go through `super` or a `this`")
    add("call into another class; that split reads the receiver syntactically and is approximate.")
    add("'Other resolution misses' by call syntax and callee kind (base): "
        + ", ".join(f"{shape} {count}" for shape, count in base["other_by_syntax_and_callee"].items())
        + ". An `expression` call is one whose callee is neither a name nor a member, such as the `(0, f)(x)`")
    add("form compiled CommonJS uses.")
    add("")


def render_false_complete(report: dict, add) -> None:
    add("### Other targets and no unresolved marker (every case)")
    add("")
    add("The observed callee is not among polint's targets for the site, and the site carries no unresolved marker.")
    add("The polint site column shows which call expression the observed site was matched to.")
    add("")
    add("| package | plan | observed site | syntax / kind | observed callee | polint answer | observed site text | polint site text |")
    add("|---|---|---|---|---|---|---|---|")
    any_case = False
    for name, entry in report["packages"].items():
        for plan in PLANS:
            for example in entry.get(plan, {}).get("outcome_examples", {}).get(FALSE_COMPLETE, []):
                any_case = True
                site = example["polint_site"]
                add(
                    f"| {name} | {plan} | {example['site']} | {example['syntax']} / {example['kind']} "
                    f"| {cell(example['observed_callee'])} | {cell(polint_answer(example))} "
                    f"| `{cell(example['site_text'])}` | `{cell(site['text'])}` |"
                )
    if not any_case:
        add("| none | | | | | | | |")
    add("")


def render_wrong_targets(report: dict, add) -> None:
    plans = list(report["plans"])
    add("### Claimed targets the run never called")
    add("")
    add("A claimed target is resolved and not heuristic. It counts here when the run executed the site, the target is")
    add("in an instrumented file, and the run never called it from that site. On a **single-target** answer that is a")
    add("false precise edge: every execution of the site went elsewhere. On a **multi-target** answer it is an upper")
    add("bound only, since the run may not have taken the paths to the other targets. A site matched to a polint site")
    add("by its end byte can also inflate both counts.")
    add("")
    add("| package | " + " | ".join(f"{plan}: single / multi" for plan in plans) + " |")
    add("|---|" + "---|" * len(plans))
    totals = {plan: Counter() for plan in plans}
    for name, entry in report["packages"].items():
        cells = []
        for plan in plans:
            wrong = entry.get(plan, {}).get("wrong_targets")
            if wrong is None:
                cells.append("n/a")
                continue
            totals[plan].update({key: wrong[key] for key in ("single_target", "multi_target")})
            cells.append(f"{wrong['single_target']} / {wrong['multi_target']}")
        add(f"| {name} | " + " | ".join(cells) + " |")
    add(
        "| **total** | "
        + " | ".join(f"**{totals[plan]['single_target']} / {totals[plan]['multi_target']}**" for plan in plans)
        + " |"
    )
    add("")
    add("| package | plan | site | site text | single-target answer | the run called |")
    add("|---|---|---|---|---|---|")
    any_case = False
    for name, entry in report["packages"].items():
        for plan in plans:
            for example in entry.get(plan, {}).get("wrong_targets", {}).get("examples", []):
                if example["answer"] != "single_target":
                    continue
                any_case = True
                add(
                    f"| {name} | {plan} | {example['site']} | `{cell(example['site_text'])}` "
                    f"| {cell(example['target'])} | {cell(', '.join(example['observed_callees']))} |"
                )
    if not any_case:
        add("| none | | | | | |")
    add("")


def render_outcome_examples(report: dict, add) -> None:
    add("### One example per package, plan and outcome")
    add("")
    add(f"`report.json` keeps up to {EXAMPLES_PER_OUTCOME} per outcome (all of them for '{FALSE_COMPLETE}') and up to")
    add(f"{EXAMPLES_PER_GAP} per gap.")
    add("")
    add("| package | plan | outcome | observed site | syntax / kind | observed callee | polint answer |")
    add("|---|---|---|---|---|---|---|")
    for name, entry in report["packages"].items():
        for plan in PLANS:
            for outcome, examples in entry.get(plan, {}).get("outcome_examples", {}).items():
                example = examples[0]
                add(
                    f"| {name} | {plan} | {outcome} | {example['site']} `{cell(example['site_text'])}` "
                    f"| {example['syntax']} / {example['kind']} | {cell(example['observed_callee'])} "
                    f"| {cell(polint_answer(example))} |"
                )
    add("")


def render_types(report: dict, add) -> None:
    """What the TypeScript type tier adds to the refined plan, and what it costs."""
    add(f"### What types add (refined vs typed, TypeScript {report.get('typescript')})")
    add("")
    add("**typed** is the refined plan with `[languages.ts] type_sidecar = true` over a copy of each tree with a")
    add("tsconfig (`allowJs`, `nodenext`). The type tier only feeds the refined layer, so base is unchanged by it.")
    add("Dump time and peak RSS are one fact dump each, sidecar included.")
    add("")
    add("| package | recall refined | recall typed | false-complete refined / typed | type callsite rows | dump s refined / typed | dump peak MB refined / typed |")
    add("|---|---|---|---|---|---|---|")
    dump_cost = report["dump_cost"]
    for name, entry in report["packages"].items():
        refined, typed = entry.get("refined", {}), entry.get(TYPED_PLAN, {})

        def recall(scored):
            if "error" in scored:
                return scored["error"]
            return frac(scored["found"], scored["observed_edges"])

        def cost(plan, key, scale):
            row = dump_cost.get(name, {}).get(plan)
            return f"{row[key] / scale:.1f}" if row else "n/a"

        add(
            f"| {name} | {recall(refined)} | {recall(typed)} "
            f"| {refined.get('outcomes', {}).get(FALSE_COMPLETE, 0)} / {typed.get('outcomes', {}).get(FALSE_COMPLETE, 0)} "
            f"| {typed.get('ts_types', {}).get('callsites', 'n/a')} "
            f"| {cost('refined', 'wall_ms', 1000)} / {cost(TYPED_PLAN, 'wall_ms', 1000)} "
            f"| {cost('refined', 'peak_rss_bytes', MB)} / {cost(TYPED_PLAN, 'peak_rss_bytes', MB)} |"
        )
    add("")
    fc = report["constructs"]["false_complete"]
    add(
        f"Constructs, typed: false-complete hidden paths {fc.get(TYPED_PLAN, 0)}, "
        f"false-complete call lines {fc.get('call_lines_' + TYPED_PLAN, 0)}."
    )
    add("")


TIER_VIEWS = (
    ("exact_complete", "exact-complete"),
    ("exact", "exact"),
    ("exact_conservative", "exact + conservative"),
    ("apart_only", "apart only"),
)


def scored_plans(packages: dict, plans: list[str]):
    """(name, plan, scored entry) for every package and plan that has a dump."""
    for name, entry in packages.items():
        for plan in plans:
            scored = entry.get(plan, {})
            if "tiers" in scored:
                yield name, plan, scored


def render_tiers(packages: dict, plans: list[str], add, title: str = "") -> None:
    add(f"## Tiers{title}")
    add("")
    add("Recall over the same observed edges as above, split by the tier of the edge that found the callee")
    add("(`research/package-graphs/TIERS.md`): `Exact` and `SetupAware` are exact, except typed-hierarchy edges")
    add("that dispatch to an `implementation` or `union_member`, which are candidates and count as conservative;")
    add("`Conservative` and `Ambiguous` are conservative; `Heuristic` is apart and never counts in the first three")
    add("columns. **exact-complete** also needs the site to be complete: at least one exact or conservative target and")
    add("no unresolved row.")
    add("")
    add("| package | plan | observed edges | " + " | ".join(label for _, label in TIER_VIEWS) + " | apart edges |")
    add("|---|---|---|" + "---|" * len(TIER_VIEWS) + "---|")
    totals = defaultdict(Counter)
    for name, plan, scored in scored_plans(packages, plans):
        tiers = scored["tiers"]
        totals[plan]["observed"] += scored["observed_edges"]
        totals[plan].update(tiers["found"])
        totals[plan]["apart_edges"] += tiers["apart_edges"]
        add(
            f"| {name} | {plan} | {scored['observed_edges']} | "
            + " | ".join(frac(tiers["found"][view], scored["observed_edges"]) for view, _ in TIER_VIEWS)
            + f" | {tiers['apart_edges']} |"
        )
    for plan in plans:
        if plan in totals:
            row = totals[plan]
            add(
                f"| **total** | {plan} | {row['observed']} | "
                + " | ".join(frac(row[view], row["observed"]) for view, _ in TIER_VIEWS)
                + f" | {row['apart_edges']} |"
            )
    add("")
    add("Targets per site that claims any in the tier: mean / p95 (nearest rank) / max.")
    add("")
    widths = ("exact", "conservative", "exact_conservative", "apart")
    add("| package | plan | " + " | ".join(view.replace("_", " + ") for view in widths) + " |")
    add("|---|---|" + "---|" * len(widths))
    for name, plan, scored in scored_plans(packages, plans):
        cells = []
        for view in widths:
            row = scored["tiers"]["targets_per_site"][view]
            cells.append("–" if not row["sites"] else f"{row['mean']} / {row['p95']} / {row['max']} ({row['sites']} sites)")
        add(f"| {name} | {plan} | " + " | ".join(cells) + " |")
    add("")


def render_tier_precision(packages: dict, plans: list[str], add, title: str = "") -> None:
    add(f"## Precision by tier{title}")
    add("")
    add("At executed sites only: a site counts once the run was seen calling a polint function from it, and a")
    add("claimed target counts when it is a function in an instrumented file. **precision** is the share of those")
    add("targets the run called there; it is a lower bound, since a run takes only some paths. A target the run never")
    add("called on a single-target answer in the exact tier is a **wrong exact target**; the gate is 0.")
    add("")
    add("| package | plan | " + " | ".join(f"{tier}: precision, single / multi not called" for tier in CLAIM_TIERS) + " |")
    add("|---|---|" + "---|" * len(CLAIM_TIERS))
    totals = defaultdict(Counter)
    for name, plan, scored in scored_plans(packages, plans):
        cells = []
        for tier in CLAIM_TIERS:
            row = scored["tiers"]["precision"][tier]
            totals[(plan, tier)].update({key: row[key] for key in ("claimed", "observed", "single_target", "multi_target")})
            cells.append(
                "–" if not row["claimed"]
                else f"{frac(row['observed'], row['claimed'])}, {row['single_target']} / {row['multi_target']}"
            )
        add(f"| {name} | {plan} | " + " | ".join(cells) + " |")
    for plan in plans:
        cells = []
        for tier in CLAIM_TIERS:
            row = totals.get((plan, tier))
            if row is None:
                break
            cells.append(
                "–" if not row["claimed"]
                else f"**{frac(row['observed'], row['claimed'])}, {row['single_target']} / {row['multi_target']}**"
            )
        if len(cells) == len(CLAIM_TIERS):
            add(f"| **total** | {plan} | " + " | ".join(cells) + " |")
    add("")
    add("| package | plan | wrong exact target at | site text | target | the run called |")
    add("|---|---|---|---|---|---|")
    any_case = False
    for name, plan, scored in scored_plans(packages, plans):
        for example in scored["tiers"]["precision"]["exact"]["examples"]:
            if example["answer"] != "single_target":
                continue
            any_case = True
            add(
                f"| {name} | {plan} | {example['site']} | `{cell(example['site_text'])}` "
                f"| {cell(example['target'])} | {cell(', '.join(example['observed_callees']))} |"
            )
    if not any_case:
        add("| none | | | | | |")
    add("")


def render_open_sites(packages: dict, plans: list[str], add, title: str = "") -> None:
    add(f"## Open sites{title}")
    add("")
    add("A site is open when it has no exact or conservative target, or the answer marks it unresolved. Each open")
    add("site counts under every reason its answer gives; `none recorded` is an open site whose answer gives none,")
    add("which the engine must not produce.")
    add("")
    add("| package | plan | call sites | complete | open | open with a claimed target | open by reason |")
    add("|---|---|---|---|---|---|---|")
    for name, plan, scored in scored_plans(packages, plans):
        row = scored["tiers"]["sites"]
        reasons = ", ".join(f"{cell(reason)} {count}" for reason, count in row["open_by_reason"].items()) or "–"
        add(
            f"| {name} | {plan} | {row['call_sites']} | {frac(row['complete'], row['call_sites'])} | {row['open']} "
            f"| {row['open_with_a_claimed_target']} | {reasons} |"
        )
    add("")


def render_budget_events(packages: dict, plans: list[str], add, title: str = "") -> None:
    add(f"## Budget events{title}")
    add("")
    add("The solver's budget status for each dump and every budget it reported as reached. The target is none on")
    add("the tune set, and a budget event must never discard an edge already derived.")
    add("")
    add("| package | plan | status | reasons |")
    add("|---|---|---|---|")
    for name, plan, scored in scored_plans(packages, plans):
        budget = scored["solver_budget"]
        add(f"| {name} | {plan} | {budget['status']} | {', '.join(budget['reasons']) or '–'} |")
    add("")


def render_refined_edges(packages: dict, plans: list[str], add, title: str = "") -> None:
    add(f"## Refined edges{title}")
    add("")
    add("Every refined row by the tier the bench gives it, its algorithm, status and precision, and whether it")
    add("names a function. Rows with a status other than `Resolved` or `Ambiguous` claim nothing.")
    add("")
    add("| package | plan | tier | algorithm | status | precision | names a function | rows |")
    add("|---|---|---|---|---|---|---|---|")
    for name, plan, scored in scored_plans(packages, [plan for plan in plans if plan != "base"]):
        for row in scored["refined_edge_census"]:
            add(
                f"| {name} | {plan} | {row['tier']} | {row['algorithm']} | {row['status']} | {row['precision']} "
                f"| {'yes' if row['has_function'] else 'no'} | {row['rows']} |"
            )
    add("")


def render_sites_gained(packages: dict, plans: list[str], add, title: str = "") -> None:
    add(f"## Sites gained by the refined plan{title}")
    add("")
    add("Sites where a refined plan claims an exact or conservative target and the base plan claims none, by file.")
    add("**test path** marks a file in a test or benchmark folder or named `*.test.*` / `*.spec.*`; **executed** counts")
    add("the gained sites the run called a polint function from, the only ones recall can credit.")
    add("")
    add("| package | plan | sites gained | in test paths | executed | top files |")
    add("|---|---|---|---|---|---|")
    for name, plan, scored in scored_plans(packages, [plan for plan in plans if plan != "base"]):
        rows = scored.get("sites_gained")
        if rows is None:
            continue
        top = ", ".join(f"{row['file']} {row['sites']}" for row in rows[:3]) or "–"
        add(
            f"| {name} | {plan} | {sum(row['sites'] for row in rows)} "
            f"| {sum(row['sites'] for row in rows if row['test_path'])} | {sum(row['executed'] for row in rows)} "
            f"| {cell(top)} |"
        )
    add("")


def render_analysed_set_comparisons(report: dict, add) -> None:
    comparisons = report.get("analysed_set_comparisons") or {}
    if not comparisons:
        return
    add("## Analysed set: globs alone against shipped source")
    add("")
    add("The analysed set of every package is its shipped source: the `analysed` globs without test and benchmark")
    add("paths. Reports written before this rule analysed what the globs alone select; each package listed here is")
    add("scored both ways once, so this report reads against them. Every other table uses the shipped set.")
    add("")
    add("| package | set | files | source KB | plan | recall | exact / exact + conservative | wrong targets single / multi | wrong exact targets | call sites | dump s | dump peak MB |")
    add("|---|---|---|---|---|---|---|---|---|---|---|---|")
    for name, sets in comparisons.items():
        for label, row in sets.items():
            cost_key = previous_set_cost_key(name) if label == PREVIOUS_SET else name
            for plan in report["plans"]:
                scored = row.get(plan, {})
                cost = report["dump_cost"].get(cost_key, {}).get(plan) or {}
                prefix = (
                    f"| {name} | {'globs alone' if label == PREVIOUS_SET else 'shipped'} | {row['files']} "
                    f"| {(row['source_bytes'] or 0) / 1000:.0f} | {plan} "
                )
                if "error" in scored:
                    add(prefix + f"| {scored['error']} | | | | | | |")
                    continue
                tiers = scored["tiers_found"]
                add(
                    prefix
                    + f"| {frac(scored['found'], scored['observed_edges'])} "
                    f"| {tiers['exact']} / {tiers['exact_conservative']} "
                    f"| {scored['wrong_targets']['single_target']} / {scored['wrong_targets']['multi_target']} "
                    f"| {scored['wrong_exact_targets']} | {scored['call_sites']} "
                    f"| {cost.get('wall_ms', 0) / 1000:.1f} | {cost.get('peak_rss_bytes', 0) / MB:.0f} |"
                )
    add("")


def render_tier_sections(packages: dict, plans: list[str], add, title: str = "") -> None:
    """The per-tier measures, in the order the README describes them."""
    render_tiers(packages, plans, add, title)
    render_tier_precision(packages, plans, add, title)
    render_open_sites(packages, plans, add, title)
    render_budget_events(packages, plans, add, title)
    render_refined_edges(packages, plans, add, title)
    render_sites_gained(packages, plans, add, title)


def outside_tools():
    """The outside-tool scorer, which imports this module for its matching."""
    sys.path.insert(0, str(bench.BENCH_DIR / "outside"))
    import score_outside

    return score_outside


def coverage_cell(entry: dict) -> str:
    coverage = entry.get("coverage") or {}
    if coverage.get("coverage") is None:
        return "n/a"
    cell = f"{frac(coverage['entered_census_functions'], coverage['census_functions'])}"
    return f"{cell} (flagged)" if coverage["flagged"] else cell


def render_held_out(report: dict, add) -> None:
    """The held-out packages: the tune set's measures, apart, with no miss attribution."""
    held_out = report["held_out"]
    plans = list(report["plans"])
    gate = report["held_out_gate"]
    add("## Held-out packages")
    add("")
    add("Packages no analysis change may be tuned on (`research/package-graphs/HELD-OUT.md`). They get the tune")
    add("set's measures with no gap ranking, no cause and no example of a missed edge. Wrong exact targets and")
    add("answers with other targets and no unresolved marker keep their examples: either is a defect to fix.")
    add(f"A package whose recording entered under {COVERAGE_FLOOR:.0%} of its functions is flagged and left out")
    add("of the held-out recall.")
    add("")
    add("| package | format | files analysed / selected | source KB | census functions | entered | coverage | observed edges | " + " | ".join(f"recall {plan}" for plan in plans) + " |")
    add("|---|---|---|---|---|---|---|---|" + "---|" * len(plans))
    for name, entry in held_out.items():
        coverage = entry["coverage"]
        recalls = []
        for plan in plans:
            scored = entry.get(plan, {})
            recalls.append(scored["error"] if "error" in scored else frac(scored["found"], scored["observed_edges"]))
        add(
            f"| {name} {entry['version']} | {entry['format']} | {entry.get('files', 'n/a')} / {entry['files_selected']} "
            f"| {entry.get('source_bytes', 0) / 1000:.0f} | {coverage['census_functions'] if coverage['census_functions'] is not None else 'n/a'} "
            f"| {coverage['entered_census_functions'] if coverage['entered_census_functions'] is not None else 'n/a'} "
            f"| {coverage_cell(entry)} | {entry.get('base', {}).get('observed_edges', 'n/a')} | " + " | ".join(recalls) + " |"
        )
    add("")
    add("### Held-out gate")
    add("")
    add(f"Recall per tier pooled over the tune set and over the unflagged held-out packages "
        f"({', '.join(gate['counted']) or 'none'}; flagged: {', '.join(gate['flagged']) or 'none'}). "
        f"The gate holds when held-out recall is at most {gate['margin_points']} points below the tune set's in every tier,")
    add("and no held-out package, flagged or not, has a wrong exact target or an answer with other targets and no")
    add("unresolved marker.")
    add("")
    add("| plan | tier | tune | held-out | points below tune | within |")
    add("|---|---|---|---|---|---|")
    labels = dict(TIER_VIEWS)
    for plan in plans:
        for view, row in gate["plans"][plan]["tiers"].items():
            gap = "n/a" if row["points_below_tune"] is None else f"{row['points_below_tune']:.1f}"
            add(f"| {plan} | {labels[view]} | {pct1(row['tune'])} | {pct1(row['held_out'])} | {gap} | {'yes' if row['within'] else 'no'} |")
    add("")
    add("| plan | wrong exact targets | other targets, no unresolved marker |")
    add("|---|---|---|")
    for plan in plans:
        row = gate["plans"][plan]
        add(f"| {plan} | {row['wrong_exact_targets']} | {row['false_complete']} |")
    add("")
    held_report = {**report, "packages": held_out}
    render_wrong_targets(held_report, add)
    render_false_complete(held_report, add)
    render_tier_sections(held_out, plans, add, " (held-out)")


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
    add("| construct | cases (held out) | call recall base | call recall refined | callback recall base | callback recall refined | precision base | precision refined | hidden paths base: reported / false-complete / silent | hidden paths refined | false-complete call lines base / refined | stops at import binding (base) |")
    add("|---|---|---|---|---|---|---|---|---|---|---|---|")
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
            f"| {hidden(base)} | {hidden(refined)} "
            f"| {base.get('false_complete_call_lines', 0)} / {refined.get('false_complete_call_lines', 0)} "
            f"| {base.get('import_binding_stops', 0)} |"
        )
    add("")
    fc = report["constructs"]["false_complete"]
    add(
        f"**False-complete count** (a labelled hidden path where polint gives a resolved answer with no "
        f"unresolved marker): base {fc['base']}, refined {fc['refined']}, over {fc['hidden_lines']} labelled hidden paths."
    )
    add("")
    add(
        f"**False-complete call lines** (a labelled call line where polint gives resolved targets that miss a "
        f"labelled callee, with no unresolved marker on the line): base {fc['call_lines_base']}, refined "
        f"{fc['call_lines_refined']}, over {fc['call_lines']} labelled call lines."
    )
    add("")
    add("| case | plan | line | resolved to | missed |")
    add("|---|---|---|---|---|")
    any_line = False
    for case_id, entry in sorted(report["constructs"]["cases"].items()):
        for plan in PLANS:
            for row in entry[plan].get("false_complete_call_lines", []):
                any_line = True
                add(
                    f"| {case_id} | {plan} | {row['file']}:{row['line']} | {', '.join(row['resolved'])} "
                    f"| {', '.join(row['missed'])} |"
                )
    if not any_line:
        add("| none | | | | |")
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
    add("Coverage is the share of the analysed files' functions (syntax census, classes and static blocks left out)")
    add("that the recording entered.")
    add("")
    add("| package | format | files analysed / selected | source KB | coverage | observed edges | recall base | recall refined | stops at import binding (base) | not mappable to a polint site or function | unresolved call share base | call sites without target base / refined | unresolved reference share |")
    add("|---|---|---|---|---|---|---|---|---|---|---|---|---|")
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
            f"| {entry.get('source_bytes', 0) / 1000:.0f} | {coverage_cell(entry)} | {base.get('observed_edges', 'n/a')} "
            f"| {recall(base)} | {recall(refined)} "
            f"| {base.get('outcomes', {}).get('stops at import binding', 0)} | {unmappable} "
            f"| {pct(base.get('unresolved_calls', {}).get('marked_unresolved_share'))} | {without} "
            f"| {pct(base.get('references', {}).get('not_resolved_share'))} |"
        )
    add("")
    render_wrong_targets(report, add)
    if TYPED_PLAN in report["plans"]:
        render_types(report, add)
    else:
        add("The typed plan (refined with the TypeScript type sidecar) was not run: no TypeScript compiler was given.")
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

    render_gaps(report, add)
    render_false_complete(report, add)
    render_outcome_examples(report, add)
    render_tier_sections(report["packages"], list(report["plans"]), add)
    render_analysed_set_comparisons(report, add)
    if report.get("outside") is not None:
        outside_tools().render(report["outside"], report["packages"], list(report["plans"]), add)

    if report.get("held_out"):
        render_held_out(report, add)

    add("## Exports (against the loaded module's export keys)")
    add("")
    add("`Symbols` is the exported-symbol answer a rule can read; export facts are the module export rows.")
    add("Both are compared with the export keys the loaded module actually has. A file that fails to load")
    add("on its own is left out of both and counted under `load errors`.")
    add("")
    add("| tree | files exact (Symbols) | name recall (Symbols) | name precision (Symbols) | files exact (export facts) | name recall (export facts) | name precision (export facts) | export fact rows by kind/status | load errors |")
    add("|---|---|---|---|---|---|---|---|---|")
    rows = [(f"package {n}", e.get("exports")) for n, e in report["packages"].items()]
    rows += [(f"held-out package {n}", e.get("exports")) for n, e in report.get("held_out", {}).items()]
    rows += [(f"case {n}", e.get("exports")) for n, e in sorted(report["constructs"]["cases"].items())]
    for name, exports in rows:
        if not exports or not exports["symbols"]["files"] + exports["facts"]["files"]:
            continue
        s, f = exports["symbols"], exports["facts"]
        statuses = ", ".join(f"{k} {v}" for k, v in exports["export_fact_rows"].items()) or "–"
        add(
            f"| {name} | {s['files_exact']}/{s['files']} | {frac(s['matched_names'], s['observed_names'])} "
            f"| {frac(s['matched_names'], s['predicted_names'])} | {f['files_exact']}/{f['files']} "
            f"| {frac(f['matched_names'], f['observed_names'])} | {frac(f['matched_names'], f['predicted_names'])} | {statuses} "
            f"| {exports.get('load_errors', 0)} |"
        )
    add("")

    add("## Cost")
    add("")
    cost = report.get("cost")
    if not cost:
        add("Not measured in this run (`--no-cost`).")
    else:
        add("The only part of this report that changes between runs. Machine: "
            f"{cost['machine']['platform']}, {cost['machine']['cpus']} CPUs, commit {cost['machine']['git_head'][:12]}, "
            f"load average {load_line(cost['machine'].get('load_average_start'))} at the start and "
            f"{load_line(cost['machine'].get('load_average_end'))} at the end (1, 5, 15 minutes).")
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
        add("| tree | base ms | base peak RSS MB | refined ms | refined peak RSS MB | budget s |")
        add("|---|---|---|---|---|---|")
        for name, row in cost["dump"].items():
            add(
                f"| {name} | {row['base']['wall_ms']} | {row['base']['peak_rss_bytes'] / MB:.0f} "
                f"| {row['refined']['wall_ms']}{' (' + row['refined']['error'] + ')' if row['refined'].get('error') else ''} "
                f"| {row['refined']['peak_rss_bytes'] / MB:.0f} | {report['dump_budgets_s'].get(name, 'none')} |"
            )
    add("")
    render_stage_times(report, add)
    add("## Decisions recorded by this run")
    add("")
    for decision in report["decisions"]:
        add(f"- {decision}")
    add("")
    return "\n".join(out)


def load_line(load: list[float] | None) -> str:
    return "not recorded" if not load else " / ".join(f"{value:.2f}" for value in load)


SEMANTIC_GRAPH_STAGE = "polint.semantic_graph"


def render_stage_times(report: dict, add) -> None:
    """The three largest provider stages of every fact dump, from the kernel's stage log."""
    add("### Stage times")
    add("")
    add(f"Every fact dump runs with `RUST_LOG={STAGE_LOG}`; each provider's `stage done` lines are summed. "
        "Stage times are as noisy as the dump times above. `semantic_graph share` is the "
        f"`{SEMANTIC_GRAPH_STAGE}` stage's part of the refined plan's summed stage time. Peak MB is the process "
        "peak when the stage ended.")
    add("")
    add("| tree | plan | stage total s | largest stages: s (peak MB) | semantic_graph share |")
    add("|---|---|---|---|---|")
    for name, plans in report["dump_cost"].items():
        for plan, row in plans.items():
            stages = row.get("stages") or {}
            total = sum(stage["elapsed_ms"] for stage in stages.values())
            largest = sorted(stages.items(), key=lambda item: (-item[1]["elapsed_ms"], item[0]))[:3]
            listed = ", ".join(
                f"`{provider.removeprefix('polint.')}` {stage['elapsed_ms'] / 1000:.1f} ({stage['peak_rss_mb']})"
                for provider, stage in largest
            ) or "no stage log"
            share = "n/a"
            if plan == "refined" and total:
                share = pct(stages.get(SEMANTIC_GRAPH_STAGE, {}).get("elapsed_ms", 0) / total)
            add(f"| {name} | {plan} | {total / 1000:.1f} | {listed} | {share} |")
    add("")


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
    "A labelled call line is false-complete when polint resolves a call site on it to labelled functions that "
    "miss a labelled `@calls` callee and no call site on the line carries an unresolved marker. Lines are the unit "
    "of the labels, so on a line with several call sites the targets and markers of all of them count together.",
    "A base edge whose target is an import binding rather than a function is a miss, counted as 'stops at import "
    "binding'.",
    "Observed package edges are mapped to polint by exact call-expression span (falling back to the polint call "
    "site in the same file ending at the same byte) and by callee span (the same span, else the innermost polint "
    "function in the same file ending at the same byte that does not start inside the observed span).",
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
    "An observed callee maps to the polint function with the same span before the innermost one ending at the same "
    "byte: a concise arrow returning an arrow, `(a) => (b) => ...`, ends at the same byte as the arrow it returns.",
    "Every missed observed edge is attributed to one gap from a syntax census: Oxc's own function and call spans with "
    "their syntactic context, parsed again apart from the analysis. A site or callee polint does not extract is "
    "charged to the outermost function around it that polint does not extract, else to the outermost enclosing "
    "function or the call in a gap; edges polint maps go to constructors, callbacks to built-ins, computed keys and "
    "prototype or class methods, in that order. A computed-key miss is counted whether or not the key set is known, "
    "so that row is an upper bound for computed keys over a known set.",
    "A census function counts as extracted when polint has a function in the same file ending at the same byte, "
    "or, for a class constructor, when polint has the class.",
    "Every package edge with other targets and no unresolved marker is kept as an example; other outcomes and gaps "
    "keep a bounded sample, in observed-edge order.",
    "Process peak RSS is measured with `wait4` like the matrix runner does. Linux carries the parent's resident "
    "size into the child's peak at fork, so every process is launched before the report loads any fact dump, and "
    "the launcher's own peak is recorded with the cost section. The in-process analysis peak has no such floor.",
    "Each fact dump has a wall-clock limit (`--dump-timeout`, 600 s by default); a tree whose dump exceeds it is "
    "reported as timed out for that plan and has no scores in that column.",
    "A package with `dump_budget_s` in `inputs.toml` has a time budget for each of its fact dumps; the report is "
    "written either way, and the run exits with status 1 naming every dump that ran over its budget or timed out.",
]


# ---------------------------------------------------------------------------
# entry


def run(args) -> int:
    inputs = bench.load_inputs()
    cases = bench.construct_cases()
    typescript = find_typescript(args)
    if typescript is not None:
        PLANS[TYPED_PLAN] = PLANS["refined"]
    polint, test_binary = build(args)
    # The machine's load once the build is done, before anything is measured.
    load_at_start = os.getloadavg()
    version = subprocess.run([str(polint), "--version"], capture_output=True, text=True, check=True).stdout.split()[-1]

    budgets = {package["name"]: package["dump_budget_s"] for package in inputs["package"] if "dump_budget_s" in package}

    with tempfile.TemporaryDirectory(prefix="polint-js-bench-report-") as scratch_name:
        scratch = Path(scratch_name)
        case_trees, package_trees = stage(scratch, inputs, cases)
        previous_trees = stage_previous_sets(scratch, inputs)
        typed_trees = {}
        if typescript is not None:
            typed_trees = stage_typed(
                scratch,
                {
                    **{("case", case_id): tree for case_id, tree in case_trees.items()},
                    **{("package", name): tree for name, tree in package_trees.items()},
                    **{(PREVIOUS_SET, name): tree for name, tree in previous_trees.items()},
                },
            )
        (scratch / "dumps").mkdir()
        dumps = {}
        trees = [("case", case_id, tree) for case_id, tree in case_trees.items()]
        trees += [("package", name, tree) for name, tree in package_trees.items()]
        trees += [(PREVIOUS_SET, name, tree) for name, tree in previous_trees.items()]
        for kind, name, tree in trees:
            for plan in PLANS:
                out = scratch / "dumps" / f"{kind}-{name.replace('/', '__')}-{plan}.json"
                if plan == TYPED_PLAN:
                    dumps[(kind, name, plan)] = {
                        **run_dump(test_binary, typed_trees[(kind, name)], plan, out, args.dump_timeout, typescript),
                        "typed": True,
                    }
                else:
                    dumps[(kind, name, plan)] = run_dump(test_binary, tree, plan, out, args.dump_timeout)
                if dumps[(kind, name, plan)]["error"]:
                    print(f"{kind} {name} {plan}: {dumps[(kind, name, plan)]['error']}", file=sys.stderr)
            if kind == "package":
                out = scratch / "dumps" / f"{kind}-{name}-census.json"
                dumps[(kind, name, "census")] = run_dump(test_binary, tree, "census", out, args.dump_timeout)
                if dumps[(kind, name, "census")]["error"]:
                    print(f"{kind} {name} census: {dumps[(kind, name, 'census')]['error']}", file=sys.stderr)
                print(f"dumped {name}", file=sys.stderr)

        over_budget = [
            f"{name} {plan}: {run_entry['cost']['wall_ms'] / 1000:.1f} s, budget {budgets[name]} s"
            for (kind, name, plan), run_entry in dumps.items()
            if kind == "package"
            and name in budgets
            and (run_entry["cost"]["wall_ms"] > budgets[name] * 1000 or (run_entry["error"] or "").startswith("timed out"))
        ]

        # Every measured process is started before any dump is loaded. A child's
        # peak RSS from wait4 starts at this process's own resident size when it
        # forks, so this process must stay small while it measures.
        tune_names = [package["name"] for package in inputs["package"] if not package.get("held_out")]
        check_cost = None
        if not args.no_cost:
            constructs_tree = scratch / "cost-constructs"
            for case_id, tree in case_trees.items():
                shutil.copytree(tree, constructs_tree / case_id)
            # The cost runs measure the tune set, the set the cost targets are stated on.
            cost_trees = {"constructs (all cases)": constructs_tree, **{name: package_trees[name] for name in tune_names}}
            check_cost = measure_cost(polint, cost_trees, scratch, args.cost_runs, args.dump_timeout)
        launcher_peak_rss_bytes = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * 1024
        if args.keep_dumps:
            shutil.copytree(scratch / "dumps", Path(args.keep_dumps), dirs_exist_ok=True)
        for run_entry in dumps.values():
            load_dump(run_entry)

        case_results = score_constructs(cases, dumps)
        hidden_lines = sum(len(e["base"].get("unknown_lines", [])) for e in case_results.values())
        scored_packages = score_packages(inputs, dumps)
        packages = {name: entry for name, entry in scored_packages.items() if not entry["held_out"]}
        held_out = {name: entry for name, entry in scored_packages.items() if entry["held_out"]}
        comparisons = score_previous_sets(inputs, dumps, packages)
        report = {
            "schema": REPORT_SCHEMA,
            "polint_version": version,
            "plans": {plan: caps.split(",") for plan, caps in PLANS.items()},
            "typescript": typescript_version(typescript),
            "constructs": {
                "summary": construct_summary(case_results),
                "false_complete": {
                    "hidden_lines": hidden_lines,
                    "call_lines": sum(e["base"].get("labelled_call_lines", 0) for e in case_results.values()),
                    **{
                        f"call_lines_{plan}": sum(
                            len(e[plan].get("false_complete_call_lines", [])) for e in case_results.values()
                        )
                        for plan in PLANS
                    },
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
            "packages": packages,
            "held_out": held_out,
            "held_out_gate": held_out_gate(packages, held_out, list(PLANS)),
            "dump_budgets_s": budgets,
            # One fact dump per package and plan, kept even without the cost runs.
            "dump_cost": {
                **{name: {plan: dumps[("package", name, plan)]["cost"] for plan in PLANS} for name in package_trees},
                **{
                    previous_set_cost_key(name): {plan: dumps[(PREVIOUS_SET, name, plan)]["cost"] for plan in PLANS}
                    for name in previous_trees
                },
            },
            "gap_ranking": rank_gaps(packages),
            "analysed_set_comparisons": comparisons,
            "outside": outside_tools().score_all(inputs),
            "decisions": DECISIONS,
            "cost": None,
        }

        if check_cost is not None:
            add_per_mb(
                check_cost["constructs (all cases)"],
                sum(
                    sum(row["bytes"] for row in dumps[("case", case_id, "base")]["dump"]["files"])
                    for case_id in case_trees
                    if dumps[("case", case_id, "base")]["dump"] is not None
                ),
            )
            for name in tune_names:
                add_per_mb(check_cost[name], report["packages"][name].get("source_bytes", 0))
            report["cost"] = {
                "machine": machine(launcher_peak_rss_bytes, load_at_start),
                "runs": args.cost_runs,
                "check": check_cost,
                "dump": {
                    name: {plan: {**dumps[("package", name, plan)]["cost"], "error": dumps[("package", name, plan)]["error"]} for plan in PLANS}
                    for name in tune_names
                },
            }

    out_dir = Path(args.out) if args.out else bench.BASELINE
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "report.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    (out_dir / "report.md").write_text(render_markdown(report))
    print(f"wrote {out_dir / 'report.json'} and {out_dir / 'report.md'}", file=sys.stderr)
    failed = [
        f"{kind} {name} {plan}: {run_entry['error']}"
        for (kind, name, plan), run_entry in sorted(dumps.items())
        if run_entry["error"]
    ]
    if failed:
        print("fact dumps that failed:\n" + "\n".join(failed), file=sys.stderr)
    if over_budget:
        print("fact dumps over their time budget:\n" + "\n".join(over_budget), file=sys.stderr)
    return 1 if failed or over_budget else 0
