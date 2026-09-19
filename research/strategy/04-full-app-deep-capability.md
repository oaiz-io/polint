# 04 — Full-Application Deep Capability: Why the Graph Layer Does Not Scale, and What to Build Instead

Date: 2026-09-19
Researcher: Claude Fable 5.1 (delegated)
Verified against: this worktree at `686461be` (origin/main, v0.3.10 plus the AGENTS.md doc commit). Every `file:line` below was re-read at that commit.
Reads with: [02-gap-analysis.md](02-gap-analysis.md) (axis B "scale" gap, gap items 6 and 7) and [03-build-plan.md](03-build-plan.md) (Stage 0 "scale root cause", Stage 2 keystone, Stage 3 envelope). This document is the Stage 0 "scale root cause" deliverable that report 03 asked for, extended into the architecture decision that Stage 2 and Stage 3 need before they can be sequenced.
Measured input: the full-application benchmark and deep-capability stress run of 2026-09-17/18 (local artifact, `/workspace/polint-bench-20260917/report.md`, not committed; referred to below as "the benchmark report"). Numbers quoted from it are labeled as such. Nothing else in this document is a measurement unless it names its source; everything else is code reading or a labeled hypothesis.

Note on numbering: `04-evaluation-as-a-weapon.md` already carries the `04` prefix in this series. This document was commissioned under the name `04-full-app-deep-capability.md` and keeps it; the README index should list both.

How to read this document: section 3 is the evidence and can be verified line by line against Appendix A; section 4 is the survey; sections 5 and 6 are the decision; sections 7 and 8 are what to build and how to know it worked; section 11 says what was not checked. A reader who wants only the decision should read the TL;DR, section 6, the Resolved Questions, and the one Open Question.

Cross-reference to the build plan. Report 03 sequences six stages and names the items this document refines:

| Report 03 item | Where this document lands it |
|---|---|
| Stage 0, "Scale root cause": profile the full pipeline, add a runtime ceiling and wall-clock budget, parallelise per-file stages | section 3 is the root cause; W1 to W4 are the fixes; W9 is the budget; per-file parallelism becomes per-unit parallelism in W6 |
| Stage 0, exit criterion "excalidraw full pipeline under 6 GB and 300 s" | already met by `.scale-envelope` X6 (5.47 GB, 235 s); this document sets the next envelope on the consumer backend (section 7) |
| Stage 1, IFDS tabulation | unchanged in content; should be built on the unit ICFG after W6 (section 4.6) |
| Stage 1, TS type sidecar | landed as PR #121; positioned in section 9 |
| Stage 2, Phase 67 summary manifests and invalidation frontier; Phase 68 internal query engine | W7 and W8, in scope for this track (Q1); storage shape resolved (Q3) |
| Stage 2, exit criterion "peak RSS proportional to the working set" | G9 and the W6 unit model; option B's query engine is the Stage 2 scheduling layer, not the representation |
| Stage 3, envelope on `grafana/grafana` on a 16 GB host | out of scope here; the consumer backend at 4,752 files is the gate this document sets, and grafana is the next corpus once G6 passes |
| Stage 3, parallel per-SCC summary closure | W8, after unit shards exist |
| Report 02 gap items 6 and 7 (summary persistence; envelope and per-file parallelism) | W7 to W9; W6 |

## TL;DR

- The deep stack is off in production and cannot be turned on. The kernel logs `requested_capabilities={}` on every consumer scan because no rule requests `calls`, `control_flow` or `dataflow`, and only those three capabilities seed the 15 deep providers (`crates/polint/src/analysis_kernel/provider.rs:1020-1028`, `:1070-1090`). Forced on, a deep scan of the 4,752-file Go backend does not finish on a 30 GB host at any time budget: `polint.semantic_mir` alone took 565 s and +10.4 GB, and the process was killed at 634 s and 18.3 GB inside `polint.cfg` (benchmark report, section 5.6).
- The Go sidecar is not the problem. Whole-program `packages.Load` plus SSA over 1,020 packages costs a flat 25 to 27 s at every scope and is overlapped by the prefetch (`crates/polint/src/go/semantic/prefetch.rs:1-23`; benchmark report, section 5.3.2). The wall is Rust-side lowering and graph construction.
- Root cause, from code: five compounding defects in the Rust graph layer, none of which is "the sidecar" and none of which requires new theory to fix.
  1. Identity is text. Every fact carries a composed stable key whose text embeds its parents' full keys (body key embeds owner key embeds path; operation key embeds body key and up to four place keys; dominator key embeds function key and two block keys), interned once and retained for the run in one global `RwLock` interner that its own doc comment says cannot be sharded (`crates/polint/src/internal_core/stable_key.rs:24-41`). On the full backend that is 5,525,029 keys and 3,448 MB of key text (benchmark report, section 5.6). Every fact also carries a heap-allocated `payload_digest: String` (`crates/polint/src/analysis_api/metadata.rs:231-239`).
  2. Quadratic joins in the lowerers. `matching_function` scans every function in the program once per function declaration (`crates/polint/src/go/mir/lower.rs:2317-2329`, `crates/polint/src/ts/mir/lower.rs:4540-4553`); `push_body` scans every package and every module node once per body (`go/mir/lower.rs:645-656`); closure capture analysis scans every reference in the database once per closure through the trait default `references_for_file` (`crates/polint/src/analysis_neutral/host.rs:129-138`) even though an indexed inherent method exists and is not used (`crates/polint/src/core/db.rs:3746-3753`); `lower_control_flow` filters every operation in the program once per body (`go/mir/lower.rs:115-124`, `ts/mir/lower.rs:132-141`). The same shape recurs downstream: `Icfg::build` scans every CFG node once per call site (`crates/polint/src/analysis_neutral/ifds/mod.rs:93-98`), and the refined-calls provider scans every native call site once per Go sidecar call-site row (`crates/polint/src/analysis_neutral/refined_calls/provider.rs:333-340`, `:361-403`).
  3. Digest materialisation. Six providers still build one `String` per fact (each containing 600 B to 3 KB of resolved key text), push them all into one `Vec`, and `parts.sort()` before hashing (`crates/polint/src/analysis/provider.rs:190`, `crates/polint/src/analysis_neutral/domains/provider.rs:224`, `types/provider.rs:300`, `calls/provider.rs:186`, `refined_calls/provider.rs:657`, `data_flow/provider.rs:466`). The one provider that was fixed (evidence, `.scale-envelope/EXPERIMENTS.md` X5a) dropped 2.3 GB of transient peak. Abstract domains does the same with five resolved keys per observation row, which is why a 45-file run peaks at 5.0 GB while retaining 1.7 GB (benchmark report, appendix A.3).
  4. The abstract-domain solver is a whole-program, unbounded-call-string IDE worklist with a 10,000-iteration budget (`crates/polint/src/analysis_neutral/domains/solver.rs:61-65`, `:84`, `:146-157`, `:222`). At application scale the budget trips almost immediately, every function is marked `BudgetExceeded`, and the provider then fans out observation rows for every visible place at every block entry, block exit, operation-before and operation-after (`crates/polint/src/analysis_neutral/domains/store.rs:69-125`, `:316-380`). Its results are needed only because `calls` transitively requires `summary_call` from `polint.direct_summaries`, which lists `domain_observations` as an input (`analysis_kernel/provider.rs:1568-1598`, `:1813-1843`). The stage is 80 percent of all facts on a 45-file run and is pure fan-out of a solver that gave up.
  5. The CFG derives dominance with the naive iterative set-intersection algorithm over `BTreeSet` clones, quadratic per function in both time and space (`crates/polint/src/analysis_neutral/cfg/derived.rs:339-400`), and computes the full relation even when the materialisation budget says it will only emit the tree (`:83`, `:148`). The materialisation bound (`cfg/budget.rs:30`) already trips at 314 files (benchmark report, section 5.5.1), so at consumer scale the control-flow facts a rule would read are the tree, not the relation.
- Structural amplifier: the 23 providers run strictly sequentially in one loop over a single `&mut AnalysisDb` (`crates/polint/src/analysis_kernel/mod.rs:260-300`); the only `rayon` site in the analysis path is rule execution (`crates/polint/src/core/rule.rs:437`). Every stage rebuilds whole-program `BTreeMap<Id, String>` side tables from resolved key text (`domains/provider.rs:248-297`). Nothing is per-function, per-package, or streamed.
- Prior work already found the identity-text problem and measured that allocator tuning, lock traffic and sort comparisons are not the cost (`.scale-envelope/EXPERIMENTS.md` X1, X2 reverts; `research/deep-analysis-performance/NEXT-LEVER.md`). This document agrees and goes one step further: the composed-text identity is a design choice that no scaling fix can work around, because every quadratic above is a join on that text or a scan that exists because there is no dense index keyed by it.
- Recommendation (section 6): option D, a scoped-deep plus persisted-graph architecture built on a dense-identity graph core. Concretely: replace text identity with a structural identity (family, parent id, ordinal) that materialises canonical text only at the public boundary; make MIR lowering, CFG and local domains per-function and parallel with arena storage; persist per-package graph shards keyed by the existing content digests; join across shards on demand for the interprocedural stages. This is a re-architecture of the Rust graph stages, not a rewrite into another language, because they are already Rust and the non-Rust part is the fast part.
- Acceptance gate (section 7): a forced `calls` scan of the full 4,752-file backend completes under 300 s wall at 12 threads with peak tree RSS under 12 GB, with byte-identical `polint check` output across cold, warm and shuffled provider order, verified by the probe commands given. Intermediate gates are stated per workstream so progress is measurable before the final gate can be attempted.
- Secondary, independent, small: the rule-host store key hashes the `CARGO_HOME` path string even though the cargo config files under it are already hashed by content (`crates/polint/src/cache/rules_store.rs:797-806` versus `:827-834`, helper at `:1098-1110`). Dropping the path line turns a 193 s rule-pack compile into a 5 s store restore on every fresh-container CI run (benchmark report, section 4.2). It is workstream W0 in section 8.
- The nine questions the first draft left open were researched afterwards (Resolved Questions): eight are resolved with evidence, and one owner decision remains, how the acceptance-gate runner is provisioned, because it turns on spend and a private-repository token in a public repository's workflows rather than on anything the code can decide.
- The TypeScript type sidecar that landed as PR #121 is orthogonal: it adds a typed resolution tier feeding `polint.refined_calls` and costs 13.4 percent of pipeline wall on a 265-file repository (`research/ts-type-sidecar/measurement.md` on that branch). It does not touch `semantic_mir`, `cfg` or `abstract_domains`, so it neither causes nor fixes the wall; section 9 positions it.

## 1. Method and starting position

### 1.1 What was done

- Read the benchmark report in full and took its numbers as the only measurements in this document.
- Traced the three stages the report names as the wall (`polint.semantic_mir`, `polint.cfg`, `polint.abstract_domains`) and the stages that consume them (`polint.calls`, `polint.refined_calls`, `polint.solver`, `polint.direct_summaries`, `polint.data_flow`) through the code at `686461be`, looking for the allocation shape, the join shape, and the control structure. Line numbers were re-read after the trace.
- Read the two prior performance investigations in the repository (`.scale-envelope/EXPERIMENTS.md`, `research/deep-analysis-performance/`) so that nothing already measured is re-proposed as a hypothesis.
- Read the TS type sidecar branch (`origin/feat/ts-type-sidecar`, nine commits on top of `82a3c129`) to position it rather than re-research it.
- Ran three web-research passes for the state-of-the-art survey in section 4; every external claim there carries its URL inline and unverified claims are marked.
- Built nothing and benchmarked nothing. The benchmark report is the measurement; this document is the explanation and the plan.

### 1.2 What the benchmark report established (quoted)

| Scope (Go files) | wall | peak RSS | outcome | `semantic_mir` | `cfg` |
|---:|---:|---:|---|---:|---:|
| 45 | 68.3 s | 10,964 MB | completes | 468 ms | 1,852 ms |
| 314 | 91.2 s | 12,290 MB | completes; dominance bound tripped | 5,922 ms | 9,983 ms |
| 885 | 152.3 s | 13,689 MB | completes; dominance bound tripped | 15,432 ms | 20,783 ms |
| 1,588 | 300.6 s | 15,064 MB | timeout after `polint.identity` | 88,971 ms | 65,078 ms |
| 4,752 | 300.6 s | 14,555 MB | timeout inside `semantic_mir` | >287,000 ms | not reached |
| 4,752, 1,800 s budget | 634.6 s | 18,304 MB | SIGKILL inside `cfg` | 565,269 ms | killed after dominators |

Source: benchmark report, sections 5.3.1 and 5.6. The per-file cost of `semantic_mir` grows from 10.4 ms at 45 files to 56.0 ms at 1,588 and 119 ms at 4,752. The report is careful that the 885 to 1,588 jump may include memory pressure; section 3.7 below argues from code that the growth is algorithmic first and memory-bound second.

Two further quoted facts anchor the argument:

- At the end of `semantic_mir` on the full backend the interner held 5,525,029 keys and 3,448 MB of key text (benchmark report, section 5.6). That is 624 bytes per key on average.
- `polint.abstract_domains` on the 45-file scope: 15.3 s, +732,066 facts (80 percent of the run's facts), RSS 274 MB to 1,736 MB with a 5,024 MB peak (benchmark report, appendix A.3).

### 1.3 What the prior performance work already settled

Two investigations preceded this one and their verdicts are inputs, not hypotheses to re-test:

- `.scale-envelope/EXPERIMENTS.md` (excalidraw, 385 TS files): glibc allocator tuning moves peak by 0.2 percent (X1, rejected: "the memory is live data, not allocator retention"); sharing `Arc<str>` instead of cloning key text into evidence facts saved 1.42 GB (X2, kept); a non-interning key-text builder saved 268 MB (X3, kept); streaming the evidence digest one family at a time saved 2.3 GB of transient peak (X5a, kept); bounding the materialised dominance relation saved 0.8 GB and is a reported semantic change (X5b, kept); indexing three whole-vector scans in data flow saved 12.6 s (X6, kept). Reverted because measured as no-ops: interner lock traffic, `sort_by_cached_key` over resolved keys, `Arc<str>` in points-to fragments. Final: 8.9 GB to 5.5 GB peak, 251 s to 235 s wall.
- `research/deep-analysis-performance/FINAL-REPORT.md` (excalidraw 234 s to 78 s; hugo 313 s to 91 s, both warm medians): indexed data-flow and CFG/evidence joins, alias preparation, streamed evidence digest. Its `NEXT-LEVER.md` names the next structural lever explicitly: "composite keys include previously expanded parent keys, so interning identical complete strings does not share common substructure between distinct keys. This is structural retained memory". It proposes interned structural nodes with streamed canonical text and states the proof obligations (byte-identical canonical streams and digests).

The present document takes `NEXT-LEVER.md` as the correct diagnosis of the memory term and adds the time term: the joins that are quadratic because the only identity is text.

## 2. The wall, restated as a dependency chain

A rule that requests `calls` seeds five providers (`analysis_kernel/provider.rs:1020-1028`) and the closure over manifest inputs (`:1094-1130`) pulls the rest. The chain that matters for cost is:

```
calls
  -> polint.refined_calls        inputs include summary_call, summary_events     (provider.rs:1813-1843)
       -> polint.direct_summaries  inputs include domain_observations, domain_events (provider.rs:1568-1598)
            -> polint.abstract_domains  inputs include cfg_* and mir_*               (provider.rs:1545-1567)
                 -> polint.cfg            inputs mir_bodies, mir_operations, places, ... (provider.rs:1448-1457)
                      -> polint.semantic_mir                                            (provider.rs:1425-1447)
```

So a rule that wants "which functions can reach this call" pays for a whole-program IDE domain solve and the full CFG derived relations, because the summary provider that refines call edges lists the domain observations as an input. Whether it needs them for the `summary_call` family specifically is examined in section 3.4. The chain is the reason "turn on `calls`" costs two orders of magnitude rather than the cost of a call graph.

The benchmark report's stage split of the timed-out full-backend `calls` run (section 5.3) is the same chain seen on a clock:

```text
t=0.00 s   requested_capabilities={calls, module_graph, references, resolved_imports, symbols}, rule_scoped=false
t=0.23 s   4,752 Go files loaded (48 MB)
t=5.86 s   polint.go.syntax done        (5,837 ms, 548,817 facts)
t=10.91 s  polint.module_graph done     (4,103 ms)
t=11.74 s  polint.symbol_graph done     (822 ms)
t=13.63 s  polint.module_topology done  (1,891 ms)
t=13.63 s  polint.semantic_mir starts   -- single thread, whole program
t=138.2 s  go semantic sidecar returned on its prefetch thread (unused until refined_calls)
t=300.6 s  killed by timeout, still inside polint.semantic_mir
```

Everything before 13.6 s is linear and parallel where it matters (the syntax stage uses `rayon` across files); everything after it is one thread walking whole-program tables. The providers execute in one sequential loop (`analysis_kernel/mod.rs:260`), each taking `&mut AnalysisDb` (`:283-291`). There is no intra-stage parallelism in `analysis_neutral` or `core`: the grep for `rayon` and `par_iter` in those trees returns only `core/rule.rs:437`, which parallelises rule execution after all providers have finished. The kernel gauge logs interner key count and text bytes after every stage (`analysis_kernel/mod.rs:327-341`), which is how the benchmark report obtained the 3,448 MB figure.

## 3. Root cause: five compounding causes, with evidence

The order below is by estimated contribution to the full-backend failure, highest first. Each cause states the mechanism, the code, the scaling term, and what the benchmark report shows that is consistent with it.

### 3.1 Identity is composed text, retained for the run

**Mechanism.** A stable key is built by `write_stable_key_text` as `family|label=len:value|...` with every value length-prefixed (`crates/polint/src/analysis_api/metadata.rs:512-523`). Keys compose by embedding parent key text as a value:

- A MIR body key embeds the owner function key, which embeds the file path, function name and span (`go/mir/lower.rs:629-641`, `:2335-2347`).
- An operation key embeds the full body key plus up to four place keys (`go/mir/lower.rs:2238-2260`).
- A place key embeds the file key, the function key, and for temporaries the body key (`crates/polint/src/analysis_neutral/places.rs:135-196`).
- A statement key is the operation key plus `:statement` (`go/mir/lower.rs:150`).
- A dominator key embeds the function key and both block keys (`cfg/derived.rs:96-104`); `cfg/budget.rs:1-19` records this as "roughly 1.4 KB of interned identity text per pair".
- A domain observation key embeds the source key (a body, block or operation key) and the place key (`domains/store.rs:499-509`).

Each key text is interned exactly once, but interning is by whole string: two operation keys in the same body share nothing (`internal_core/stable_key.rs:37-41`, `:62-73`). The interner is one `Arc<RwLock<...>>` whose doc comment states that ids are dense insertion indexes and that "splitting this state across shards would break it" (`:24-36`). `intern_and_resolve`, used by every metadata construction, takes the write lock even for an existing key (`:78-88`).

Every fact additionally gets a `FactMeta` row holding a `payload_digest: String` (`analysis_api/metadata.rs:231-239`), built by formatting the resolved key text and extra parts and hashing them (`core/metadata.rs:301-321`, `:380-398`). The rows live in `FactMetaStore` with a per-family `HashMap<StableKeyId, StableKeyOwner>` that clones the digest string again for conflict detection (`analysis_api/metadata.rs:353-357`, `:360-392`). For MIR the metadata is rebuilt in `refresh_semantic_mir_metadata` after every replace, formatting `function_key`, `owner_stable_key`, `file_key` and span strings per body and per operation just to hash them (`core/db.rs:2706-2750`, `:4578-4640`).

**Scaling term.** Linear in facts, with a large constant: 624 B of key text per key measured (benchmark report, section 5.6), plus roughly 40 B of `Arc<str>` and map overhead, plus a `FactMeta` of about 80 B with a 16-hex-character heap string. At 5.5 million keys and about 2.9 million metadata rows (benchmark report, section 5.6, "facts at the kill 2,926,814") that is 3.4 GB of text plus on the order of 0.5 GB of structure before any fact payload is counted. It is the floor under every other cost and it is why the process could not survive the CFG stage after `semantic_mir` completed.

**Why it is a design defect, not a tuning problem.** `NEXT-LEVER.md` already showed the same shape on excalidraw (2.5 million keys, 3,085 MiB text for 2.5 MB of source). The X2 experiment then showed that lock traffic and sort comparisons are not where the time goes. What remains is that text identity forces every consumer that needs to join two fact families to either resolve and compare strings or build a `BTreeMap<Id, String>` side table first, which is exactly what sections 3.2 and 3.3 find.

### 3.2 Quadratic joins in lowering and in the consumers of lowering

**Mechanism.** The lowerers and several consumers look things up by scanning a whole-program `Vec` instead of an index. Each site below is a loop nested inside a loop over a program-sized collection.

| Site | Outer loop | Inner scan | Term |
|---|---|---|---|
| `matching_function` (`go/mir/lower.rs:2317-2329`; TS twin `ts/mir/lower.rs:4540-4553`, plus `enclosing_function` `:4568-4581`) | every function declaration in every file (`go/mir/lower.rs:532-540`) | `db.functions().iter().find(...)` over every function in the program | O(F²) in functions |
| `push_body` package and module lookup (`go/mir/lower.rs:645-656`) | every body, including every closure body (`:543`, `:555`) | `db.packages().iter().find(...)` and `db.module_nodes().iter().find(...)` | O(B × P) bodies times packages |
| `go_closure_capture_names` (`go/mir/lower.rs:732-767`; TS `ts/mir/lower.rs:764`) | every closure literal | `references_for_file(file)`, which is the trait default that filters every reference in the database (`analysis_neutral/host.rs:129-138`), then `definition_for_symbol` per reference, which filters every definition (`:141-152`) | O(C × R) closures times references, times definitions |
| `lower_control_flow` (`go/mir/lower.rs:115-124`; TS `ts/mir/lower.rs:132-141`) | every body | `operations.iter().filter(op.body == body.id)` and the same over `control_effects` | O(B × Ops) bodies times operations |
| `Icfg::build` (`ifds/mod.rs:93-98`) | every call site | `cfg_nodes.iter().find(node.operation == site.operation)` | O(S × N) sites times CFG nodes; runs in `abstract_domains` (`domains/solver.rs:119`) and in data flow |
| refined-calls Go semantic join (`refined_calls/provider.rs:333-340` calling `:361-403`) | every Go sidecar call-site row (286,671 to 362,952 rows on the consumer backend, benchmark report sections 5.3.2 and A.4) | `db.call_sites().iter().filter(...)` over every native call site, then `db.functions().iter().filter(...)` per row (`:426-454`) | O(Rows × S) |
| `owner_symbol` (`calls/extract.rs:554-567`) | every call site | `db.symbols().iter().find(...)` | O(S × Sym) |

The indexed alternative already exists for two of these and is not reachable from the lowerers: `AnalysisDb::references_for_file` uses a `DenseFileIndex` (`core/db.rs:157`, `:3746-3753`), but the `impl AnalysisHost for AnalysisDb` block (`core/db.rs:6043`) does not override the trait defaults, and the lowerers are generic over `impl AnalysisHost` (`go/mir/lower.rs:28`), so they get the linear filter. The CFG lowerer, by contrast, indexes its inputs once per run (`cfg/lower.rs:40-97`); it is the model the others should follow.

**Scaling term.** Between 885 and 1,588 files the scope grows 1.8 times and `semantic_mir` grows 5.8 times (benchmark report, section 5.3.1). A pure quadratic would give 3.2 times; a quadratic plus memory pressure gives more; a linear cost gives 1.8. The report's own caveat about memory pressure is right, but the code says the quadratic is there independently of it. The `lower_control_flow` term is the largest by count: it touches every operation in the program once per body, and operations are the largest MIR family (291,243 MIR facts at 885 files, benchmark report A.4, most of them operations, statements and places).

**Why it is invisible on small repositories.** At 45 files every scan is over a few thousand rows and completes in under half a second (468 ms for the whole stage). The fixtures and examples in this repository are smaller still. The quadratic term only dominates past roughly a thousand files, which is exactly where the benchmark report's curve bends.

### 3.3 Digest materialisation: one string per fact, sorted

**Mechanism.** A provider's output digest must be order-independent. Six providers achieve that by formatting every fact into a `String` that includes the resolved key text and often several resolved parent keys, collecting all of them into a `Vec<String>`, and sorting it:

- `semantic_mir`: one string per body, block, statement, terminator, place, operation and unsupported row (`analysis/provider.rs:85-190`, sort at `:190`).
- `abstract_domains`: one string per observation containing the observation key plus the resolved body, block, operation and place keys, after first building four whole-program `BTreeMap<Id, String>` side tables of resolved key text (`domains/provider.rs:120-224`, sort at `:224`, side tables at `:248-297`).
- `type_value_alias` (`types/provider.rs:300`), `calls` (`calls/provider.rs:186`), `refined_calls` (`refined_calls/provider.rs:657`), `data_flow` (`data_flow/provider.rs:466`).

The evidence provider was converted to stream one family at a time in X5a (`evidence/provider.rs:570-649`) and dropped from +2,306 MB to +1,162 MB of transient peak on excalidraw (`.scale-envelope/EXPERIMENTS.md` X5). The CFG provider uses the streaming `DigestBuilder` and never materialises (`cfg/provider.rs:155-330`).

**Scaling term.** Linear in facts with a constant of one to three kilobytes per fact, transient. At 732,066 observations on the 45-file scope, five resolved keys of about 600 B each per row is on the order of 2 GB of transient strings, which matches the 5.0 GB peak against 1.7 GB retained in that stage (benchmark report, appendix A.3). At 4,752 files `semantic_mir` alone holds about 2.9 million rows at the digest step; the +10.4 GB the stage retained includes the interner growth, and the transient on top of it is what the digest adds. The X5a fix is mechanical and identity-preserving, so this cause is the cheapest to remove, but removing it alone does not change the retained floor of section 3.1.

### 3.4 The abstract-domain solver gives up immediately and then fans out

**Mechanism.** `IdeDomainSolver::solve_with_output_mode` builds a whole-program ICFG (`domains/solver.rs:119`), enqueues the entry node of every function (`:134-143`), and runs one worklist over `ExplodedPoint { node, call_stack: Vec<CallSiteId> }` (`:61-65`), cloning the call stack on every edge (`:222`) and pushing on every resolved call (`:247`). The call string is unbounded. The worklist stops after `max_iterations: 10_000` (`:84`, checked at `:146-157`), at which point `mark_ide_budget_exceeded` marks every state and every function status as budget-exceeded (`:662-690`).

The consumer application's forced runs confirm the budget trips: both completed `calls` probes report a budget-exhaustion row (benchmark report, section 5.5.1). Ten thousand node visits is a few hundred functions' worth of straight-line code; a 45-file scope already exceeds it.

After the solve, `materialize_results` iterates every function and, for each, iterates every exploded state to find the entry state (`:700-704`), an O(F × States) nested loop. Then `DomainOutput::from_results_with_place_filter` emits observation rows for every place in every domain slot (nilness, truthiness, constants, strings, initializedness, reachability) at every function entry, block entry, block exit, operation-before and operation-after (`domains/store.rs:69-125`, `:316-380`). The provider then builds four whole-program `BTreeMap<Id, String>` key tables and the sorted digest of section 3.3 (`domains/provider.rs:85-96`, `:120-224`).

**Why it runs at all.** `polint.direct_summaries` lists `domain_observations` and `domain_events` as inputs (`analysis_kernel/provider.rs:1583-1584`), and `polint.refined_calls` lists `summary_call` and `summary_events` (`:1823-1824`). `polint.type_value_alias` lists `domain_observations` too (`:1688`). The capability closure therefore pulls `abstract_domains` into every `calls` run. The summaries builder shows how little of that is needed: it groups observations by body (`crates/polint/src/analysis_neutral/summaries/builder.rs:75-81`) and passes them only to `build_control_effects` (`:133-147`, `:347`, `:422`), which produces the `summary_control` family; `summary_call`, the family `refined_calls` reads, does not consult them. `type_value_alias` declares the family as an input and never reads it (no reference to `abstract_domain_observations` under `analysis_neutral/types/`). The closure does not distinguish between "this provider produces five families and one of them needs domains" and "the family you asked for needs domains", so a `calls` rule pays for a solver whose output reaches only a family it never reads. A demand-driven closure at family granularity fixes this without changing any summary; see workstream W3.

**Scaling term.** The fan-out is linear in (program points × visible places × slots), which is itself superlinear in file count because visible places per function grow with function size and closure count. 732,066 facts for 45 files is 16,000 facts per file; 280,617 for 885 files is 317 per file (benchmark report, A.3 versus A.4), so the fan-out is already being truncated by the budget at 885 files. The output is therefore both enormous and almost entirely `BudgetExceeded`. Nothing in the consumer application would have read a correct value from it.

### 3.5 CFG derived relations: naive dominators, computed even when not emitted

**Mechanism.** `derive_dominators` and `derive_postdominators` compute the dominance relation per function with the textbook iterative data-flow formulation over `BTreeMap<BasicBlockId, BTreeSet<BasicBlockId>>`: every block starts as the full universe (`cfg/derived.rs:346-357`, note `universe.clone()` at `:352`), and each pass intersects predecessor sets until no change (`:359-400`). That is O(N²) space per function before the loop starts and O(N² × passes) time with allocation on every changed set. The relation is then walked to derive immediate dominators (`:438-462`) and to emit one fact per pair (`:85-110`), each with a composed key.

When the worst-case pair estimate exceeds `DEFAULT_MAX_DOMINANCE_PAIRS` (250,000; `cfg/budget.rs:30`), emission is restricted to tree edges (`cfg/provider.rs:109-116`), but the relation is still computed in full (`derived.rs:83`, `:148`). The bound trips at 314 consumer files with a worst case of 1.6 million pairs and at 885 files with 4.1 million (benchmark report, section 5.5.1).

**Scaling term.** Time is quadratic in blocks per function, summed over functions, so it is linear in files with a large per-function constant for large functions; memory is the same but transient. The report's full-backend timeline shows `lower_normalize` 30.2 s, `reachability` 5.4 s, `dominators` 11.2 s, then the kill (benchmark report, section 5.6). The retained cost of the CFG stage is the derived-row identity text of section 3.1 (+881,253 facts at 885 files, A.4), not the algorithm; the algorithm is the wall-clock term. Cooper, Harvey and Kennedy's reverse-postorder algorithm with an `idom: Vec<u32>` array (section 4) is near-linear in practice and allocates nothing per iteration; it produces the tree directly, which is all the bound ever emits anyway.

**Consequence for rules.** `docs/facts/control-flow.md:120-122` describes guard policies in terms of "the guard call's basic block dominates the operation's block". At consumer scale the `cfg_dominators` family carries tree edges only, and the relation is their transitive closure. A policy that reads the family as a relation is wrong past 314 files unless it closes the tree itself. This is not a scale bug; it is a contract note that the scale plan must preserve.

### 3.6 Structural amplifiers

Three properties of the kernel multiply every cause above.

1. **Sequential, monolithic stages.** `run_scheduled_providers` runs providers one at a time (`analysis_kernel/mod.rs:260`), each over the whole program, each holding `&mut AnalysisDb`. `lower_go_mir` re-parses every Go file with tree-sitter (`go/mir/lower.rs:500-507`), lowers every file in path order in one thread (`:39-41`), then runs `lower_control_flow` over all bodies (`:79-87`). A 16-core host observed at most 8.12 busy cores across all phases and typically far fewer (benchmark report, section 2).
2. **Whole-program normalisation, twice.** `lower_go_mir` returns `.normalized(interner)` (`go/mir/lower.rs:99`), which sorts every table by resolved key text (`crates/polint/src/ir/body.rs:123-160`); `SemanticStore::from_output` calls `normalized` again (`analysis_neutral/store.rs:38`) and then sorts each table a third time inside `normalize_bodies`, `normalize_places`, `normalize_operations` and friends (`store.rs:192`, `:207`, `:225`, `:263`, `:285`, `:308`, `:420`). Each sort resolves the key through the interner lock per element (cached), and the id remap tables are `BTreeMap<Id, Id>` per family (`:191-235`). X2 measured that the sort comparisons are not the wall-clock term; they are, however, the reason every provider needs a global sorted order, which in turn is why nothing can be per-function.
3. **Side tables of owned text.** Consumers that need to join by identity build `BTreeMap<Id, String>` tables by resolving every key (`domains/provider.rs:248-297`; `refined_calls/provider.rs:311-330`; `go/mir/lower.rs:44-47`). Each such table is a second copy of the family's key text, and each is rebuilt per provider.

### 3.7 What is not the problem

- **The Go sidecar.** Flat 25 to 27 s at every scope, prefetched at `go.syntax` digest time and overlapped (`go/semantic/prefetch.rs:1-23`; benchmark report, section 5.3.2 and the "sidecar is effectively free" note). Its own RTA is compiled out in release builds because the only reader is `#[cfg(test)]` (`crates/polint/src/go/lifecycle.rs:29-37`, `:163-166`; `go/semantic/client.rs:206`); the live RTA is the Rust `polint.solver` policy, which cost 146 ms on the 45-file scope (benchmark report, bullet 2).
- **Parsing.** `polint.go.syntax` on all 4,752 files is 5.8 s; the whole upstream pipeline through `module_topology` is 13.6 s (benchmark report, section 5.3).
- **The allocator.** X1 measured 0.2 percent.
- **Interner lock contention and sort comparisons.** X2 reverts measured no change; the interner is single-threaded in practice because the stages are.
- **The rule host and store.** The 193 s cold cost is the rule-pack compile and is addressed separately (section 8, W0). Warm production scans are 4.4 s.

### 3.8 A cost model for the full backend (hypothesis)

This is inference from the code and the measured points, labeled as such. Let F be files, B bodies, Ops operations, S call sites, R references. On the consumer backend the syntax stage emits 548,817 facts for 4,752 files (benchmark report, A.5), which gives a rough 115 syntax facts per file and, by the 885-file ratio (291,243 MIR facts for 86,667 syntax facts), roughly 1.8 million MIR facts and 2.9 million total at the point `semantic_mir` finished (measured 2,926,814, benchmark report section 5.6). With those magnitudes:

| Term | Cause | Estimated order at 4,752 files |
|---|---|---|
| retained key text | 3.1 | 3.4 GB measured; grows with facts, so roughly 5 to 7 GB by the end of `cfg` if it had finished |
| `lower_control_flow` | 3.2 | B × Ops; with B in the tens of thousands and Ops above a million, on the order of 10^10 to 10^11 comparisons |
| `matching_function` | 3.2 | F_decl × F; tens of thousands squared, on the order of 10^9 |
| closure capture scans | 3.2 | C × R; closures in the thousands times references in the hundreds of thousands, on the order of 10^9 |
| `semantic_mir` digest | 3.3 | 2.9 million strings of 0.6 to 2 KB, sorted; 2 to 5 GB transient |
| CFG dominators | 3.5 | quadratic per function; the measured 11 s was for the bounded path |
| domains fan-out | 3.4 | not reached; would be the largest fact family and the largest transient by the 45-file ratio |

The measured 565 s for `semantic_mir` is consistent with the B × Ops term dominating: it is the largest of the superlinear terms in that stage (the others in the section 3.2 table, `matching_function` at O(F²), `push_body` at O(B × P) and the closure-capture scans at O(C × R), are bounded by function, body and closure counts rather than by operations) and it is touched for every body. Their relative shares are a prediction from the loop shapes, not a measurement; G1b measures them. The 1,588-file run's 89 s and the 885-file run's 15 s give a ratio of 5.8 for a size ratio of 1.8, which is between quadratic (3.2) and cubic (5.8) and includes swapping; the 4,752-file run's 565 s against 1,588's 89 s is a ratio of 6.3 for a size ratio of 3.0, which is below quadratic (9.0), consistent with a quadratic term plus a large linear term. No number in this table is a measurement; the probes in section 7 are how to confirm the split.

### 3.9 Per-stage attribution at the practical ceiling

The 885-file run is the largest scope that completes, so its stage table (benchmark report, appendix A.4) is the best available map of where the remaining wall goes once the two headline stages are fixed. Stages under one second are omitted.

| Stage | ms | retained after (MB) | facts added | Cause it exhibits |
|---|---:|---:|---:|---|
| `polint.module_graph` | 3,096 | 147 | +15,925 | not traced; small |
| `polint.semantic_mir` | 15,432 | 1,555 | +291,243 | 3.1, 3.2, 3.3, 3.6 |
| `polint.cfg` | 20,783 | 2,858 | +881,253 | 3.1 (pair keys), 3.5 |
| `polint.calls` | 4,346 | 3,163 | +117,263 | 3.2 (`owner_symbol`), 3.3 |
| `polint.go.semantic` | 19,069 | 3,465 | +0 | overlapped sidecar wait plus Rust-side row lowering and validation; not traced here |
| `polint.identity` | 1,263 | 3,467 | +64,808 | not traced |
| `polint.abstract_domains` | 15,080 | 4,048 | +280,617 | 3.4, 3.3 |
| `polint.direct_summaries` | 1,199 | 4,048 | +41,797 | consumer of 3.4 |
| `polint.type_value_alias` | 19,100 | 7,070 | +684,576 | 3.1, 3.3 (`types/provider.rs:300`); 70 percent of hugo's wall in `.scale-envelope/EXPERIMENTS.md`; not root-caused in this document |
| `polint.semantic_graph` | 9,940 | 7,083 | +1 | mints 435,051 keys for one fact on excalidraw (X2); 3.1 |
| `polint.solver` | 2,777 | 7,086 | +59,090 | the live RTA; cheap |
| `polint.refined_calls` | 25,984 | 7,113 | +81,125 | 3.2 (Go semantic join), 3.3 |
| total of the above | 138,089 | | | of a 152,300 ms run |

Three observations follow. First, the four stages this document root-causes in depth (`semantic_mir`, `cfg`, `abstract_domains`, `refined_calls`) are 77 s of the 152; `type_value_alias` and `semantic_graph` are another 29 s and share causes 3.1 and 3.3 but were not traced line by line here, which is recorded in section 11. Second, retained memory grows by 1.4 GB at `semantic_mir`, 1.3 GB at `cfg`, and 3.0 GB at `type_value_alias`; the last is the largest single owner and is the stage `NEXT-LEVER.md` also flagged. Third, at 885 files no single stage dominates; the full-backend failure is the product of the superlinear terms in 3.2 with the linear-but-large constants in 3.1, which is why W1 and W5 are both necessary and neither is sufficient.

### 3.10 Why "rewrite in Rust" is the wrong framing, and what the right one is

The owner's framing allows rewriting anything into Rust. The layers that fail are already Rust; the part that is not Rust is the fast part. The problem is not the language, it is that the Rust graph layer was designed as a fact pipeline for small repositories: every fact is a row with a text identity and a metadata row, every stage is a whole-program pass that produces a globally sorted table, and the only index is the interner. That design gives excellent determinism and honesty properties (byte-identical output, per-fact precision, replayable evidence), which the gap analysis correctly counts as the moat. The re-architecture must keep those properties and change the representation underneath them. Section 5 sets out how, and section 6 chooses.

## 4. Survey: how engines that scale to monorepos avoid these five causes

Three web-research passes were run for this section (fact stores; incremental and demand-driven engines; graph representations and pointer-analysis scaling). Each subsection states what the primary source says, with the URL inline, then the idea that transfers to polint and what it would cost against the provider and fact model. Claims that could not be confirmed against a primary source are marked unverified. Access date for every URL: 2026-09-19.

### 4.1 Extraction separated from evaluation: CodeQL, Kythe, SCIP

**What the sources say.** CodeQL's extractor "produces the relational data and source reference for each input file" as TRAP files, and a `.dbscheme` describes "the column types and extensional relations that make up a raw QL dataset" (https://codeql.github.com/docs/codeql-overview/codeql-glossary/); import is a separate replayable step, `codeql dataset import`, that can "add data from TRAP files to an existing dataset" when "the correct dbscheme and its ID pool has been preserved" (https://docs.github.com/en/code-security/codeql-cli/codeql-cli-manual/dataset-import). The ECOOP 2016 QL paper states that QL "compiles to Datalog and runs on a standard relational database" and that "in practice, we use our own custom database system" (https://drops.dagstuhl.de/storage/00lipics/lipics-vol056-ecoop2016/LIPIcs.ECOOP.2016.2/LIPIcs.ECOOP.2016.2.pdf). The evaluator is bottom-up relational algebra with "special data structures for dealing with transitive closures" (https://codeql.github.com/docs/writing-codeql-queries/troubleshooting-query-performance/), a bounded disk cache for intermediate results (`--max-disk-cache`, `--min-disk-free`) and an explicit RAM budget (`--ram`, "the query evaluator will try hard to keep its total memory footprint below this value") (https://docs.github.com/en/code-security/codeql-cli/codeql-cli-manual/database-run-queries). Hardware guidance is 16 GB for 100K to 1M lines and "64 GB or higher" above that (https://docs.github.com/en/code-security/code-scanning/creating-an-advanced-setup-for-code-scanning/recommended-hardware-resources-for-running-codeql).

Kythe's unit is the entry: source VName, edge kind, target, fact label, value; "a node is a collection of entries in the store that share a source, kind, and target"; stores combine by set union and `write_tables` denormalises a graph store into a serving table (https://kythe.io/docs/kythe-storage.html, https://kythe.io/examples/). The VName signature "should be consistently generated given the same input to the indexer" (https://kythe.io/docs/schema/).

SCIP is "meant to be a transmission format" whose protobuf "enables streaming reads and writes as well as merging by concatenation", with string symbols because "string types in mainstream languages support equality and hashing" and to bound the blast radius of indexer bugs (https://github.com/sourcegraph/scip/blob/main/docs/DESIGN.md); Sourcegraph reports LSIF payloads "on average 4x larger when gzip compressed" than SCIP (https://sourcegraph.com/blog/announcing-scip).

**Transferable.** Every one of these separates a per-file or per-unit extraction artifact from evaluation state, and keys it by a content-derived name rather than a run-local id. polint has the artifact boundary already for the sidecars (`.polint/cache/sidecar`, the TS NDJSON) but not for its own lowered graph; W7's unit shard is that artifact. CodeQL is also the one system that documents a disk-backed intermediate cache with a RAM ceiling, and it pays for it with a batch positioning (64 GB hosts for large repos) that polint's diff-time positioning cannot adopt; this argues against option C as the in-run representation.

### 4.2 Fact stores with numeric identity and ownership: Glean, Infer, Souffle

**What the sources say.** Glean facts "are immutable terms described by user-defined schemas, and form a DAG", "automatically de-duplicated by the storage backend" on RocksDB (https://glean.software/docs/introduction/); each fact has "a unique 64-bit integer" id and a key, and "each fact is stored only once" (https://glean.software/docs/schema/basic/). Incrementality is by ownership: facts belong to units, a slice is a bitmap over ownership sets, and the invariant "every fact referenced by a visible fact is also visible" is maintained in "O(facts) time" (https://glean.software/docs/implementation/incrementality/); ownership metadata "only adds about 7% to the DB size" and query overhead is "less than 10% for typical queries" (https://glean.software/blog/incremental/). Stacked databases let each layer "non-destructively add information to, or hide information from, the layers below" (https://engineering.fb.com/2024/12/19/developer-tools/glean-open-source-code-indexing/).

Infer keeps per-procedure summaries as blobs in an SQLite database in WAL mode, with `procedures`, `source_files` and `specs` tables (https://github.com/facebook/infer/blob/main/infer/src/base/Database.ml), analyses "each function and method separately" (https://fbinfer.com/docs/infer-workflow), and offers `--reactive` and `--incremental-analysis` so that "only the modified files/procedures and their dependencies" are re-analysed, with `--replay-analysis-schedule` to "drastically limit non-determinism" (https://github.com/facebook/infer/blob/main/infer/man/man1/infer-analyze.txt). The compositional argument is stated in the CACM paper: "a compositional analysis can even have a runtime that is (modulo mutual recursion) a linear combination of the times to analyze the individual procedures" and is "naturally incremental" (https://discovery.ucl.ac.uk/id/eprint/10084236/1/O'Hearn%20AAM%20scaling-static-analysis-at-facebook.pdf).

Souffle compiles Datalog to a relational algebra machine and then to C++ with OpenMP, stores relations in "efficient parallel variations of B-trees and Tries", and selects indexes by a Dilworth-based minimal cover (https://souffle-lang.github.io/pdf/cav16.pdf); the data-structure choice is per relation, B-tree by default, brie for dense low-arity relations, union-find for equivalence relations (https://souffle-lang.github.io/relations). Doop on LogicBlox stored "domain values as integers" and found explicit relations "outperform BDDs by an order of magnitude" (https://yanniss.github.io/doop-oopsla09prelim.pdf). The CAV 2016 numbers for OpenJDK points-to are 35 s and 8.5 GB in Souffle against 30 min in bddbddb and over 6 h in SQLite (https://souffle-lang.github.io/pdf/cav16.pdf).

**Transferable.** Three ideas, all of which W5 to W8 adopt: identity is an integer and the key is a structured term, never a composed string (Glean, Doop); facts carry an owner unit so incremental re-scan is "hide these units, append a layer" (Glean's 7 percent overhead is the budget to beat); per-relation indexes chosen from access patterns rather than one generic map (Souffle). Infer's schedule replay is the model for W6's deterministic merge order. The cost against polint's model is the `AnalysisHost` accessor shape (crate-private, so free to change: Q2) and the `FactMetaStore`, which today is the only index and is keyed by the text identity.

### 4.3 In-memory graph layout: Joern's move from OverflowDB to flatgraph

**What the sources say.** OverflowDB was "an in-memory graph database with low memory footprint" that "overflows to disk when running out of heap space" (https://github.com/ShiftLeftSecurity/overflowdb). Joern 4 replaced it with flatgraph, "an efficient columnar layout, essentially we hold everything in few (albeit very large) arrays", and dropped overflow-to-disk because "in practice it was too slow to be useful" (https://github.com/joernio/joern/blob/master/changelog/4.0.0-flatgraph.md); flatgraph states "your graph will need to fit into the heap at all times" and that applying a DiffGraph costs "almost independent of its size" (https://flatgraph.joern.io/). Reported effect on the Linux kernel CPG: heap after import 33 GB to 20 GB, on-disk 2,600 MB to 400 MB (changelog above); a 9 million node, 84 million edge graph is 3.7 GB in memory and 300 MB on disk (https://flatgraph.joern.io/).

**Transferable.** A struct-of-arrays, dense-id layout is the representation the section 3 causes call for, and the Joern history is a warning against the opposite shortcut: a transparent spill-to-disk layer was tried by a mature project and removed. polint's unit shard should therefore be an explicit, compact serialised form of an arena graph, not paging.

### 4.4 Incremental query engines: Salsa, rust-analyzer, rustc

**What the sources say.** Salsa is "a Rust framework for writing incremental, on-demand programs" whose interned structs guarantee that equal field values "get back the same integer id" (https://salsa-rs.github.io/salsa/overview.html); red-green revalidation records "what other tracked functions it depends on, and the revisions when their value last changed" and can "backdate the result" when inputs changed but the output is equal (https://salsa-rs.github.io/salsa/reference/algorithm.html); durability lets it skip traversing dependencies of high-durability inputs (https://salsa-rs.github.io/salsa/reference/durability.html). Memory is managed by per-query LRU: "with `lru = 128`, Salsa will keep at most 128 memoized values", evicting values but not dependency metadata, and "input identities remain until the database is dropped" (https://salsa-rs.github.io/salsa/tuning.html); the LRU RFC records that after opening a project "the number of values within a single revision can be high" and that LRU trades guaranteed recomputation for a bounded working set (https://github.com/salsa-rs/salsa-rfcs/blob/master/RFC0004-LRU.md). Serialization to disk has been an open issue since 2018 (https://github.com/salsa-rs/salsa/issues/10). rust-analyzer keeps all inputs in memory, builds syntax trees per file, and arranges that "typing inside a function's body never invalidates global derived data" (https://github.com/rust-lang/rust-analyzer/blob/master/docs/book/src/contributing/architecture.md).

rustc's query system memoizes per `(query, key)`, records the dependency graph at runtime, and colours nodes red or green so that "if all the inputs to query Q are colored green, then the query Q must result in the same value as last time" (https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation.html); nodes are identified by "a fingerprint of the query key", results can be cached on disk per query, and "computing fingerprints is quite costly. It is the main reason why incremental compilation can be slower than non-incremental compilation" (https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation-in-detail.html). MIR is a per-body query result: "the main MIR data type is `Body`. It contains the data for a single function" (https://rustc-dev-guide.rust-lang.org/mir/index.html), built by the `mir_built` query (https://rustc-dev-guide.rust-lang.org/mir/construction.html). Types live in a long-lived arena and interned values compare by pointer (https://rustc-dev-guide.rust-lang.org/memory.html).

**Transferable.** The unit of memoisation must be small and cheap to rebuild before a query engine helps; rustc's per-body MIR and rust-analyzer's per-file trees are that unit, and polint's whole-program `MirOutput` is not. That is why option B follows W6 rather than preceding it. Two specific ideas transfer now: fingerprint keys per unit so unchanged units are green without loading (W7), and an LRU over expensive per-unit results so peak memory is a capacity, not a program size (W6). Salsa's lack of persistence and its memo retention are the reasons it is a scheduling layer inside D, not D itself.

### 4.5 Per-package modular analysis: gopls and go/analysis

**What the sources say.** gopls v0.11 "held all these symbols in memory, as though gopls was compiling your entire program at once", with typed syntax trees "typically 30x larger than the source text"; v0.12 "brings separate compilation to gopls, reusing the same package summary format used by the compiler", persisted in "a file-based cache store" of per-package summaries, and "gopls's memory use is proportional to the number of open packages and their direct imports", with savings that "average around 75%" across 28 repositories (https://go.dev/blog/gopls-scalability). The analysis driver's cache key is the hash of dependencies' export data and facts plus the package's metadata and file contents, and facts "must be serializable, so that there is no reliance on types.Object identity across packages" (https://groups.google.com/g/golang-codereviews/c/oYyD_9AG0hs). The go/analysis framework defines a modular analysis as one that "inspects one package at a time but can save information from a lower-level package and use it when inspecting a higher-level package", with the driver ensuring "that facts for a pass's dependencies are generated before analyzing the package" (https://pkg.go.dev/golang.org/x/tools/go/analysis).

**Transferable.** This is option D for Go, stated by the Go team about their own tool. The unit is the package; the persisted artifact is the summary plus serialisable facts; dependencies are processed first; memory tracks the frontier. polint's Go sidecar already loads whole-program packages in 25 s and could emit per-package rows in import order; W6 and W7 make the Rust side match. The cost is that `polint.symbol_graph` and `module_topology` must provide the package DAG to the scheduler, which they already compute.

### 4.6 Demand-driven analysis and the memory behaviour of IFDS and IDE

**What the sources say.** Demand-driven pointer analysis performs "just enough computation to determine the points-to sets for these query variables" (https://dl.acm.org/doi/10.1145/378795.378802); refinement-based analysis spends "extra effort on analyzing these necessary method invocations under a greater number of contexts" only for the query's slice (https://dl.acm.org/doi/10.1145/1133981.1134027); Boomerang argues for analyses that "compute information only where required" (https://drops.dagstuhl.de/opus/volltexte/2016/6116); synchronized pushdown systems shift complexity to "|S||F|^2, where F is the set of fields and S the set of statements involved in a data-flow" (https://dl.acm.org/doi/10.1145/3290361).

For IFDS, Naeem, Lhotak and Rodriguez state that the exploded supergraph's node count "is approximately the product of the size of D and the number of instructions in the program", that "the complete supergraph contains 2081 times as many nodes as the reachable part", and that "constructing the supergraph on demand rather than exhaustively is key to analyzing benchmarks of this size in reasonable time and memory bounds" (https://plg.uwaterloo.ca/~olhotak/pubs/cc10.pdf). The ECOOP 2024 work on scaling IDE reports that state-of-the-art implementations "start to run into scalability issues for programs with several thousands of lines of code", that "the high memory requirements are often impossible to solve due to hardware limits", and that garbage-collecting intermediate jump functions "speeds up the analysis on average by up to 7×, while also reducing memory consumption by 7× on average" (https://drops.dagstuhl.de/entities/document/10.4230/LIPIcs.ECOOP.2024.36). A bottom-up IFDS formulation that runs "local sub-analyses in isolation" per procedure reports "51% of the memory compared to SparseIFDS" (https://drops.dagstuhl.de/entities/document/10.4230/LIPIcs.ECOOP.2026.23). PhASAR mitigates context cost "through summaries that are computed ... reused for subsequent calls to an already summarized function" and evaluated on a 128 GB machine (https://sse.cs.tu-dortmund.de/storages/sse-cs/r/Publications/Preprints/shb19-phasar.pdf). The classic O(ED³) bound from Reps, Horwitz and Sagiv is widely cited but the paper body could not be extracted in this session, so it is unverified here (https://research.cs.wisc.edu/wpis/papers/popl95.pdf).

**Transferable.** polint's abstract-domain solver is exactly the exhaustively constructed exploded supergraph these papers warn against, with an unbounded call string on top (section 3.4), and its observation fan-out is a materialised product of program points and facts. W3's per-function default and the summaries-at-unit-boundary shape of W8 are the literature's answer; report 03's Stage 1 tabulation item should land on the unit ICFG.

### 4.7 Compiler practice: per-function passes and arenas

**What the sources say.** LLVM's `FunctionPass` may not "inspect or modify a Function other than the one currently being processed" nor "maintain state across invocations of runOnFunction"; the pass manager runs all function passes on one function before moving to the next so that "only one DominatorSet needs to be calculated at a time" (https://llvm.org/docs/WritingAnLLVMPass.html).

**Transferable.** Section 3.6's whole-program stages are the module-pass shape; W6 is the function-pass shape with a per-unit arena.

### 4.8 Dense identity and arenas: petgraph, la-arena, rustc, CodeQL's dbscheme

**What the sources say.** petgraph's `Graph` is an adjacency list with `u32` indices by default and O(1) `add_node` and `add_edge` (https://docs.rs/petgraph/latest/petgraph/graph/struct.Graph.html); its `Csr` is "a sparse adjacency matrix graph" with O(|V|+|E|) space, O(1) out-degree and neighbour slices, requiring sorted unique edges and offering no single-edge removal (https://docs.rs/petgraph/latest/petgraph/csr/struct.Csr.html). la-arena, used by rust-analyzer, is "yet another index-based arena" whose `Idx<T>` wraps a `u32` and whose `ArenaMap` has "space requirement O(highest index)" (https://docs.rs/la-arena/latest/la_arena/); id-arena returns ids rather than references and deliberately has no deletion "to maintain implementation simplicity and allocation speed" (https://docs.rs/id-arena/latest/id_arena/). typed-index-collections wraps `Vec` so that struct-of-arrays side tables are index-checked at compile time (https://docs.rs/typed-index-collections/latest/typed_index_collections/). The newtype-index rationale is stated by rust-analyzer's author: for graph-shaped data, "arena with integer indices" avoids reference overhead and borrow-checker conflicts (https://matklad.github.io/2018/06/04/newtype-index-pattern.html). rustc arena-allocates and interns so that equality "can just compare pointers" (https://rustc-dev-guide.rust-lang.org/memory.html).

CodeQL's Go dbscheme defines every entity as an integer id with a type tag, for example `exprs(unique int id: @expr, int kind: int ref, int parent: @exprparent ref, int idx: int ref)`, so an expression's identity is the tuple (parent, index), not a composed path string (https://github.com/github/codeql/blob/main/go/ql/lib/go.dbscheme). Glean's facts refer to other facts by 64-bit id and "form a DAG" with the storage backend de-duplicating (https://glean.software/docs/introduction/, https://glean.software/docs/schema/basic/).

**Transferable.** This is W5 and W6 stated by their reference implementations: identity is `(family, parent, ordinal)`, the graph is arena-backed with dense `u32` ids, and adjacency is CSR after the unit is built. petgraph is already a dependency (`crates/polint/Cargo.toml`, `petgraph.workspace = true`) and is used only for the debug `ImportGraph` and `FunctionGraph` (`analysis_neutral/graph.rs:1-60`).

### 4.9 Interning at scale: rustc's Symbol, lasso, hash-consing

**What the sources say.** rustc's `Symbol` is "an interned UTF-8 string. Internally, a Symbol is implemented as an index, and all operations (including hashing, equality, and ordering) operate on that index"; `as_str` "is a slowish operation because it requires locking the symbol interner" (https://doc.rust-lang.org/nightly/nightly-rustc/rustc_span/symbol/struct.Symbol.html). The interner is a single lock over a `DroplessArena` plus a hash table of borrowed slices, so key text is stored once and the map holds slices rather than owned strings (https://github.com/rust-lang/rust/blob/master/compiler/rustc_span/src/symbol.rs). lasso's `ThreadedRodeo` offers O(1) concurrent interning and can be frozen into a contention-free `RodeoReader` or a memory-minimal `RodeoResolver`, and exposes memory limits (https://docs.rs/lasso/latest/lasso/struct.ThreadedRodeo.html). hashconsing gives every value a uid so equality and hashing are constant time with "perfect sharing" (https://docs.rs/hashconsing/latest/hashconsing/). egg's e-graph relaxes invariants and restores them in a batched `rebuild` because per-insert maintenance "requires an expensive traversal through the e-graph on every union" (https://docs.rs/egg/latest/egg/tutorials/_01_background/index.html).

**Transferable.** Two of these matter for polint. First, ordering and hashing on the id, never on the text, which the current interner already does for equality but the normalise sorts do not (`ir/body.rs:123-160` sorts by resolved text). W5 keeps text ordering only where the canonical order is a contract and derives it from streamed structural comparison. Second, the freeze pattern: intern during lowering, freeze into a resolver for the graph stages, which removes the write lock that `intern_and_resolve` takes on every fact (`internal_core/stable_key.rs:78-88`).

### 4.10 Dominators

**What the sources say.** Cooper, Harvey and Kennedy describe an O(N²) worst-case algorithm that "runs faster, in practice, than the classic Lengauer-Tarjan algorithm" on graphs of compiler size; "rather than keeping distinct Dom sets, the algorithm can represent the dominator tree and read the Dom sets from the tree. The algorithm keeps a single array, doms, for the whole cfg, indexed by node", iterating "for all nodes, b, in reverse postorder" with a two-finger intersect (https://www.cs.tufts.edu/~nr/cs257/archive/keith-cooper/dom14.pdf). petgraph ships it as `algo::dominators::simple_fast`, returning immediate dominators and iterators that walk the tree (https://docs.rs/petgraph/latest/petgraph/algo/dominators/index.html). Lengauer and Tarjan's 1979 algorithm is the classic near-linear alternative (https://dl.acm.org/doi/10.1145/357062.357071).

**Transferable.** W4 exactly. The current implementation materialises Dom sets per block (`cfg/derived.rs:346-357`) and iterates set intersections; the reference keeps one array and never materialises the relation. The `reverse_postorder` field on `BasicBlockFact` already exists.

### 4.11 Pointer-analysis and call-graph scaling

**What the sources say.** SPARK's hybrid sets (explicit up to 16 elements, then bit vector) are "even faster than the bit set implementation, while maintaining modest memory requirements", declared-type filtering shrinks points-to sets, and difference propagation ("new" versus "old" parts) with a worklist avoids the naive solver's 60-plus iterations (https://plg.uwaterloo.ca/~olhotak/pubs/cc03.pdf). Whaley and Lam report that "introducing type filtering actually improved the analysis time and memory usage" in bddbddb (http://web.cs.ucla.edu/~palsberg/course/cs232/papers/Whaley-pldi04.pdf). Hardekopf and Lin's lazy and hybrid cycle detection find cycles from identical points-to sets or from a linear offline pass, and their BDD variant is "on average 2x slower than using sparse bitmaps but uses 5.5x less memory" (https://www.cs.utexas.edu/~lin/papers/pldi07.pdf). Wave propagation processes an acyclic constraint graph in topological order with a cached previous set per node and "is memory hungry" for it; deep propagation "uses far less memory" (https://dl.acm.org/doi/10.1109/CGO.2009.9). Graspan processes a Linux kernel graph of "more than 1B edges" by loading two edge partitions at a time and joining them, finishing "in a few hours with less than 6GB memory on the desktop" where an in-memory Datalog engine needed 29 GB and 3.5 days (https://aftabhussain.github.io/documents/pubs/asplos17-graspan.pdf).

Tip and Palsberg state that "algorithms such as RTA that use a single set for the whole program scale well" and that RTA "does almost as well as the seemingly more powerful algorithms" for unreachable-method detection (http://web.cs.ucla.edu/~palsberg/paper/oopsla00.pdf). Go's `x/tools` RTA is described as "a fast algorithm for call graph construction and discovery of reachable code" using dynamic programming over address-taken functions and dynamic call sites until a fixed point (https://pkg.go.dev/golang.org/x/tools/go/callgraph/rta); a near-linear complexity statement is not in that page and is unverified.

**Transferable.** polint's live RTA (`polint.solver`, `GoRtaPolicy`) is already cheap (146 ms at 45 files, benchmark report bullet 2); the expensive stages around it are lowering and joins, not propagation. When W8 designs cross-unit propagation, the applicable ideas are difference propagation in import order, hybrid sets, and type filtering, all of which fit a per-unit shard model. Graspan is the fallback if unit shards ever exceed memory on the largest monorepos; it is not needed for the consumer application's size.

### 4.12 Disk-backed stores from Rust

**What the sources say.** SQLite performs "50,000 or more INSERT statements per second" but "only a few dozen transactions per second", so inserts must be batched in one transaction (https://www.sqlite.org/faq.html#q19); WAL mode is "significantly faster in most scenarios" but "does not work well for very large transactions. For transactions larger than about 100 megabytes, traditional rollback journal modes will likely be faster" (https://www.sqlite.org/wal.html); rusqlite's `prepare_cached` reuses statement handles (https://docs.rs/rusqlite/latest/rusqlite/struct.Connection.html#method.prepare_cached). DuckDB is an embedded columnar engine with out-of-core joins and aggregates, but "if multiple blocking operators appear in the same query, DuckDB may still throw an out-of-memory exception" (https://duckdb.org/docs/current/guides/performance/how_to_tune_workloads.html). redb is a pure-Rust copy-on-write B+tree store with a single writer and concurrent readers (https://docs.rs/redb/latest/redb/). Arrow's columnar layout provides O(1) random access and relocation "without pointer swizzling", and dictionary encoding as an integer index array plus a dictionary (https://arrow.apache.org/docs/format/Columnar.html).

**Transferable.** For W7's shards the evidence favours an explicit columnar blob per unit (dictionary-encoded ids, one file, no per-row SQL) with SQLite for manifests and cross-unit indexes; this is Q3's answer and the reason option C's row-per-fact store is not recommended for millions of rows per run. The 100 MB transaction note bounds how a fact-row store would have to be chunked if Q3 chose otherwise.

### 4.13 Synthesis: cause to reference

| Cause (section 3) | What the references do instead | Workstream |
|---|---|---|
| 3.1 composed-text identity, retained | integer fact ids with structured keys (Glean, CodeQL dbscheme); `Symbol(u32)` with text only on resolve (rustc); hash-consed uids | W5 |
| 3.2 quadratic joins | per-relation indexes chosen from access patterns (Souffle); dense id arenas and CSR adjacency (petgraph, la-arena); tuple-keyed relations (CodeQL) | W1, W6 |
| 3.3 digest materialisation | hash sorted id tuples, never formatted rows (Arrow-style dictionaries); the repository's own X5a streaming | W2 |
| 3.4 whole-program exploded solver | demand construction of the supergraph (CC 2010); per-procedure summaries bottom-up (Infer, CACM 2019, ECOOP 2026); jump-function garbage collection (ECOOP 2024) | W3, W8 |
| 3.5 naive dominators | one idom array in reverse postorder (Cooper, Harvey, Kennedy; petgraph) | W4 |
| 3.6 sequential whole-program stages | per-package summaries in import order with persisted facts (gopls, go/analysis); per-body MIR queries (rustc); function passes (LLVM); unit-owned facts with stacked layers (Glean) | W6, W7 |


## 5. Architecture options

Five candidates, stated so that they can be compared on the same axes: scaling behaviour, memory ceiling, what changes in the public contract, migration path, and how the consumer scan validates it. "Public contract" here means what `docs/facts/` documents and what the rule host and the ai-friendly report carry: typed fact views and policy queries (`docs/facts/capability-plans.md`, `docs/facts/calls.md:11-13` states that the public API "does not expose raw call-graph nodes, dense IDs, solver internals, or provider data structures"), the `evidence_v1` envelope with its stable keys and replay keys (`docs/facts/evidence.md:55`; `crates/polint/src/sdk/policy.rs:1029-1063`), and the symbol, definition and reference stable-key text that `sdk/facts.rs:490-566` resolves for rule authors. MIR, CFG, domain, call-site and refined-call identities are internal.

### Option A: targeted algorithmic fixes inside the existing pipeline

What it is: index every join in section 3.2 once per stage (the `CfgInputs` pattern at `cfg/lower.rs:40-97`); stream every digest in section 3.3 the way X5a did for evidence; replace the dominator algorithm with an array-based reverse-postorder iteration and stop computing the relation when only the tree is emitted; gate `abstract_domains` behind the families that actually need it rather than the providers; parallelise `lower_file` across files with the existing sort-by-path merge.

- Scaling: removes the O(B × Ops), O(F²), O(C × R), O(S × N) and O(Rows × S) terms and the transient digest peaks. What remains is linear in facts with the section 3.1 constant.
- Memory ceiling: the retained floor of section 3.1 stays. By the measured 624 B per key and the fact growth ratios in section 3.8, the full backend's `calls` run would retain on the order of 6 to 9 GB by the end of `refined_calls` (hypothesis). That fits a 30 GB host and probably a 16 GB CI runner, and does not fit an 8 GB one.
- Contract: none. Every fix is identity-preserving and provable by the per-provider digest oracle (`.scale-envelope/EXPERIMENTS.md`, "The A/B that proves what changed"), except the domains gate and the solver budget reporting, which change which facts exist and must be reported as such.
- Migration: a series of small PRs under the delivery rules of report 03 section 3. Nothing is deleted; nothing dual.
- Validation: the 885-file scope should fall from 152 s to well under 60 s before any identity change (hypothesis, from the stage split in benchmark report A.4 where `semantic_mir`, `cfg`, `type_value_alias`, `refined_calls` and `abstract_domains` sum to 96 s of 152); the full backend should complete `calls` for the first time.
- Ceiling of the option: it does not change the representation, so every future provider inherits the text-identity floor and the whole-program-stage shape. It buys the first completion, not the target.

### Option B: query-based incremental core (Salsa-style)

What it is: replace the sequential provider loop with memoized queries keyed by inputs (per file, per function, per package), with derived queries for MIR, CFG, summaries and call edges, red-green re-validation, and demand-driven evaluation so that a `calls` rule computes only what its query touches.

- Scaling: work becomes proportional to the demanded slice on warm runs; cold runs still compute everything the demand touches, which for a repo-wide reachability query is everything. The cold full-backend cost is therefore the same as option D's cold cost, and the warm cost is what B is for.
- Memory ceiling: memoized values are retained unless evicted; Salsa-style engines rely on LRU and cancellation and hold the memo tables in process. Without a persistence layer the ceiling is the same as today's, only reorganised.
- Contract: the capability derivation from typed views is unchanged; the provider manifests, layer cache keys (`analysis_api/digest/keys.rs:9-21`), run manifests and provider-outcome rows in the store would all change meaning, because "provider ran" stops being the unit of work. The ai-friendly report's `providers` array is a public surface.
- Migration: a rewrite of `analysis_kernel` and every provider entry point, and a conflict with the delivery rule against dual paths, because a query engine cannot coexist with a stage loop for long. The repository already has `analysis_neutral/demand` (2,457 lines) as a trace-only engine and Phase 68 pending; this option promotes it to the core.
- Validation: warm `polint review` after a one-file change recomputes only the changed functions and their dependents (report 03, Stage 2 exit criterion). The consumer scan validates cold cost only after the representation is also fixed.
- Assessment: B is the right model for warm latency and the wrong first move for cold scale. Its precondition is that the unit of memoization is cheap to build and small to retain, which is the representation problem, not the scheduling problem.

### Option C: sharded persistent fact store with evaluator-side joins

What it is: providers write facts as relational rows into per-package (Go) or per-project (TS) shards in the existing SQLite store, with numeric ids instead of text keys; consumers, including the interprocedural stages, run joins against the store rather than against in-memory `Vec`s; the working set is whatever the query touches.

- Scaling: the join engine's, which for SQLite is B-tree indexed joins with page caching. Memory ceiling becomes the page cache plus the per-query result, which is the property CodeQL and Glean rely on.
- Contract: stable-key text can still be produced at the boundary from the relational identity, so `evidence_v1` and the symbol keys survive if the canonical text recipe is preserved. The store schema (`analysis_kernel/store/migrations.rs`, currently two provider mirrors) becomes a one-way door under report 03's rule 5.
- Migration: the largest single change of the five; the store today is maintenance-only and holds no facts (`research/strategy/03-build-plan.md` section 1, "Store: schema v5, two provider mirrors, maintenance-only"). Every consumer would be rewritten against a query API. Write throughput for millions of rows per run is a real cost that must be measured before committing.
- Validation: peak RSS proportional to working set (report 03 Stage 2 exit criterion) becomes directly testable.
- Assessment: C is where persistence should end up for cross-scope joins, but as the primary in-run representation it trades a memory problem for a serialisation problem. The engines that use this model (section 4) separate extraction from evaluation and accept a batch cost polint's diff-time positioning does not.

### Option D: scoped-deep analysis units with a dense-identity core and persisted graph shards

What it is: make the unit of deep analysis a scope (a Go package; a TS project), not the program. Inside a unit, MIR lowering, CFG, local domains and direct call extraction run per function into arena storage with dense integer ids, in parallel across units, with a deterministic merge order. A unit's graph is persisted as a shard keyed by the content digests the layer cache already computes. Interprocedural stages (refined calls, RTA, reachability, summaries, data flow) read unit shards and join across them on demand, in import order for Go, where the package graph is acyclic by language rule, and per project for TS. Text identity is computed from the structural identity only at the public boundary.

- Scaling: lowering and CFG become linear in the unit and parallel across units; the interprocedural stages become linear in (edges touched) with the unit boundary as the summary point. The cold full-backend cost is bounded by the largest unit plus the cross-unit join, and the warm cost by the changed units plus dependents.
- Memory ceiling: the in-process working set is the units being analysed plus the cross-unit index; retained key text disappears because identity is `(family, parent id, ordinal)` in a per-unit arena. The text of a key is derivable, so the interner becomes a boundary cache, not a run-long store.
- Contract: unchanged at the SDK. The per-fact honesty and evidence properties survive because they are computed from the same canonical recipe at the boundary. What changes is internal: the `AnalysisHost` trait's `Vec<&Fact>` accessors, the `FactMetaStore` shape, and the layer kinds in the cache manifest. The determinism gate (byte-identical output across order and cache state) is the acceptance test for every step.
- Migration: A's fixes first (they are the first workstreams and are identity-preserving), then the identity pivot behind the existing resolver contract as `NEXT-LEVER.md` proposed, then per-unit lowering, then shards, then cross-unit joins. Each step deletes the path it replaces.
- Validation: the consumer backend's forced `calls` run is the gate; the scope table in section 1.2 is the curve that must flatten.
- Assessment: this is the option the evidence supports. It removes the root cause (text identity and whole-program stages) rather than compensating for it, it keeps the moat properties, and it lands as a sequence that produces a measurable win at every step.

### Option E: "rewrite the graph layer in Rust"

Stated plainly: the graph layer is already Rust (`analysis_neutral` is 89,615 lines, `analysis_kernel` 37,313, `core` 12,560; section 1 of report 03 gave the same order of magnitude at v0.3.3). The sidecar that is not Rust is the fast part. A rewrite therefore means option D's re-architecture of the Rust stages: a new identity model, a new arena-based per-unit core, new derived-relation algorithms, and new interprocedural joins, with the public boundary types kept. What it does not mean is rewriting the Go sidecar in Rust: `packages.Load` and SSA are the Go toolchain's own type checker and IR, and no Rust reimplementation would be more accurate or, on the evidence, faster than 25 s. The same holds for the TS type sidecar (section 9). The "rewrite" framing is useful only as permission to delete: the delivery rule against dual paths means that each stage of D replaces its predecessor outright.

### Option D in detail

Stated at the level `research/AGENTS.md` asks of an implementation recommendation: unit boundary, data model, identity, cache keys, first vertical slice, non-goals.

**Unit boundary.** A unit is the set of files the language's own build unit groups: a Go package (the `module_topology` provider already emits `import_to_package_edges`, `analysis_kernel/provider.rs:1418`), or a TS project as discovered by the type sidecar's `tsconfig` walk, falling back to a single file when no project claims it (Q7). Units are ordered by the package import DAG for Go; TS project references likewise, with intra-unit cycles handled by the existing SCC closure. Every fact a unit produces is owned by that unit, in Glean's sense (section 4.2).

**Data model.** Per unit, an arena graph:

```text
UnitGraph
  bodies:      Arena<Body>            # Body { function: FunctionId, span, status, owner: SymbolRef }
  blocks:      Arena<Block>           # Block { body: BodyIdx, ordinal, first_stmt, n_stmts, terminator }
  statements:  Arena<Statement>       # Statement { body, ordinal, operation: OpIdx }
  operations:  Arena<Operation>       # Operation { body, ordinal, span, kind, status }   (kind as today, ir/op.rs:10-18)
  places:      Arena<Place>           # Place { body: Option<BodyIdx>, root, projections: Range<ProjIdx> }
  cfg:         Csr<BlockIdx, EdgeKind> per body, plus idom: Vec<Option<BlockIdx>> and ipdom
  call_sites:  Arena<CallSite>        # CallSite { body, operation, callee: CalleeRef, span }
  local_facts: per-family Vec<T> indexed by the arenas above (domains, direct calls, unsupported)
  identity:    IdentityTable          # see below
  digest:      Digest                 # over the arenas' canonical byte stream, not formatted rows
```

The `MirTerminatorKind`, `MirOperationKind`, `PlaceRoot` and `PlaceProjection` enums are kept as they are (`ir/body.rs:51-99`, `ir/op.rs:20`, `ir/places.rs:8-17`); only the id and key fields change. `Vec<PlaceId>` argument lists and `Vec<(MirValue, MirBlockId)>` switch cases become ranges into side arenas so that an operation is `Copy`.

**Identity.** `StableKeyId` remains a `u32` newtype in the public resolver contract. Internally it indexes an identity table whose rows are `(family: u8, parent: StableKeyId, discriminant: u32, extra: Option<AtomId>)`, where `discriminant` is the ordinal, byte offset or projection index the current recipe already uses and `extra` is an interned atom for the few free-text parts (function name, projection field name, unsupported construct label). Canonical text is produced by walking parents into the existing `write_stable_key_text` buffer (`analysis_api/metadata.rs:512-523`); the output must be byte-identical to today's, which is testable per family by differential replay of every fixture and corpus. `payload_digest` becomes a `u64` FNV state computed by streaming the same walk (`core/metadata.rs:380-398` already hashes text parts in this order; `NEXT-LEVER.md`'s composable-summary result applies if walk cost shows up). Ordering by canonical text, where it is a contract, compares streamed prefixes; where it is not (every internal `normalized` sort), it becomes ordering by `(unit, family, index)`.

**Cross-unit references.** A unit refers to another unit's function by `(unit digest, family, index)` plus the symbol's public stable-key text for the SDK boundary. The cross-unit index is a table from qualified function name and from `(file, span)` to that triple; it is what W8's joins consult, and it replaces every `db.functions().iter().find` in section 3.2.

**Cache keys.** A unit shard's key is the layer cache's existing `LayerKey` for the unit's files (`analysis_kernel/incremental/keys.rs:51-62`, which already carries per-provider `input_digests` and `dependency_layer_digests`) extended with the unit's import list digests, so that a change in a dependency's exported symbols invalidates the dependents' shards but not their siblings; this is the gopls analysis-driver key shape (section 4.5). Sidecar rows are already keyed this way (`go/semantic/cache_key.rs`).

**First vertical slice.** One unit type (Go package), one lowering (`lower_unit` for Go), CFG with idom arrays, direct call extraction, and the `Calls` view answered from unit shards through the cross-unit index, with the whole-program `SemanticStore` and `CfgFactStore` deleted for Go in the same PR series; TS follows the same path once the Go slice passes G6. The slice is measured on the 885-file scope (G2) and then the full backend (G3, G4, G6).

**What stays.** The provider manifests and the capability closure (moved to family granularity by W3), the `evidence_v1` envelope, the SDK views, `polint unknowns`, the determinism gate, the resource envelope, the sidecars and their caches, and the honesty vocabulary (precision, status, budget reasons).

**Non-goals.** No change to which facts a rule can ask for; no new public raw-graph API (Phase 69's `polint graph` is separate); no query language; no distributed or GPU solving; no attempt to move MIR lowering into the Go sidecar (the sidecar's SSA is a different IR with different semantics, and the lowerer's tree-sitter path is language-shared with TS).

### Rejected alternatives

| Alternative | Why not | Evidence |
|---|---|---|
| Swap the allocator | 0.2 percent of peak for a 6.6 percent wall regression | `.scale-envelope/EXPERIMENTS.md` X1 |
| Shard the interner | ids are dense insertion indexes; the type's own doc says sharding breaks resolve; and X2 showed lock traffic is not the cost | `internal_core/stable_key.rs:24-36`; X2 reverts |
| Raise the dominance and solver budgets | the relation is derivable and the solver's output is unread by `summary_call`; larger budgets buy more facts nobody consumes | sections 3.4, 3.5 |
| Emit MIR from the Go sidecar instead of lowering in Rust | the sidecar is 25 s whole-program and already overlapped; the Rust lowerer is shared with TS; SSA is not the MIR the rules are specified against | benchmark report 5.3.2; `go/mir/lower.rs`, `ts/mir/lower.rs` |
| Rewrite the sidecars in Rust | they are the fast part; reimplementing `go/types` and the TypeScript checker would lose accuracy and gain nothing measured | benchmark report bullet 4; PR #121 measurement |
| Drop per-fact metadata rows | they carry the precision, confidence and validation status the honesty contract exposes | `analysis_api/metadata.rs:231-239`; report 02 section 6 |
| Transparent spill-to-disk for the in-memory graph | tried and removed by Joern as too slow; CodeQL's disk cache comes with 64 GB host guidance | section 4.3, 4.1 |
| Query engine first (option B) | memo units must be small before memoisation helps; Salsa has no persistence; the whole-program `MirOutput` is the wrong unit | section 4.4 |

### Comparison

| Axis | A: targeted fixes | B: query core | C: sharded store | D: scoped-deep + shards | E: rewrite |
|---|---|---|---|---|---|
| Cold full-backend `calls` | completes; likely over 300 s or 12 GB (hypothesis) | same as D once representation fixed; otherwise same as today | bounded by join engine; write cost unmeasured | bounded by largest unit plus cross-unit join | is D |
| Warm one-file change | no change from today's layer cache | recompute demanded slice | recompute changed shards | recompute changed units plus dependents | is D |
| Memory ceiling | retained text floor stays (6 to 9 GB estimate) | memo retention, in process | page cache plus query | working set of units | is D |
| Public contract | none | provider-outcome surfaces change | store schema one-way door | none at SDK; internal traits change | is D |
| Migration size | small PRs, identity-preserving | rewrite of kernel and providers, no dual path possible | rewrite of consumers against a query API | staged; A first; each step deletes its predecessor | is D |
| Consumer validation | first completion of full backend | warm review parity | working-set RSS | the scope curve flattens; 300 s / 12 GB gate | is D |
| Risk | leaves the design in place | scheduling before representation | serialisation cost; batch shape | identity pivot must be byte-identical | as D |

## 6. Recommendation

Adopt option D, sequenced so that option A is its first three workstreams. The reasoning in one paragraph: the benchmark report proves the wall is Rust-side graph construction; the code shows that the wall is five defects that all reduce to "identity is text and stages are whole-program"; the prior experiments prove that tuning around the text does not move the floor; the public contract does not expose the internal identities, so the representation can change without an SDK break; and Go's acyclic package graph makes a per-package unit a natural, deterministic, parallel scope with summaries at the boundary.

Three constraints on how D is built:

1. **Identity-preserving until the pivot, byte-identical after it.** The thing that must not move is the fact rows: for every family, the sorted set of (canonical stable-key text, payload digest) pairs, plus the `polint check` diagnostics digest and the ai-friendly stdout. A provider's `digest=` value is a cache key, not a fact digest: in this tree it is a function of the provider's own rows and of the output digests of the upstream providers its recipe names, fetched through `ProviderCtx::dependency_digest` (`crates/polint/src/analysis_kernel/provider.rs:69-74`), which returns a fixed `absent` value for a provider that did not run. `direct_summaries` folds `abstract_domains=` (`analysis_neutral/summaries/provider.rs:42`), `type_value_alias` folds `abstract_domains=` and `direct_summaries=` (`types/provider.rs:221-222`), `refined_calls` folds `direct_summaries=`, `type_value_alias=` and `solver=` (`refined_calls/provider.rs:629-632`), and so on for every deep provider (Appendix A, "digest recipes"). Two consequences fix the verification story. First, for a workstream that leaves the provider graph and the demand set unchanged (W1, W2, W4, W5), every provider digest must be byte-identical, and the provider-digest oracle (`.scale-envelope/digests.py`) is sufficient because a moved row would move its provider's digest. Second, for a workstream that changes which providers run (W3) or which providers exist (W6), the provider digests of every consumer that names a changed provider move by construction, and the oracle for those workstreams is the fact-row dump (section 7, G1c) plus the diagnostics digest and stdout; the set of provider digests expected to move is written down per workstream (section 8) and any digest outside that set moving is a failure. W5 changes the internal identity but must reproduce the canonical key text and every digest at the boundary; `NEXT-LEVER.md` lists the proof obligations and they are adopted unchanged.
2. **No dual paths.** When per-unit lowering lands, whole-program lowering is deleted in the same PR; when shards land, the in-memory whole-program tables for the families they replace are deleted. This is report 03 rule 2 and it is what keeps E from becoming a second engine.
3. **Demand at family granularity.** The capability closure moves from "which providers" to "which fact families" so that `calls` stops paying for domain observations it does not read. This is the cheapest large win and it is also the seam that later lets a query model (option B) grow inside D without replacing the kernel.

What the consumer can do before any of this lands, restated from the benchmark report so that the plan does not read as "wait": a deep rule scoped to one bounded context of at most about 900 Go files completes today (885 files in 152 s and 13.7 GB) and needs a job with at least 16 GB; nothing repo-wide should request `calls`, `control_flow` or `dataflow` until G6 passes; and any deep rule must read the `budget_exceeded` rows because the dominance relation is bounded from 314 files up (benchmark report, section 7, items 6 to 8b). W0 is independent and should ship first because it is the only item that changes the consumer's CI cost today.

What is explicitly not recommended: a query engine as the first move (B), the store as the in-run representation (C), a rewrite of either sidecar, and any change to the public fact views or the `evidence_v1` envelope.

Decisions taken by research after the first draft (Resolved Questions): W7 and W8 are in scope for this track but are not prerequisites for the cold gate (Q1); the analysis traits are crate-private and change freely, with the SDK views and `evidence_v1` frozen (Q2); a unit shard is one binary columnar blob in the layer cache with SQLite for manifests and the cross-unit index, layout locked by the SUM-03 benchmark (Q3); domains leave the `calls` path because no `calls` consumer reads the one summary row they influence (Q4); the domain solver gets per-function caps with a per-run total (Q5); the TS unit is the `tsconfig` project, or the file when no project claims it (Q7); the wall gates bind on the benchmark host and the memory gate is sized for the 16 GB runner class (Q9). Provisioning the gate runner is the one decision left to the owner (Q6).

## 7. Acceptance targets and the probes that verify them

Every gate below is a command plus a threshold. The environment is the one the benchmark report used (16 cores, 30 GB, `RAYON_NUM_THREADS=12 POLINT_JOBS=12 GOMAXPROCS=12`, Go toolchain on `PATH`, `POLINT_CACHE_STORE` unset). Two memory figures are reported for every probe and the gates say which one they bind. The polint-process peak is the `peak_rss_mb` field of the last stage row, which is `getrusage(RUSAGE_SELF).ru_maxrss` (`crates/polint/src/measure.rs:27-32`) and excludes every child process. The tree peak is measured by the committed sampler `.scale-envelope/rssrun.py`, which walks `/proc/<pid>/task/<pid>/children` from the probe's pid every 200 ms, sums `VmRSS` over the whole tree, and prints a JSON summary with `peak_rss_bytes` and `wall_s` on stderr (`rssrun.py:1-8`, `:22-58`, `:75-107`); it also installs an `RLIMIT_AS` guard so a runaway probe fails with an allocation error instead of taking the host down. `/usr/bin/time -v` is not used for memory: its "Maximum resident set size" is `ru_maxrss` of the timed process or of its largest single waited-for child, never a sum over the tree, and the Go sidecar is a grandchild. The benchmark report's `runner.py` (uncommitted) measured the same tree quantity as `rssrun.py` at a 150 ms interval, so the report's tree peaks are comparable to `rssrun.py`'s. `POLINT_MEMORY_CEILING_MB` remains the in-process safety net. The scratch checkout and the `[languages.go]` block are the ones the benchmark report's section 5.3 describes; the consumer repository is never named in any committed artifact.

```sh
export GOROOT=/opt/data/home/.local/share/go
export PATH=$GOROOT/bin:$HOME/.local/bin:$HOME/.cargo/bin:$PATH
export RAYON_NUM_THREADS=12 POLINT_JOBS=12 GOMAXPROCS=12 GOFLAGS=-p=12
export RUST_LOG=polint=debug
export POLINT_GATE_OUT=${POLINT_GATE_OUT:-/tmp/polint-gate}; mkdir -p "$POLINT_GATE_OUT"
probe() { # $1 tag, $2 cap, $3... paths; POLINT_GATE_TIMEOUT (s, default 300); KEEP_CACHE=1 for warm cells
  tag=$1; cap=$2; shift 2
  [ -z "${KEEP_CACHE:-}" ] && rm -rf .polint/cache/analysis .polint/cache/layers
  python3 .scale-envelope/rssrun.py --label "$tag" --as-limit-gb 28 \
    --timeout "${POLINT_GATE_TIMEOUT:-300}" --timeline "$POLINT_GATE_OUT/$tag.timeline.json" -- \
    polint unknowns --cap "$cap" "$@" > "$POLINT_GATE_OUT/$tag.stdout" 2> "$POLINT_GATE_OUT/$tag.stderr"
  echo "exit=$?"
  grep '"peak_rss_gb"' "$POLINT_GATE_OUT/$tag.stderr" | tail -1          # tree peak and wall, from rssrun.py
  grep "stage done" "$POLINT_GATE_OUT/$tag.stderr" | tail -1 | grep -oE 'peak_rss_mb=[0-9]+'   # polint-process peak
  python3 .scale-envelope/stages.py "$POLINT_GATE_OUT/$tag.stderr"
}
```

Probe matrix. Each gate is run on the cells its threshold names; the full matrix is what a release measurement records so that a regression in one cell is visible against the others.

| Scope | cold (analysis and layers wiped) | warm (cache kept) | one-file change (W7 onward) |
|---|---|---|---|
| 45 Go files, `calls` | G1 oracle, G1c | G7 | |
| 45 Go files, `dataflow` | G1 oracle, G1c (the only Go cell where `data_flow` and `evidence` run) | | |
| 885 Go files, `calls` | G1 oracle, G1c, G2, G1b, G5 | G7 | G9 |
| 885 Go files, `control_flow` | G5 | | |
| 1,588 Go files, `calls` | G1b, curve point (`POLINT_GATE_TIMEOUT=900`; the scope measures 300.6 s today, so the default timeout kills it) | | |
| 4,752 Go files (full backend), `calls` | G3, G4, G6, G10 | G6 warm | G9 |
| 2,381 TS files (full frontend), `calls` | G8 | | |
| excalidraw (public, 385 TS files), `dataflow` | G1 oracle and G1c against `.scale-envelope` X6 (the X-series ran the full `dataflow` plan) | | |

Provider counts per cell: a `calls` request enables 21 of the 23 manifests (`providers_enabled_by_boolean_gates`, `analysis_kernel/provider.rs:1031-1068`; `data_flow` and `evidence` are added only for `dataflow`, `:1065-1067`), and a `digest=` field is printed only for a provider that ran (`analysis_kernel/mod.rs:324-341`). The oracle's pass string is therefore "N of N" where N is the number of providers with a stage row in the before capture: 21 on a Go `calls` cell, 23 on a `dataflow` cell; a missing row in the after capture is a `MISSING` failure. On excalidraw the Go providers run and produce no output, so N is read from the capture, not assumed.

Digest comparison uses `.scale-envelope/digests.py`, which extracts the `provider=... digest=...` pairs from two stderr captures and prints the count of identical providers with any `DIFFER` or `MISSING` rows; it is the same tool that caught the X6 false positive. Fact-row comparison (G1c) uses a dump the plan adds in its first slot: for every fact family, the sorted list of (canonical stable-key text, payload digest) as the kernel's test-only metadata report already materialises per family (`crates/polint/src/analysis_kernel/debug.rs:29-68`, `metadata_debug_json_for_test`), exposed through the eval harness so it can be run on a checkout, and diffed byte for byte between the before and after binaries. It is the oracle for the two workstreams whose provider digests move by construction (W3, W6) and the mechanism W5's obligation 1 needs anyway.

| Gate | Workstream it closes | Probe | Threshold |
|---|---|---|---|
| G0 rule-host store hit from a fresh cargo home | W0 | `CARGO_HOME=$(mktemp -d) polint check ...` twice, second run after publishing once from a different cargo home | second run restores in under 10 s; no `cargo` or `rustc` in the process tree |
| G1 provider-digest oracle after each identity-preserving change | W1, W2, W4, W5 | `.scale-envelope/digests.py` comparison of per-provider `digest=` values in `stage done` rows, before and after, on excalidraw (`dataflow`), the 45-file scope (`calls` and `dataflow`) and the 885-file scope (`calls`) | N of N digests identical, N read from the before capture (21 on a `calls` cell, 23 on a `dataflow` cell); no `MISSING` row; `polint check` diagnostics digest identical |
| G1b cost split confirmed | W1 | temporary `polint::probe` step rows inside `lower_go_mir` around `lower_file`, `finish_with_types`, `lower_control_flow` and `normalized`, as X4 did for evidence, on the 885 and 1,588-file scopes | the four superlinear terms of the section 3.2 table together account for most of the stage before W1 and are not visible after; their relative order is recorded, and section 3.8 is corrected if it disagrees |
| G1c fact-row oracle | W3, W6 (and W5 obligation 1) | per-family sorted (canonical key text, payload digest) dump, before and after, on the same cells as G1 | byte-identical for every family except those the workstream names as changed (W3: `domain_*` absent on `calls`; W6: none); diagnostics digest and ai-friendly stdout identical |
| G2 885-file scope | W1, W2 | `probe s885 calls <885-file scope>` | wall under 60 s; polint-process `peak_rss_mb` under 7,500 (today 8,143 with 7,113 retained after `refined_calls`, benchmark report A.4); tree peak reported beside it and expected to stay near today's 13.7 GB because the whole-program sidecar's roughly 7 GB coincides with the early stages and W1 and W2 do not touch retained memory |
| G3 `semantic_mir` on the full backend | W1, W2, W6 | `probe full-mir calls <core>` and read the `polint.semantic_mir` stage row | stage under 60 s; `rss_delta_mb` under 3,000; `key_mb` growth under 1,500 (today 565 s, +10,415 MB, 3,448 MB, measured by the benchmark's 1,800 s over-budget run; the first post-W1 probe on this cell uses `POLINT_GATE_TIMEOUT=1800` to obtain a comparable baseline) |
| G4 `cfg` on the full backend | W4 | same run, `polint.cfg` stage row and step rows | stage under 30 s; `dominators` step under 5 s; with `POLINT_CFG_MAX_DOMINANCE_PAIRS=0` the `cfg_dominators` and `cfg_postdominators` pair sets are identical to today's, including the vacuous post-dominance rows of exit-unreachable blocks (section 8, W4) |
| G5 domains no longer on the `calls` path | W3 | `probe s885-calls calls <885-file scope>`; `probe s885-cf control_flow <885-file scope>` | `polint.abstract_domains` absent from the stage rows on `calls`, present on `control_flow`; G1c: every family byte-identical on `calls` except `domain_observations` and `domain_events`, which are absent; provider digests of exactly `direct_summaries`, `type_value_alias`, `semantic_graph`, `solver` and `refined_calls` move (they fold the absent `abstract_domains=` digest, directly or transitively) and no other digest moves; `polint unknowns` reports the budget rows on `control_flow` |
| G6 full backend `calls` completes | W1 to W6 | `probe full-calls calls <core>` | exit 0; wall under 300 s; tree peak (`rssrun.py`) under 12 GB; polint-process `peak_rss_mb` reported beside it; all 21 selected providers have a stage row |
| G7 determinism across order and cache state | every workstream | the existing N=10 permutation gate extended to include the 885-file consumer scope, cold and warm | byte-identical ai-friendly stdout |
| G8 full frontend `calls` completes | W6 (TS lowerer twin fixes) | `probe full-ts calls <frontend paths>` | exit 0; wall under 300 s; tree peak under 12 GB (no Go sidecar runs on this cell, so tree and polint-process peaks are close; today `semantic_mir` alone is 203 s) |
| G9 warm re-scan after one changed file | W7, W8 | `probe` twice, editing one Go file between runs, cache kept | second run under 30 s; only the changed unit and its dependents recomputed, asserted by shard manifest (hypothesis until W7 exists) |
| G10 memory envelope reporting | W9 | `POLINT_MEMORY_CEILING_MB=8192 probe full-calls calls <core>` | run finishes with a `polint/resource-budget` diagnostic rather than a kill; the capabilities that degraded are listed |

The thresholds for G2, G3, G4 and G6 are defended as follows. G2's memory figure is the polint process alone because that is the only figure W1 and W2 can move: at 885 files the stage table shows 7,113 MB retained after `refined_calls` and an 8,143 MB polint-process peak (benchmark report A.4), the sidecar reports a 6,846 MB heap in its own process (A.4, sidecar phases), and the measured 13,689 MB tree peak is those two coinciding. W1 and W2 remove scans and transients, not retained rows, so the polint-process peak can fall by roughly the transient gap (about 1 GB) and no further, which is what 7,500 MB asserts; the retained floor is W5's job. A tree ceiling below the sidecar's heap plus the retained floor would be unreachable by construction, which is why the earlier 8 GB tree figure was wrong. For G6, the tree peak is the maximum over time of the polint process plus the sidecar: the sidecar's roughly 7 GB is present during its 25 to 27 s prefetch window, when polint is in the syntax, module-graph and early lowering stages (under 1 GB at 13.6 s on the full backend today, A.5), and is gone by the time the interprocedural stages run, so 12 GB leaves polint about 5 GB during the overlap and the full 12 GB afterwards. The rest of the argument: The upstream pipeline through `module_topology` is 13.6 s on the full backend (benchmark report, section 5.3); the sidecar is overlapped; the linear parts of lowering are bounded by parsing, which is 5.8 s for the same files. A linear, parallel `semantic_mir` should therefore land within a small multiple of the syntax stage, and 60 s is a ten-times safety margin. The `cfg` gate follows from the dominator algorithm change being near-linear. The 300 s / 12 GB end-to-end gate is the benchmark report's own budget and the 16 GB CI runner class named in report 02's hygiene table, with headroom for the sidecar's 7 GB heap, which runs in a separate process and is not counted in polint's RSS but is counted in the tree. For reference, GitHub's standard hosted Linux runners are 4 vCPU and 16 GB for public repositories and 2 vCPU and 8 GB for private ones, and larger runners are offered at 8 vCPU / 32 GB and 16 vCPU / 64 GB (https://docs.github.com/en/actions/reference/runners/github-hosted-runners, https://docs.github.com/en/actions/reference/runners/larger-runners); the memory gate is the one that ports across those classes, and the wall gates bind on the benchmark host until W9's job exists (Q9).

## 8. Roadmap: dependency-ordered workstreams

No calendar; each workstream names what it builds, why it is on the critical path, its probe, and what it depends on. Delivery rules are report 03 section 3.

### W0. Rule-host store key independent of the cargo home path

- Build: drop the `cargo_home` line from `build_fingerprint` (`crates/polint/src/cache/rules_store.rs:797-806`). The cargo config files discovered under the cargo home are already hashed by content (`:827-834`, via `cargo_config_files` at `:828`), and `shareable_with_cargo_home` (`:1190-1197`) already decides shareability from those contents. The path string carries no information the contents do not. Add a fixture: two empty cargo homes at different paths produce the same fingerprint.
- Why: 193 s per fresh-container CI run for a 5 s analysis (benchmark report, section 4.2, experiment E1). Independent of everything else in this document.
- Probe: G0.
- Depends on: nothing.

### W1. Index every join (identity-preserving)

- Build: `lower_file` and `push_body` take pre-built maps (function by `(file, name, span)`, package by file, module node by file); `go_closure_capture_names` and its TS twin use the indexed `references_for_file` and a `definitions_by_symbol` index, exposed through `AnalysisHost` overrides in `impl AnalysisHost for AnalysisDb` (`core/db.rs:6043`; the inherent method returns an iterator, `core/db.rs:3746-3753`, while the trait method returns a `Vec` of references, `analysis_neutral/host.rs:129-137`, so the override collects); `lower_control_flow` groups operations and effects by body once (`go/mir/lower.rs:115-124`, `ts/mir/lower.rs:132-141`); `Icfg::from_facts` indexes CFG nodes by operation (`ifds/mod.rs:93-98`); the refined-calls Go semantic join indexes native call sites by `(file, span)` and functions by `(file, name)` (`refined_calls/provider.rs:361-454`); `owner_symbol` indexes symbols by `(file, name, span)` (`calls/extract.rs:554-567`). Delete the duplicate `normalized` call on the lowerer's return (`go/mir/lower.rs:99`, `ts` twin) since `SemanticStore::from_output` normalises again (`store.rs:38`).
- Why: removes every superlinear term in section 3.2; the first change that makes the full backend's `semantic_mir` finish in a bounded time.
- Probe: G1 on excalidraw and the 885-file scope; G2.
- Depends on: nothing. This is Stage 0 "scale root cause" in report 03.

The index set for W1, stated so that the PR can be reviewed against it:

| Scan today | Index that replaces it | Key | Built where |
|---|---|---|---|
| `matching_function` (`go/mir/lower.rs:2317-2329`; TS `:4540-4553`) | `HashMap<(FileId, &str), Vec<&FunctionFact>>` then span containment over the bucket | file, name | once per `lower_go_mir` / `lower_ts_mir` call |
| `enclosing_function` (`ts/mir/lower.rs:4568-4581`) | per-file `Vec<&FunctionFact>` sorted by span start, binary search then smallest containing | file | once per lowering |
| `push_body` package and module (`go/mir/lower.rs:645-656`) | `HashMap<FileId, PackageId>`, `HashMap<FileId, ModuleNodeId>` | file | once per lowering |
| closure captures (`go/mir/lower.rs:741-742`; TS `:764`) | the existing `DenseFileIndex` behind `AnalysisDb::references_for_file` (`core/db.rs:157`, `:3746-3753`), exposed by overriding the trait default in `impl AnalysisHost for AnalysisDb` (`:6043`); `definitions_for_symbol` likewise (`:3730`) | file; symbol | already built by `finalize_fact_view_indexes` (`core/db.rs:760`) |
| `lower_control_flow` per-body filters (`go/mir/lower.rs:115-124`; TS `:132-141`) | group operations and control effects by body once, `Vec<Range<usize>>` since operations are pushed in body order | body | once per lowering |
| `Icfg::from_facts` (`ifds/mod.rs:93-98`) | `HashMap<MirOpId, CfgNodeId>` | operation | once per `Icfg::build` |
| refined-calls Go semantic join (`refined_calls/provider.rs:361-454`) | `HashMap<(FileId, u32, u32), Vec<&CallSiteFact>>` by byte span; `HashMap<(FileId, &str), Vec<&FunctionFact>>` for callers | file, span; file, name | once per provider run |
| `owner_symbol` (`calls/extract.rs:554-567`) | `HashMap<(FileId, &str, Span), SymbolId>` | file, name, span | once per provider run |

Hash maps are lookup-only, as the repository's rule already states for the X6 change ("Hash maps are lookup-only; output order never follows hash iteration", `research/deep-analysis-performance/FINAL-REPORT.md`, retained mechanisms). Every replaced scan has first-match semantics that the index must reproduce, including `matching_function`'s "first function in `db.functions()` order that matches", which the bucket preserves by keeping insertion order.

### W2. Stream every provider digest

- Build: apply the X5a family-prefix streaming to the six sorted-`Vec<String>` digests (`analysis/provider.rs:190`, `domains/provider.rs:224`, `types/provider.rs:300`, `calls/provider.rs:186`, `refined_calls/provider.rs:657`, `data_flow/provider.rs:466`), reusing `evidence/provider.rs:570-649` and its `family_prefixes_partition_the_sorted_order` property test. Remove the four whole-program `BTreeMap<Id, String>` tables in `domains/provider.rs:248-297` in favour of resolving through the interner at emission.
- Why: removes the transient peaks of section 3.3 without changing a digest byte.
- Probe: G1; the `peak_rss_mb` minus `rss_mb` gap on the `semantic_mir` and `abstract_domains` stage rows shrinks to the retained size.
- Depends on: nothing; parallel to W1.

### W3. Demand at fact-family granularity, and an honest domain solver

- Build: the capability closure (`analysis_kernel/provider.rs:1094-1130`) seeds and closes over families, not providers, so that `calls` pulls `summary_call` and whatever `summary_call` reads, and not `domain_observations`. The reading is done (Q4): observations reach only `build_control_effects`, where they set `DoesNotReturn` (`summaries/builder.rs:420-431`), and `refined_calls` consumes `CallEffects` summaries only (`refined_calls/summaries.rs:19-20`), so the family split is `summary_control` (needs domains) versus the other four (do not), declared per output family in the manifest rather than per provider. The manifest edit that makes this true covers both domain families: `polint.type_value_alias` declares `domain_observations` and `domain_events` as inputs (`analysis_kernel/provider.rs:1688-1689`) and reads neither, and `polint.abstract_domains` is the sole producer of both (`:1561`), so both rows leave that manifest, and `direct_summaries` declares both against `summary_control` only. The kernel's closure parity is enforced today by a `debug_assert_eq!` against `providers_enabled_by_boolean_gates` (`analysis_kernel/mod.rs:603-607`, function at `provider.rs:1031-1068`) and by the exhaustive 128-subset test `capability_closure_matches_boolean_pipeline_gates` (`provider.rs:1999-2025`); the behaviour commit deletes the boolean-gate function and the assert and replaces the parity test with an explicit expected table, which is a stated behaviour change, not an expectation edit. In the same workstream, make `IdeDomainSolver` per-function by default (intraprocedural, which is what the L3 domains are documented as in report 02 section 3.2) with the interprocedural call-string mode behind the summaries request, and replace the fixed `max_iterations: 10_000` (`domains/solver.rs:84`) with a per-function iteration cap plus a per-run total, the convention the summaries closure and the points-to solver already use (Q5), reporting which functions were cut. Rules that request `control_flow` for guard policies continue to get domains.
- Why: on every measured run the domain solver had already exhausted its budget before it produced its rows (benchmark report 5.5.1), and those rows are 80 percent of all facts on the 45-file scope and the fourth-largest family at 885 files (section 3.9); `calls` should cost a call graph. Because `direct_summaries`, `type_value_alias`, `semantic_graph`, `solver` and `refined_calls` fold the `abstract_domains` output digest into their own (section 6, constraint 1), those five provider digests move on a `calls` request once domains stop running; the fact rows they produce must not, which G1c checks.
- Probe: G5; fact counts on the 45-file and 885-file scopes; the budget row in `polint unknowns`.
- Depends on: nothing for the closure change; W1 for the per-function solver to be measured fairly.

### W4. Dominators from the reverse-postorder algorithm, tree only

- Build: replace `dominator_relation_with_extra_exit` (`cfg/derived.rs:339-400`) with the Cooper-Harvey-Kennedy iteration over an `idom: Vec<Option<BlockIndex>>` in reverse postorder (the CFG already records `reverse_postorder` per block, `cfg/facts.rs` `BasicBlockFact`), for both directions; emit tree edges always and the closure only when the budget allows, computing the closure from the tree rather than the other way round. Control dependence follows from the post-dominator tree (`derived.rs:203-257`). The two directions do not share a universe today and the replacement must keep that: forward dominance runs over the blocks reachable from the entry (`derived.rs:82-83`, `reachable_blocks` at `:309-321`), while post-dominance runs over every block of the function plus a virtual exit that the selected exit blocks feed (`:133-154`, `:404-425`), and a block with no path to any exit reaches the fixpoint with the whole universe as its post-dominator set (`:346-356` seeds every non-root set with `universe.clone()`), so today it is emitted as post-dominated by every block, a vacuous lattice-top row rather than a tree. Those rows are reproduced exactly when the closure is emitted (an explicit "exit-unreachable" case that emits the universe minus the virtual exit and the `immediate` flag `immediate_relation` derives for it, `:438-462`), so that the `POLINT_CFG_MAX_DOMINANCE_PAIRS=0` pair set is byte-identical; whether they should exist at all is a separate, reported semantic change and is not part of W4.
- Why: section 3.5; the `dominators` step was 11 s on the bounded path and the relation compute is quadratic per function.
- Probe: G4; digest identity is expected to change only when the budget is disabled (`POLINT_CFG_MAX_DOMINANCE_PAIRS=0`) and the closure is emitted, in which case the set of pairs must be identical.
- Depends on: nothing.

### W5. Structural identity behind the resolver contract (the pivot)

- Build: `StableKeyId` becomes an index into a per-family arena of `(family, parent StableKeyId, discriminating parts)` nodes; `resolve` materialises canonical text by walking parents into the existing `write_stable_key_text` buffer, with a bounded boundary cache; `payload_digest` is computed by streaming the same walk into the FNV state, using the composable-summary technique `NEXT-LEVER.md` proved if the walk cost shows up; `FactMeta::payload_digest` becomes a `u64`. All 35 `semantic_stable_key` call sites construct nodes instead of strings. Ordering by canonical text is preserved by comparing streamed prefixes.
- Why: section 3.1; this is the retained floor and the reason every join needed a side table. It is the one change that alters the memory curve's slope rather than its intercept.
- Probe: every provider digest and every `polint check` output byte-identical on the fixture suite, excalidraw, and the consumer scopes; `key_mb` on the full backend `semantic_mir` row drops by an order of magnitude (G3).
- Depends on: W1 and W2, so that the pivot is measured against a pipeline whose other costs are already linear; the proof obligations in `NEXT-LEVER.md`.

### W6. Per-unit lowering, arenas, and parallel units

- Build: the unit is a Go package (from `polint.module_topology`) or a TS project (from the TS lifecycle's `tsconfig` discovery, which PR #121 already implements for the sidecar); `lower_go_mir` and `lower_ts_mir` become `lower_unit` producing a `UnitGraph` with dense local ids in arenas; CFG, per-function domains and direct call extraction run inside the unit; units run in parallel under `rayon` with the merge order fixed by unit path; whole-program `SemanticStore`, `CfgFactStore` and the per-run `normalized` sorts are deleted in the same PR that lands the unit graph. The `AnalysisHost` accessors that return `&[Fact]` are replaced by unit-aware iterators.
- Why: makes the largest stages linear in the unit and parallel across units; removes the whole-program sorts; gives the shard boundary of W7 a natural key.
- Probe: G3, G4, G6 (first attempt), G7, G8.
- Depends on: W5 (identity must be structural before ids can be unit-local), W1, W3.

W5's proof obligations, adopted from `NEXT-LEVER.md` and restated as a checklist a reviewer can tick:

1. For every fact family, the canonical byte stream produced by walking the structural identity equals the string produced today by `write_stable_key_text`, on every fixture, on excalidraw, and on the consumer scopes; the comparison is byte-for-byte, not digest-for-digest.
2. Every provider output digest and every `polint check` diagnostics digest is unchanged (`.scale-envelope/digests.py` over the `stage done` rows).
3. Duplicate-identity and sparse-id behaviour in `FactMetaStore` (`analysis_api/metadata.rs:360-392`) is unchanged; the conflict set on every corpus is identical.
4. Backslash normalisation and length prefixes (`analysis_api/metadata.rs:525-542`) remain in the boundary text.
5. The public resolver (`resolve_stable_key`, `sdk/facts.rs:490-566`) returns the same `Arc<str>` text; the count of boundary materialisations is measured and reported so that "memory improved" is a measured claim.
6. Nested identities are walked iteratively, not recursively, and encoded lengths are computed with checked arithmetic.

W6's determinism rules:

1. Units are numbered by sorted unit path; ids inside a unit are assigned in the lowerer's existing deterministic traversal order (sorted by relative path then byte offset, `go/mir/lower.rs:37`, `:517-530`).
2. The merge of unit outputs into any whole-program view iterates units in unit order; no output order ever follows a hash map or a thread schedule.
3. The N=10 permutation gate runs with unit scheduling permuted, cache cold and warm, on the 885-file scope, before per-unit parallelism is enabled by default.

### W7. Persisted unit shards

- Build: a unit's graph serialises to one shard file under `.polint/cache/layers` keyed by the unit's input digests (the layer cache's `LayerKey` and `InputSnapshot` machinery, `analysis_kernel/incremental/keys.rs`, `layer_cache.rs`); a cold run writes shards, a warm run loads unchanged shards and re-lowers only changed units; Phase 66's "validated fact and graph ingest" and Phase 67's summary manifests attach here, with SQLite used for manifests and cross-unit indexes and the shard payloads as one binary columnar blob per unit (Q3). The layer cache's `serde_json::to_vec` payload encoding (`layer_cache.rs:218`, `:385`) is replaced for shard payloads, and the 64 MB manifest and payload ceilings (`:31-32`) are revisited; blob-in-cache versus adjacent content-addressed file is decided by the SUM-03 benchmark before the layout is locked. W7 and W8 are in scope for this track (Q1) but G6 and G8 do not depend on them.
- Why: warm review; and the precondition for cross-unit joins that do not need every unit in memory.
- Probe: G9; the stale-reuse mutation fixtures report 03 names (VAL-04).
- Depends on: W6.

### W8. Cross-unit demand joins for calls, RTA, reachability and data flow

- Build: `polint.solver`'s RTA, `refined_calls`, `reachability` and `data_flow` read unit shards through a cross-unit index (function by qualified name, call site by unit and local id) and process Go units in import order, which is a topological order because Go forbids import cycles, and TS projects by project reference order with intra-project SCCs handled by the existing SCC closure (`summaries/closure.rs:87`); summaries at unit boundaries are the persisted `summary_*` families of Phase 67.
- Why: this is where "deep analysis per scope, graph persisted and joined across scopes on demand" becomes true for the interprocedural capabilities.
- Probe: G6 at the final threshold; G9 with the dependents recompute asserted.
- Depends on: W7; Phase 67's manifests.

### W9. Envelope enforcement and the consumer gate

- Build: extend `ResourceEnvelope` (`analysis_kernel/resource.rs`) with a wall-clock budget and a per-unit memory check; ship the G6 and G8 probes as local commands the owner runs on a machine of his choice, and record each run's results (counts and timings only) as a committed report. No hosted, self-hosted, or scheduled CI of any kind runs the gate (owner decision, Resolved Q6).
- Why: report 03 Stage 3's "envelope enforced with reported degradation"; and the acceptance gate needs a place to run.
- Probe: G10; G6 and G8 as the job's pass condition.
- Depends on: W6 for the per-unit check; nothing for the workflow itself.

### What each workstream should show in the stage rows

The stage rows (`RUST_LOG=polint=debug`, target `polint::kernel::stage`, `analysis_kernel/mod.rs:327-341`) are the instrument; this table says what a reviewer should expect to see change, on the 885-file scope unless stated, so that a workstream that lands without moving its row is a workstream that did not do what it claims.

| Workstream | Row | Expected movement | Must not move |
|---|---|---|---|
| W0 | none (rule host, not a provider) | second cold run shows no `cargo` child and restores in seconds | analysis stage rows |
| W1 | `polint.semantic_mir` `elapsed_ms`; `polint.refined_calls` `elapsed_ms`; `polint.abstract_domains` `elapsed_ms` (via `Icfg::build`) | large drops; on the full backend `semantic_mir` completes | every `digest=` value; `facts`; `keys`; `key_mb` |
| W2 | `peak_rss_mb` minus `rss_mb` on `semantic_mir`, `abstract_domains`, `type_value_alias`, `calls`, `refined_calls`, `data_flow` | transient gap shrinks toward zero | `digest=`; `rss_mb` |
| W3 | presence of the `polint.abstract_domains` row on a `calls`-only request; `facts` total; the `digest=` of `direct_summaries`, `type_value_alias`, `semantic_graph`, `solver`, `refined_calls` | domains absent on `calls`; total facts fall by the domain family size; those five digests move because their recipes fold the now-absent `abstract_domains=` digest | every other `digest=`; G1c fact rows of every family except `domain_*`; diagnostics digest; stdout (Q4) |
| W4 | `polint.cfg` step rows `dominators`, `postdominators`, `control_dependence` | near-linear; `elapsed_ms` drops | every `digest=` in both modes: with `POLINT_CFG_MAX_DOMINANCE_PAIRS=0` the full pair sets, vacuous exit-unreachable rows included, are identical; with the default bound the tree-edge sets are identical |
| W5 | `key_mb` on every row | order-of-magnitude drop; `keys` becomes a node count and is reported, not gated (it equals today's count only if every text that is embedded as a parent is also interned standalone, which holds for the recipes read here but is not asserted) | every `digest=`; `facts`; G1c canonical key text |
| W6 | `polint.semantic_mir` and `polint.cfg` rows are replaced by `polint.unit_graphs`; its `elapsed_ms` and `rss_delta_mb`; busy cores | parallel speed-up; retained delta per unit, not per program; every downstream provider digest moves because the recipes name the four deleted providers (`entrypoints/provider.rs:92-94`, `summaries/provider.rs:39-42`, `types/provider.rs:218-222`, `data_flow/provider.rs:417-422`, `evidence/provider.rs:611-616`, and the `dependency_digest` sites at `analysis_kernel/provider.rs:428`, `:513-516`, `:585-587`, `:620`, `:683-687`, `:724-734`, `:761-763`, `:789-794`, `:819-826`, `:851-859`) and are rewritten to name `polint.unit_graphs` | G1c fact rows of every family; diagnostics digest; ai-friendly stdout bytes across permutations |
| W7 | second-run `elapsed_ms` for unit-level providers | near zero for unchanged units | first-run rows |
| W8 | `polint.refined_calls`, `polint.solver`, `polint.reachability`, `polint.data_flow` | bounded by touched units | precision and status distributions in the ai-friendly report |
| W9 | `polint/resource-budget` diagnostic under a low ceiling | present, with the degraded capabilities named | rows under a normal ceiling |

### Dependency graph

```
W0 ─────────────────────────────────────────────────────────── (independent)
W1 ─┐
W2 ─┤
W3 ─┼──> W5 (identity pivot) ──> W6 (units, arenas, parallel) ──> W7 (shards) ──> W8 (cross-unit joins)
W4 ─┘                                                             └──> W9 (envelope, gate)
```

Relationship to report 03: W1 to W4 are Stage 0 "scale root cause" done properly; W5 and W6 are new and sit between Stage 0 and Stage 2; W7 and W8 are Stage 2's keystone (Phases 66, 67, 68) restated on top of unit shards; W9 is Stage 3's envelope. The L4 certification items of Stage 1 (TS type tier, IFDS tabulation, access paths, models as data) are unaffected in content and should proceed on the unit graph once W6 lands, because tabulation over a per-unit ICFG with persisted summaries is the shape IFDS was designed for.

## 9. Position of the TypeScript type sidecar (PR #121)

The branch `feat/ts-type-sidecar` adds a Node process on the TypeScript compiler API that emits per-call-site receiver type, resolved signature and callee rows, a `polint.ts.types` provider, and a `TypeDirected` tier in `polint.refined_calls`; it moves the bounded subprocess runner and sidecar cache out of `go/` into a shared `subprocess/` module. Its own measurement record reports +2,510 call sites resolved on the Jelly `src/` tree (828 to 3,338), zero lost, every existing tier byte-identical in count, +13.4 percent pipeline wall of which the sidecar is 14.9 s, a 340-times warm speedup from the persisted NDJSON, and no change on the Jelly micro oracle because that oracle's snippets are outside any `tsconfig` (`research/ts-type-sidecar/measurement.md` on that branch, sections 1 to 3).

Positioning against this document:

- It is a resolution tier, not a graph stage. It does not touch `semantic_mir`, `cfg` or `abstract_domains`, so it neither contributes to nor relieves the wall. The full-frontend `calls` run dies in `polint.cfg` after a 203 s `semantic_mir` (benchmark report, section 5.4) with or without it.
- It follows the sidecar pattern this document endorses: an out-of-process type checker whose output is persisted by content digest and joined into the Rust graph. W6's TS unit is the `tsconfig` project the sidecar already discovers, so the two designs converge on the same unit.
- Its join is the same shape as the Go semantic join in section 3.2 (sidecar row to native call site by span). The branch indexed that join by file and reports no measurable wall change on a 265-file repository, which is consistent with section 3.2's claim that these terms only dominate past about a thousand files. W1 should treat both joins identically.
- The TS lowerer carries the same quadratic scans as the Go lowerer (`ts/mir/lower.rs:132-141`, `:764`, `:4540-4581`) and a symbol graph that is far more expensive than Go's (37.3 s for 2,381 files, benchmark report A.6); W1 covers the lowerer, and the symbol-graph cost is a separate item that this document notes but does not root-cause.

Nothing here re-researches the TS deep-analysis question; report 02 item 2 and the branch's `plan.md` own it.

## 10. Risks

| Risk | Where it bites | Mitigation | Kill criterion |
|---|---|---|---|
| The identity pivot (W5) changes a digest or a key text | every persisted layer, every baseline, `evidence_v1` replay keys | the canonical text recipe is unchanged; differential test of streamed versus materialised text for every family; W5 ships behind the resolver contract with the old path deleted only after the oracle passes on all corpora | any digest differs on any corpus |
| Per-unit lowering loses a cross-file fact the whole-program lowerer produced | closure captures across files, module-level functions, TS re-exports | unit inputs include the symbol graph for the whole program (cheap: 0.8 s on Go); cross-unit references resolve through it; fixtures per construct | any L2 or L3 probe regresses |
| Parallel units break determinism | merge order, id assignment | ids are unit-local; merge in unit path order; the N=10 permutation gate runs on the 885-file scope | any byte differs between orders |
| Family-granular demand hides a real dependency | a summary family silently loses precision when domains are not run | the summaries builder declares per-family inputs explicitly; a test asserts that each family's output is identical with and without the un-demanded families present | any summary digest differs |
| Shard write cost dominates cold runs | W7 | measure serialisation per unit before adopting a format; columnar blobs, one file per unit, no per-row SQL | shard writing exceeds 20 percent of cold wall |
| The full backend still exceeds 12 GB after W6 because of `type_value_alias` and `refined_calls` | benchmark report A.4 shows those two at 19.1 s and 26.0 s and +3 GB at 885 files; hugo showed `type_value_alias` at 70 percent of wall (`.scale-envelope/EXPERIMENTS.md`, hugo section) | they are on the W8 path and inherit W5's identity and W1's indexes; re-profile after W6 before designing W8's joins | either stage exceeds 60 s on the full backend after W6 |
| A TS file belongs to no `tsconfig` project | W6's TS unit | Q7's fallback to a file unit; the sidecar already reports the skip with a diagnostic (PR #121's project-ownership skip) | more than 10 percent of a repository's TS files fall back on the consumer frontend |
| The consumer's synthetic `go.work` or generated code outside module roots produces units the package DAG does not order | W6, W8 | treat unordered units as their own SCC after all ordered units; report them as `setup_aware` | any unit's facts change between two orderings |
| Scope collapse | any workstream | one invariant per PR; the digest oracle as the merge condition; report 03's 1,500-line and 25-file limits | a PR over budget is split |

## 11. What was validated and what was not

Validated in this session:

- Every `file:line` citation was re-read at `686461be` after the trace; the cited lines were checked again by grep before the final commit.
- The five causes were traced to the loop structure and data structures that produce them; the quadratic terms are visible in the code without profiling.
- The summaries builder's use of domain observations was checked (section 3.4), which turns W3 from a hypothesis into a mechanical change.
- The public contract boundary was checked against `docs/facts/` and the SDK sources (section 5 preamble), so the "no SDK break" claim for options A and D rests on the documented API, not on assumption.
- External claims in section 4 were fetched from the primary URL given; each unverifiable claim is marked in place.
- For the resolved questions: the crate's public surface was read from `lib.rs` and `docs/API-VISIBILITY-PLAN.md` (Q2); every non-test consumer of domain observations and control summaries was located by grep and read (Q4); every provider's `cache_policy` and the layer cache's payload encoding were read (Q1, Q3); the budget conventions of the summaries closure, the points-to solver and the data-flow search were read (Q5); the TS lifecycle on the sidecar branch was read (Q7); GitHub's runner specifications were fetched from the reference pages (Q9).

Not validated, and stated as such:

- No build or benchmark was run. The cost split in section 3.8 is inference; G1b is the probe that confirms or corrects it.
- Corrected after the adversarial review (round 1): the first version treated a provider's `digest=` as a function of its rows; it is a function of its rows and of the named upstream digests, so the digest invariants for W3 and W6 were unsatisfiable as written and are now stated at the fact-row level (section 6, constraint 1; G1c). The same review found the `domain_events` input row, the post-dominance universe asymmetry, the tree-versus-process memory conflation in G2, and the `/usr/bin/time` measurement gap; each is corrected in place.
- `polint.type_value_alias`, `polint.semantic_graph`, `polint.symbol_graph` on TS, and the Rust-side lowering of sidecar rows inside `polint.go.semantic` were not traced line by line; section 3.9 records their measured cost and the causes they visibly share.
- The claim that `summary_call` is unchanged when domains are absent is a reading of the builder, not a digest comparison; Q4 keeps the oracle as arbiter.
- The 12 GB and 300 s gate thresholds are defended by ratio arguments from measured stages, not by a model of the re-architected pipeline; the intermediate gates exist so that the final gate is approached with measurements.
- The SUM-03 layout benchmark (Q3) and the share of consumer TS files outside any `tsconfig` project (Q7) were not run or measured.
- Whether the Go package DAG is the right unit for every consumer layout (for example generated code outside the module roots, or `go.work` synthesis) was not examined; the benchmark report notes polint writes a synthetic `go.work` for the consumer's seven modules.

## References

- Benchmark report: `/workspace/polint-bench-20260917/report.md` (local artifact, 2026-09-18; sections 2, 4.2, 5.3, 5.3.1, 5.3.2, 5.4, 5.5.1, 5.6, appendix A).
- `.scale-envelope/EXPERIMENTS.md` (X1 to X6, final result, hugo section); `research/deep-analysis-performance/FINAL-REPORT.md` and `NEXT-LEVER.md`.
- `research/strategy/02-gap-analysis.md` sections 3.2, 4, 5, 7; `research/strategy/03-build-plan.md` sections 1, 3, 4, 5.
- `research/ts-type-sidecar/README.md`, `plan.md`, `measurement.md` on `origin/feat/ts-type-sidecar` (PR #121).
- `docs/facts/capability-plans.md`, `docs/facts/calls.md`, `docs/facts/control-flow.md`, `docs/facts/evidence.md`.
- Code, all at `686461be`: `crates/polint/src/analysis_kernel/{mod.rs,provider.rs,resource.rs}`, `analysis_neutral/{host.rs,store.rs,places.rs,mir_body_compose.rs}`, `analysis_neutral/cfg/{provider.rs,lower.rs,derived.rs,budget.rs,store.rs}`, `analysis_neutral/domains/{provider.rs,solver.rs,store.rs}`, `analysis_neutral/ifds/mod.rs`, `analysis_neutral/refined_calls/provider.rs`, `analysis_neutral/calls/extract.rs`, `analysis/provider.rs`, `go/mir/lower.rs`, `ts/mir/lower.rs`, `go/lifecycle.rs`, `go/semantic/{prefetch.rs,client.rs}`, `core/{db.rs,metadata.rs,rule.rs}`, `internal_core/stable_key.rs`, `analysis_api/metadata.rs`, `analysis_api/digest/keys.rs`, `ir/body.rs`, `cache/rules_store.rs`, `sdk/{facts.rs,policy.rs}`.
- External sources: listed inline in section 4.

## Appendix A. Evidence index: cause to code

For future verification, every code location this document relies on, grouped by cause, at `686461be`.

| Cause | Location | What it shows |
|---|---|---|
| 3.1 | `internal_core/stable_key.rs:24-41` | interner state, dense ids, "cannot be sharded" |
| 3.1 | `internal_core/stable_key.rs:62-73`, `:78-88`, `:104-112` | `intern`, `intern_and_resolve` (write lock), `resolve` (Arc clone) |
| 3.1 | `analysis_api/metadata.rs:231-239`, `:353-357`, `:360-392`, `:512-542` | `FactMeta` with `payload_digest: String`; `FactMetaStore`; insert with owner map; canonical text writer |
| 3.1 | `core/metadata.rs:301-321`, `:380-398` | metadata from stable key; payload digest over formatted parts |
| 3.1 | `core/db.rs:2706-2750`, `:4578-4640` | MIR metadata rebuilt per replace, formatting keys per row |
| 3.1 | `go/mir/lower.rs:629-641`, `:2238-2260`, `:2335-2347`; `analysis_neutral/places.rs:135-196`; `cfg/derived.rs:96-104`; `domains/store.rs:499-509` | key composition recipes: body, operation, owner, place, dominator, observation |
| 3.1 | `cfg/budget.rs:1-19` | "roughly 1.4 KB of interned identity text per pair" |
| 3.2 | `go/mir/lower.rs:2317-2329`; `ts/mir/lower.rs:4540-4581` | `matching_function`, `enclosing_function` whole-program scans |
| 3.2 | `go/mir/lower.rs:645-656` | per-body package and module scans |
| 3.2 | `go/mir/lower.rs:732-767`; `ts/mir/lower.rs:764`; `analysis_neutral/host.rs:129-152`; `core/db.rs:3730`, `:3746-3753`, `:6043` | closure captures through trait-default scans while an index exists |
| 3.2 | `go/mir/lower.rs:115-124`; `ts/mir/lower.rs:132-141` | per-body filter over all operations |
| 3.2 | `analysis_neutral/ifds/mod.rs:93-98` | per-call-site scan of CFG nodes |
| 3.2 | `analysis_neutral/refined_calls/provider.rs:333-340`, `:361-454` | per-sidecar-row scan of call sites and functions |
| 3.2 | `analysis_neutral/calls/extract.rs:554-567` | per-call-site scan of symbols |
| 3.2 | `analysis_neutral/cfg/lower.rs:40-97` | the indexed counter-example |
| 3.3 | `analysis/provider.rs:85-190`; `domains/provider.rs:120-224`, `:248-297`; `types/provider.rs:300`; `calls/provider.rs:186`; `refined_calls/provider.rs:657`; `data_flow/provider.rs:466` | sorted `Vec<String>` digests and key side tables |
| 3.3 | `evidence/provider.rs:570-649`; `cfg/provider.rs:155-330` | the streaming counter-examples |
| 3.4 | `domains/solver.rs:61-65`, `:84`, `:113-135`, `:146-157`, `:222`, `:247`, `:662-690`, `:700-704` | exploded point with call stack; budget; whole-program ICFG; budget trip; stack clone and push; mark-all-exceeded; nested materialisation loop |
| 3.4 | `domains/store.rs:69-125`, `:316-380` | observation fan-out |
| 3.4 | `analysis_kernel/provider.rs:1545-1567`, `:1583-1584`, `:1688`, `:1823-1824`; `summaries/builder.rs:75-81`, `:133-147`, `:347`, `:422` | why domains are on the `calls` path and which family reads them |
| 3.5 | `cfg/derived.rs:66-118`, `:339-400`, `:438-462`; `cfg/provider.rs:96-129`; `cfg/budget.rs:30`, `:66-79` | pair emission; naive relation; immediate derivation; bound applied after compute; ceiling and estimate |
| 3.6 | `analysis_kernel/mod.rs:220-300`, `:260`, `:283-291`, `:327-341`; `core/rule.rs:437` | sequential loop; single `&mut db`; gauge; the only rayon site |
| 3.6 | `go/mir/lower.rs:28-101`, `:500-507`; `ir/body.rs:123-160`; `analysis_neutral/store.rs:33-118`, `:191-235` | sequential lowering; re-parse; triple normalisation; id remap tables |
| 3.7 | `go/semantic/prefetch.rs:1-23`; `go/lifecycle.rs:29-37`, `:163-166`; `go/semantic/client.rs:206` | sidecar overlap; release-build RTA gate |
| gating | `analysis_kernel/provider.rs:1013-1028`, `:1031-1068`, `:1070-1090`, `:1094-1130`; `analysis_kernel/mod.rs:549-566`, `:603-607`; `provider.rs:1999-2025`, `:2027-2052` | seeds, boolean gates, closure, cross-file gate; closure-parity `debug_assert_eq!`; 128-subset parity test; v13 ledger test |
| digest recipes | `analysis_kernel/provider.rs:69-74` (`dependency_digest`, `absent` for a provider that did not run); `summaries/provider.rs:39-42`; `types/provider.rs:196`, `:218-222`; `refined_calls/provider.rs:611-616`, `:627-632`; `entrypoints/provider.rs:92-94`; `domains/provider.rs:158-160`; `calls/provider.rs:115-116`; `data_flow/provider.rs:417-422`; `evidence/provider.rs:611-616`; `dependency_digest` call sites per provider at `analysis_kernel/provider.rs:428`, `:463-470`, `:513-521`, `:550-555`, `:585-592`, `:620-624`, `:683-694`, `:724-735`, `:761-763`, `:789-794`, `:819-826`, `:851-859` | a provider digest is a function of its rows and of the named upstream digests |
| measurement | `measure.rs:27-32`; `.scale-envelope/rssrun.py:22-58`, `:75-107` | polint-process peak (`RUSAGE_SELF`); tree peak sampler |
| post-dominance universe | `cfg/derived.rs:82-83`, `:133-154`, `:309-321`, `:346-356`, `:404-425`, `:438-462`, `:464-509` | forward universe is entry-reachable; reverse universe is all blocks plus a virtual exit; vacuous rows for exit-unreachable blocks; immediate derivation; selected exits |
| contract | `docs/facts/capability-plans.md`; `docs/facts/calls.md:11-13`; `docs/facts/evidence.md:55`; `docs/facts/control-flow.md:120-122`; `sdk/facts.rs:490-566`; `sdk/policy.rs:1029-1063` | what is public |
| W0 | `cache/rules_store.rs:797-806`, `:827-834`, `:1098-1110`, `:1190-1197` | path digest of cargo home; config content hashing; helper; shareability |
| envelope | `analysis_kernel/resource.rs:1-90` | memory ceiling at provider boundaries |
| Q1 | `analysis_kernel/provider.rs:1328`, `:1347`, `:1369-1930`; `cache/analysis_cache_adapter.rs:154-155` | only syntax providers persist; every deep provider is `InMemoryDerived` |
| Q2 | `lib.rs:9-12`, `:19-47`; `docs/API-VISIBILITY-PLAN.md` | analysis modules are `pub(crate)`; only `runner`, `sdk`, `rule` are public |
| Q3 | `analysis_kernel/incremental/layer_cache.rs:31-32`, `:218`, `:385`; `.planning/REQUIREMENTS.md:73`, `:100-106` | JSON payloads, 64 MB ceilings; PERF-02, SUM-01 to SUM-07 |
| Q4 | `summaries/builder.rs:420-431`; `refined_calls/summaries.rs:19-20`, `:167`; `data_flow/summary_edges.rs:433`, `:548` | observations set `DoesNotReturn` only; refined calls read `CallEffects` only; other mentions are tests |
| Q5 | `summaries/closure.rs:51`, `:116-125`; `solver/budget.rs:23-50`; `ifds/mod.rs:31-43` | per-SCC, per-sub-domain and per-query budget conventions |
| Q7 | `ts/types/lifecycle.rs:32`, `:40`, `:144-165` (on `origin/feat/ts-type-sidecar`); `summaries/scc.rs:3-32` | nearest-tsconfig partition with an uncovered-file list; Tarjan SCC scheduler |
| cache keys | `analysis_kernel/incremental/keys.rs:51-62`; `analysis_api/digest/keys.rs:9-21`; `incremental/layer_cache.rs:31-32` | layer key fields; layer kinds; payload ceiling |

## Appendix B. Numbers quoted from the benchmark report

| Figure | Value | Report section |
|---|---|---|
| production scan capability set | `requested_capabilities={}`; 17 of 23 providers never start | 1, 4.3, 5.1 |
| warm production backend scan | 4.39 s, 2,348 files; provider pipeline 1.33 s | 4, 6.1 |
| cold rule-pack compile | 193 s; store hit 5.14 s; cargo home path in key | 4, 4.2 |
| sidecar phases (1,020 packages, 7,011 files) | load 9.7 s; SSA 5.2 s; emit 10.1 s; total 25.0 s; 7.0 GB heap; `rta_analyze` 0 ms | 5.3.2, A.3 |
| sidecar total across scopes | 25 to 27 s at every scope | 5.6 note |
| upstream pipeline, full backend | 13.6 s through `module_topology`; `go.syntax` 5.8 s | 5.3 |
| scaling curve | 45: 68.3 s / 11.0 GB; 314: 91.2 s / 12.3 GB; 885: 152.3 s / 13.7 GB; 1,588: timeout, 15.1 GB; 4,752: timeout, 14.6 GB | 5.3.1 |
| `semantic_mir` per scope | 468 ms; 5,922 ms; 15,432 ms; 88,971 ms; >287,000 ms; 565,269 ms (over budget) | 5.3.1, 5.6 |
| `cfg` per scope | 1,852 ms; 9,983 ms; 20,783 ms; 65,078 ms | 5.3.1 |
| over-budget full backend | 634.6 s; 18,304 MB; SIGKILL in `cfg`; 2,926,814 facts; 5,525,029 keys; 3,448 MB key text; `cfg` steps 30.2 s, 5.4 s, 11.2 s | 5.6 |
| dominance bound | 1,614,420 pairs at 314 files; 4,111,312 at 885; ceiling 250,000 | 5.5.1 |
| 45-file stage table | `abstract_domains` 15,285 ms, +732,066 facts, 274 to 1,736 MB, peak 5,024 MB; `solver` 146 ms, +1,497 | A.3 |
| 885-file stage table | the twelve rows in section 3.9 | A.4 |
| frontend | `semantic_mir` 203 s for 2,381 files; `symbol_graph` 37.3 s; all deep capabilities time out | 5.4, A.6 |
| busy cores | at most 8.12 of 16 | 2 |

## Resolved Questions (researched)

The nine questions the first commit left open were researched against the code at `686461be`, the sibling strategy reports, the v2.0 requirements, and external primary sources. Eight are resolved here; each carries the option chosen, the evidence, what it costs to reverse, and a confidence label. One remains under Open Questions because it turns on spend and security posture, not on evidence.

Q1. Is a warm re-scan (W7, W8) in scope for this track, or is the cold full-application gate (G6, G8) the whole commission?
Context: W1 to W6 deliver the cold gate; W7 and W8 are what report 03 calls the Stage 2 keystone and they are where the persisted-store decisions live. Sequencing them here commits the store format.
Options: (a) cold gate only, W7 and W8 stay in Stage 2 as written; (b) include W7 and W8 so that the unit shard is designed once; (c) include W7 only.
Answer: (b) — because nothing deep is persisted today and the unit graph W6 builds is the shard's in-memory form, so designing it once is the only path that does not do the work twice. The default is confirmed.
Evidence: every deep provider from `polint.module_graph` to `polint.metrics` declares `CachePolicy::InMemoryDerived` (`analysis_kernel/provider.rs:1369-1930`); only the two syntax providers use the file fact cache (`:1328`, `:1347`) and the layer cache is wired to `GoSyntax` and `TsSyntax` layers alone (`cache/analysis_cache_adapter.rs:154-155`). So "warm" means nothing for the deep stack until a shard exists. Report 03 names Phase 67 "the keystone" and report 05 names summary-persisted, frontier-driven analysis "the single feature that strengthens" the weakest moat (`research/strategy/05-moat-economics.md:14`, `:61-63`); its requirements SUM-01 to SUM-07 and REV-01 to REV-03 are all unchecked (`.planning/REQUIREMENTS.md:100-114`). W7 and W8 are not prerequisites for G6 or G8, which W1 to W6 must pass on their own, so including them adds scope without delaying the cold gate.
Reversal cost: low. If W7 and W8 are later moved back to Stage 2, nothing in W1 to W6 changes; only the shard serialisation in W7 would wait.
Confidence: high.

Q2. Which internal traits may change without a decision record?
Context: `AnalysisHost` (`analysis_neutral/host.rs`) exposes `&[Fact]` accessors that every provider and the SDK views use; W6 replaces them with unit-aware iterators. `docs/facts/` says raw provider structures are not public, but rule-host binaries compiled against `polint 0.3.x` link the crate.
Options: (a) treat `AnalysisHost` and `AnalysisDb` internals as private and change them freely, bumping the crate minor; (b) require a written decision per report 03 rule 5 for each trait change; (c) freeze the accessors and adapt behind them.
Answer: (a) — because the traits are already crate-private and cannot be named by any downstream crate, so there is no contract to break. The default is confirmed and its "bump the crate minor" clause is dropped: no public item changes.
Evidence: `crates/polint/src/lib.rs:9-12` exports only `runner`, `sdk` and the `rule` macro; `analysis`, `analysis_api`, `analysis_kernel`, `analysis_neutral`, `core`, `internal_core` and `ir` are `pub(crate) mod` (`lib.rs:19-47`). `docs/API-VISIBILITY-PLAN.md` records this as the baseline shape and notes that `unreachable_pub` fires for `pub` items inside those modules precisely because "no downstream crate can name them through `lib.rs`". Rule-host binaries link the crate but reach it through `polint::sdk` and the `#[polint::rule]` macro only, and the rule pack pins its own `polint` version (`.polint/rules/Cargo.toml`, benchmark report section 2), so a host is rebuilt against whatever version it names. Report 03 rule 5 applies to "on-disk schemas, public SDK types, wire protocols"; `AnalysisHost` is none of those. The SDK views (`sdk/facts.rs:490-566`, `sdk/policy.rs`) and `evidence_v1` stay frozen, as the default said.
Reversal cost: none; a decision record can be added at any time if a trait ever becomes public.
Confidence: high.

Q3. Shard storage: columnar blobs in the layer cache, SQLite tables, or both?
Context: the store is schema v5 with two provider mirrors and holds no facts today; Phase 66 planned validated row ingest; the layer cache already persists payloads by digest with a 64 MB ceiling.
Options: (a) unit shards as blobs in the layer cache, SQLite for manifests and cross-unit indexes only; (b) all facts as SQLite rows (option C); (c) a columnar file format per unit with an index in SQLite.
Answer: (a) with (c)'s encoding — one compact binary columnar blob per unit in the layer cache, SQLite for manifests and the cross-unit index — because that is what the repository's own SUM-03 rule requires to be benchmarked and what every scaled reference does. The default's "measured against (c)" is kept: the SUM-03 benchmark decides blob-in-cache versus adjacent file, not this document.
Evidence: the layer cache serialises every payload with `serde_json::to_vec` (`analysis_kernel/incremental/layer_cache.rs:218`, `:385`) under a 64 MB payload ceiling (`:31-32`); a JSON row per fact at 2.9 million facts (benchmark report, section 5.6) would exceed both the ceiling and the memory the shard exists to save, so the encoding must change even under (a). The local-store research already decided SQLite is the canonical store for manifests, identities and graph queries (`research/local-semantic-store/FINAL-REPORT.md`, "Recommendation"), and SUM-03 says "SQLite BLOBs, adjacent content-addressed files, or a hybrid must be benchmarked for DB size, WAL growth, crash behavior, restore behavior, and read latency" before the layout is locked (`.planning/REQUIREMENTS.md:102`); PERF-02 requires bounded, sorted ingest batches (`:73`). Externally, Infer keeps per-procedure summaries as blobs in an SQLite database and Glean stores facts as compact terms in RocksDB rather than rows (section 4.2), Joern abandoned transparent paging for an explicit columnar layout (section 4.3), and SQLite's own guidance bounds a row-per-fact design at about 100 MB per transaction (section 4.12). Option (b) is option C of section 5 and was rejected there on write cost and batch shape.
Reversal cost: medium. The blob encoding sits behind the layer cache's payload boundary and can be swapped by bumping `LAYER_CACHE_MANIFEST_SCHEMA`; moving to row-per-fact later would rewrite the consumers, which is why it is not the choice.
Confidence: medium-high; the SUM-03 benchmark is the remaining check.

Q4. May `calls` stop running `abstract_domains` even if that changes which `summary_call` rows exist?
Context: the summaries builder passes observations only to `build_control_effects`, which feeds `summary_control` (`summaries/builder.rs:133-147`, `:347`); `summary_call` does not read them, and `type_value_alias` declares but never reads them. Removing domains from the `calls` path therefore should not change any refined call edge, but the manifest declarations say otherwise and the digest oracle is the arbiter.
Options: (a) yes, and report the precision change in the ai-friendly output; (b) no, keep domains on the path but make them per-function and cheap; (c) split `summary_call` into a domains-free and a domains-refined family and let demand choose.
Answer: (a) — because the only thing observations change is the `DoesNotReturn` exit kind of `summary_control`, and no consumer on the `calls` path reads `summary_control`. The default is confirmed; the fallback to (c) is withdrawn because the evidence no longer supports a refined-call digest change.
Evidence: inside `build_control_effects` the observations are consulted once, to mark a function `DoesNotReturn` when every exit block's entry is observed `unreachable` (`summaries/builder.rs:420-431`). `refined_calls` reads summary facts filtered to `SummaryDomainKind::CallEffects` (`refined_calls/summaries.rs:19-20`); its only mention of `ControlEffects` is a test asserting that a control summary creates no edge (`:167`). `data_flow`'s only `ControlEffects` mention is inside its test module (`data_flow/summary_edges.rs:548`; tests begin at `:433`), and `policy_queries.rs`, `reachability` and `evidence` contain no reference to `ControlEffects` or `DoesNotReturn`. A `calls` run without domains therefore produces byte-identical `summary_call`, `refined_call_edges` and diagnostics rows; the one row that changes is `summary_control`'s exit set, which `control_flow` and `dataflow` requests still receive because W3's family closure keeps domains on those paths.
Revision after the adversarial review (round 1): Answer: (a) at the fact-row level — supersedes the prior "byte-identical `refined_calls` digest" wording, because a provider digest folds the output digests of the upstream providers its recipe names (`analysis_kernel/provider.rs:69-74`; `summaries/provider.rs:42`; `types/provider.rs:221-222`; `refined_calls/provider.rs:629-632`), so the `direct_summaries`, `type_value_alias`, `semantic_graph`, `solver` and `refined_calls` digests move by construction on a `calls` request once `abstract_domains` is absent. The acceptance check is G1c (fact rows, every family byte-identical except the absent `domain_*`), the diagnostics digest and stdout; G1 applies to every provider outside those five and any other digest moving is a failure.
Reversal cost: low. If G1c ever shows a `refined_call_edges` row change, (c) is a one-manifest split.
Confidence: high for the rows; the digest moves are certain from the recipes.

Q5. Budget semantics for the domain solver.
Context: today one global counter of 10,000 iterations marks every function `BudgetExceeded` once tripped; W3 proposes per-function budgets.
Options: (a) per-function iteration cap with a per-run total; (b) per-function only; (c) keep global but raise it and report.
Answer: (a) — because it is the convention every other bounded stage in the engine already follows. The default is confirmed.
Evidence: the summaries closure budgets per SCC and counts `budget_exceeded_sccs` for the run (`summaries/closure.rs:51`, `:116-125`, `:278-326`); the points-to and Go RTA solvers carry per-sub-domain budget bags that latch `BudgetExceeded` "honestly (D-13) rather than looping unbounded" (`solver/budget.rs:23-50`); data-flow search budgets are per query (`ifds/mod.rs:31-43`). A per-function cap stops one large function from starving every other function, which is what the single global counter does today (`domains/solver.rs:146-157`, `:662-690`), and a per-run total keeps the stage inside the resource envelope. Option (c) buys more facts that section 3.4 shows nobody reads.
Reversal cost: none beyond a constant and a diagnostic label.
Confidence: high.

Q7. Is the TS type sidecar's `tsconfig` project the TS unit for W6, or is the unit the file?
Context: Go's unit is the package and is acyclic by construction; TS projects can contain module cycles and files may belong to no project.
Options: (a) project as unit with intra-project SCCs handled by the closure; (b) file as unit with cross-file joins everywhere; (c) project when present, file otherwise.
Answer: (c) — because the sidecar lifecycle already discovers exactly this partition and reports the remainder. The default is confirmed.
Evidence: on `origin/feat/ts-type-sidecar`, `ts/types/lifecycle.rs` walks every discovered TS/JS file to its nearest `tsconfig.json` (`:144-165`, reusing `ts::module_graph::nearest_tsconfig_path`), keeps the sorted, deduplicated project list (`:32`) and the list of files "with no tsconfig above them" (`:40`), and the sidecar skips a project the scan's files do not belong to with a diagnostic (`research/ts-type-sidecar/measurement.md`, "project-ownership skip"). The design record assigns ownership of tsconfig discovery to `ts/module_graph` and has the sidecar consume "scoped project units" (`research/ts-type-sidecar/plan.md`, decision 4 and Q23). Intra-project cycles are handled by the existing Tarjan SCC scheduler (`analysis_neutral/summaries/scc.rs:3-32`). Externally, gopls uses the package and go/analysis facts flow in import order (section 4.5), and SCIP indexes per document but merges by concatenation (section 4.1), which is the file-unit fallback. The risk row in section 10 already bounds the fallback at 10 percent of a repository's TS files.
Reversal cost: low. A file unit is a degenerate project unit; moving files between the two changes shard keys, not the graph model.
Confidence: medium-high; the share of unclaimed files on the consumer frontend is unmeasured.

Q8. Does the numbering collision with `04-evaluation-as-a-weapon.md` get resolved by renaming this document or by an index note?
Context: the series README lists reports 01 to 06; this document was commissioned as `04-full-app-deep-capability.md`.
Options: (a) keep the name and add an index entry; (b) rename to `07-full-app-deep-capability.md`; (c) rename the evaluation report.
Answer: (a) — because the owner fixed the filename in the commission, and the index row plus the convention below removes the ambiguity. The default is confirmed.
Evidence: the series was created in one commit (`952b46de`, PR #105) and 21 cross-references in `research/strategy/` and `research/README.md` say "report 04" meaning the evaluation report; none refers to this document by number. The README row for this document exists (`research/strategy/README.md:26`). Convention adopted: this document is referred to by its slug, `full-app-deep-capability`, never as "report 04"; the evaluation report keeps "report 04".
Reversal cost: trivial; one `git mv` and one README edit if the owner prefers (b).
Confidence: high.

Q9. On which host are the G-gate thresholds binding?
Context: the thresholds were derived on the benchmark host (16 cores, 30 GB); a 16 GB CI runner class is the target report 02 names; the probe commands pin 12 threads.
Options: (a) the benchmark host is the reference and CI is informational; (b) a 16 GB, 8-thread runner is the reference and the thresholds are re-derived there before W1 starts; (c) both, with the stricter binding.
Answer: (a) for the wall-clock gates, with the memory gate written so that it is host-independent — because the wall gates cannot be re-derived on a host that does not exist yet, while 12 GB was chosen for the 16 GB runner class and holds on any host. The default's "then (c)" is kept for the day W9's job runs.
Evidence: GitHub's standard hosted Linux runners are 4 vCPU and 16 GB for public repositories and 2 vCPU and 8 GB for private repositories (https://docs.github.com/en/actions/reference/runners/github-hosted-runners); larger runners are offered at 8 vCPU / 32 GB and 16 vCPU / 64 GB among other sizes (https://docs.github.com/en/actions/reference/runners/larger-runners). The consumer repository is private, so its standard runner cannot hold a 12 GB tree peak at all; the wall gate at 12 threads has no meaning on 2 or 4 vCPUs; and the benchmark host is the only machine on which every number in section 1.2 was measured. Re-deriving before W1 (option b) would delay the first workstream on a runner choice that Q6 has not made. The memory gate is the one that ports: a 12 GB tree peak fits the 16 GB standard public runner and the 32 GB larger runner with headroom for the sidecar.
Reversal cost: low; thresholds are numbers in section 7 and are re-derived once per host, which the probe matrix already records per cell.
Confidence: medium; the wall gates on any CI host are unmeasured until W9.

Q6. How is the acceptance gate provisioned?
Owner decision (2026-09-19): local-only, full stop. The gate runs when and on whichever machine the owner chooses. It is never executed on GitHub-hosted infrastructure, in CI, on a self-hosted runner, or on any schedule. The deliverable is the probe commands plus a committed report format (counts and timings only, per the hygiene rules). This supersedes the researched default's "until a runner is provisioned" clause: no runner will be provisioned.

## Open Questions

Q10. W6's provider-replacement commit exceeds the 25-file delivery rule by construction; grant a recorded exception, or accept a transitional facade?
Context: the four provider ids W6 deletes are vocabulary strings in 33 files at `026407b7` (`grep -rl` over `crates/polint/src`, `crates/polint-eval/src`, `crates/polint/tests`: the six `analysis/*/provider.rs` files, `analysis/unknown_taxonomy/collect.rs`, `analysis_kernel/{debug,mod,outcome,provider,resource,validation}.rs`, `analysis_kernel/incremental/{keys,run_report}.rs`, `analysis_neutral/{cache_key,error,mod}.rs`, `analysis_neutral/{calls,cfg,demand,domains,semantic_graph}/*`, `core/mod.rs`, `core/tests/batch{1,2}.rs`, `polint-eval/src/harness/{fixtures,mod,observed}.rs`, `tests/cli.rs`), and the v13 ledger test (`analysis_kernel/provider.rs:2027-2052`) requires `analysis_neutral/cache_key.rs` to mirror every manifest change in the same commit. Report 03 rule 1 caps a PR at 25 files; rule 2 forbids a dual path, so the ids cannot be removed in two halves that both keep the tests green.
Options: (a) grant a recorded exception for that one commit, on the ground that a provider id is one vocabulary invariant; (b) accept a transitional facade commit in which `polint.unit_graphs` is registered and delegates to the four existing derive functions while their ids are still registered, followed by the deletion commit, at the cost of one commit that is dual by the letter of rule 2; (c) raise the file cap for vocabulary-only changes generally.
Default if unanswered: (a), recorded in the PR description with the file list.

Every other question is answered; the acceptance-gate provision is an owner decision, recorded under Resolved Questions (Q6).
