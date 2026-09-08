# 06 — Capability Roadmap: What We Gain at Each Stage, in Plain English

Date: 2026-09-01
Researcher: Claude Fable 5.1 (delegated via Hermes)
Revised 2026-09-02: time estimates removed, capability roadmap added.
Reads with: [03-build-plan.md](03-build-plan.md) (the stages and their exit criteria) and [02-gap-analysis.md](02-gap-analysis.md) (the gaps each stage closes). This report is the plain-English entry point; the others carry the evidence.

## Who this is for

The founder and anyone who joins later and asks "why are we building this, and what will it let us do?" Every bullet below is a concrete thing you will be able to do with polint after the stage completes, and every bullet points at the item in the build plan or gap analysis that delivers it. There are no dates here; stages are ordered by what depends on what.

Terms used more than once, glossed once:

- **Static analysis**: reading code without running it to answer questions about what it does.
- **Rule**: a repo-local policy written as a small typed Rust function that polint runs over the code and reports on.
- **Call graph**: the map of which function can call which other function.
- **Taint analysis**: following untrusted data (a request parameter, a secret) through the program to see whether it reaches somewhere dangerous (a shell command, a log line) without passing through a check.
- **Summary**: a compact description of what a function does to its inputs and outputs, so callers can be analyzed without re-reading the function's body every time.
- **Warm run**: a run after an earlier run has already analyzed most of the code; ideally it does only the work the change requires.
- **Oracle**: an independent source of truth used to score the engine, such as recorded program executions or a hand-checked list of expected findings.
- **False positive**: a finding that is wrong or not worth acting on. **False negative**: a real problem the engine missed.
- **L2, L3, L4, L5, L6**: rungs of the capability ladder in report 01. Roughly: L2 knows what names mean, L3 understands one function's control flow, L4 follows data across functions and files, L5 understands objects and callbacks precisely, L6 reasons about which branch combinations are actually possible.

## The whole journey in one paragraph

Today polint can tell you what a name refers to across files, understand the control flow inside one function, and follow untrusted data across function calls within a bounded search, but it cannot run its deep analysis on a medium-sized TypeScript repository without running out of memory, it does not use TypeScript's own type information, its framework knowledge is hard-coded, and nothing in CI proves its accuracy. The plan first makes every claim measurable and makes deep analysis finish on real repositories (Stage 0), then closes the five specific gaps that separate polint from CodeQL and Semgrep Pro on cross-function taint (Stage 1), then persists summaries so that a warm run is proportional to the change rather than the repository (Stage 2), then proves the engine on a 1.5-million-line repository and gives agents a graph they can query (Stage 3), then buys precision for callback-heavy JavaScript and interprocedural nullness and constants (Stage 4), and finally adds branch-feasibility reasoning and publishes head-to-head numbers against the incumbents (Stage 5). At the end, polint is the only engine where a repository's own policies are written as compiler-checked code, run locally in seconds, follow data across files with declared precision, and come with published, reproducible accuracy numbers.

## Summary table

| Part | What you gain | What it unlocks next |
|---|---|---|
| Stage 0: instrument and unblock | accuracy and scale become visible and gated; deep analysis finishes on a medium repository; rules can tell "clean" from "gave up" | every later stage can prove it moved something |
| The L4 certification set (inside Stages 1 and 2) | trustworthy cross-function, cross-file taint for Go and TS/JS at CodeQL and Semgrep Pro depth, with declared precision | security policies become a product, not a demo |
| Stage 1: resolution and decision | TypeScript types drive call resolution; taint search is no longer depth-limited; request objects are tracked field by field; frameworks are described as data | summaries worth persisting |
| Stage 2: persistence and warm review | memory proportional to what changed, warm review proportional to the pull request, dependencies analyzed once per version | deep rules become usable in daily workflows and CI |
| Stage 3: scale, graph surface, rule-host cost | the largest target repository runs inside a memory budget; agents query the code graph; a first rule check takes seconds, not minutes | adoption without a scale caveat |
| Stage 4: selective precision | callback and middleware dispatch resolved precisely; nullness and constants tracked across calls; agent-written framework models measured | fewer false positives on real JavaScript, richer policies |
| Stage 5: feasibility and publication | findings on impossible branch combinations disappear; review mode reports only witnessed paths; head-to-head results published | a defensible public claim to the top of the ladder |

## Stage 0: instrument and unblock

Headline: you stop guessing. Accuracy and memory are measured every night, deep analysis completes on a real repository, and a rule can say whether it searched everything.

You will be able to:

- Run a one-click benchmark (a manual GitHub Actions trigger, no schedule) that prints call-graph accuracy and speed on the Jelly and Go oracle suites to the run summary and leaves the numbers downloadable as artifacts — and that fails loudly instead of the current silent skip when the oracle repositories are missing (03 §Stage 0, on-demand oracle gate; 04 §8).
- Run the full deep pipeline on excalidraw, an 86,527-line TypeScript repository, and have it finish under 6 GB and 300 seconds instead of being killed at 12 GB (03 §Stage 0, scale root cause; 02 §7 item 7).
- Point at a probe suite that proves polint's L1 to L3 claims for both languages, with "must-not-report" twins that catch over-reporting (03 §Stage 0, probe suite v1; 04 §3.1).
- Write a rule that asks "was this run complete for what I requested, or did a budget cut the search short?" so that "no findings" is never silently mistaken for "clean" (03 §Stage 0, completeness accessor; 02 §7 item 8).
- Score real security findings against SecBench.js exploit cases and forty curated Go and TypeScript cases, instead of the current placeholder adapter (03 §Stage 0, taint corpus v0; 04 §3.2).
- Have branch conditions recorded in the engine's intermediate representation, the prerequisite for every later path-sensitivity feature (03 §Stage 0, bind branch predicates; 02 §3.5).
- Close the semantic-store slice that stalled: TypeScript syntax metadata persisted and the store privately enabled with one measured cold-versus-warm pair (03 §Stage 0, Phase 65 close; 03 §2.2).

## The L4 certification set

This block is called out on its own because it is the highest-value work in the repository. L4 is the rung where taint analysis crosses functions and files correctly, and it is where CodeQL and Semgrep Pro earn their reputation. Five items separate polint from that rung (02 §3.3, §7 items 1 to 5). Items 1 to 4 land in Stage 1; item 5 is the persistence half in Stage 2.

Headline: cross-function, cross-file taint you can trust, with every finding labeled by how it was found.

You will be able to:

- Catch a secret that flows to a logger through three helper functions in three files, with the path shown, because taint search uses **tabulation** (computing each function's summary once and reusing it) instead of a bounded path enumeration that defaults to eight hops (02 §3.3 taint decision procedure; 03 §Stage 1, IFDS tabulation).
- Resolve TypeScript method calls using the TypeScript compiler's own types before falling back to heap reasoning, which the repository's own review named the single largest real-world recall lever (02 §3.3 type-directed tier; 03 §Stage 1, TS type sidecar).
- Track `req.body.name` separately from `req.body.id`, so a policy about one field of a request object does not fire on every field, because summaries carry **access paths** (the field chain a value travels through) instead of whole-parameter facts (02 §3.3 access-path summaries; 03 §Stage 1).
- Describe your own framework, wrapper library, or internal RPC layer as a data file the engine validates, with sources, sinks, sanitizers, propagators and entrypoints, instead of relying on the fifteen hard-coded recognizers (02 §3.3 framework models as data; 03 §Stage 1, models as data v1).
- Read a taint corpus score you can repeat from a fresh clone: at least 90 percent precision and 70 percent recall on 150 to 200 labeled cases with distinct sanitizer names and explicit false-positive traps (03 §Stage 1 exit criteria; 04 §3.1).
- Know that every reported path is realizable: the engine never stitches a path that enters a helper from one caller and exits toward another (02 §5 hygiene; 01 §2 L4).
- Have dependency packages summarized once per version so `node_modules` and the Go module cache are never re-analyzed on warm runs, which is what makes deep taint affordable (02 §3.3 dependency handling; 03 §Stage 2).

## Stage 1: resolution and decision

Headline: the engine resolves calls with types, decides taint with summaries, and learns frameworks from data.

You will be able to:

- Run a raw-admin-API reachability policy on a TypeScript service and have method calls on typed receivers resolve to the right implementation rather than to every function with that name (03 §Stage 1, TS type sidecar; 02 §3.3).
- Raise the search depth on a data-flow policy without the run time exploding, because depth no longer multiplies the work (03 §Stage 1, IFDS tabulation).
- Write a sanitizer list and see it kill taint on the exact path it appears on, not on unrelated paths (02 §3.3 sanitizer semantics; 03 §Stage 1).
- See every taint finding name its tier: resolved by types, by field, or by heap reasoning, with the sanitizer evidence attached (03 §Stage 1 exit criteria).
- Measure what your framework model added: findings with the model versus without it, on a held-out subset, so an agent-written model cannot quietly inflate recall (03 §Stage 1, models as data v1; 02 §4 axis D).
- Persist files, symbols, references, functions, calls and unknown regions as validated rows, the restricted ingest that Stage 2 builds on (03 §Stage 1, Phase 66 ingest).

## Stage 2: persistence and warm review

Headline: warm review finishes in time proportional to your pull request, not the repository, and memory is proportional to what changed.

You will be able to:

- Run `polint review` on a pull request and have the engine recompute only the changed functions and the functions that depend on their summaries, with the recompute set asserted by fixtures (03 §Stage 2, invalidation frontier; 02 §4 axis C).
- Trust that warm output is byte-identical to a cold run, including findings, evidence, unknowns and budget states, because reuse ships only behind that parity gate (03 §Stage 2 exit criteria).
- Analyze hugo and excalidraw end to end with peak memory proportional to the analyzed working set: touching 10 percent of functions uses under 40 percent of cold peak (03 §Stage 2 exit criteria; 05 §5).
- Never re-parse a dependency's bodies once they are summarized, as long as the package, version, schema, toolchain and configuration identity match (03 §Stage 2, dependency package summaries).
- Have a run that hits its memory or time budget degrade with a reported reason per finding instead of dying (03 §Stage 2, runtime envelope; 05 §4).
- Ask internal queries such as callers, callees, used-by, paths and taint-style reachability over complete store generations, with one honest result envelope, before anything is public (03 §Stage 2, internal query engine).

## Stage 3: scale, graph surface, rule-host cost

Headline: the largest target repository runs inside a budget, agents can query the code graph, and the first rule check takes seconds.

You will be able to:

- Run the full pipeline on grafana, 1.55 million lines of Go and TypeScript, on a 16 GB machine inside the envelope, with cost curves published (03 §Stage 3 exit criteria).
- Let an AI agent ask `polint graph` for callers, callees, used-by, neighbors, paths and taint-style reachability and get agent-shaped JSON with precision, unknown counts and recall context rendered by default (03 §Stage 3, `polint graph`; 02 §4 axis E).
- Run a first `polint check` with one rule on a small repository in under 20 seconds instead of a 187-second compile of 225 crates, and see zero Cargo starts when rules are unchanged, because rules compile against a thin SDK and receive facts from the prebuilt engine (03 §Stage 3, thin-SDK build; 05 §5 runner-up).
- Recover from a crash during store writes with either the previous complete generation readable or an explicit rebuild diagnostic, never mixed rows (03 §Stage 3, recovery and scale gates).
- Use parallel summary closure and solver stages that keep byte-identical output, proven by the determinism gate (03 §Stage 3, parallel per-SCC closure).
- Optionally search symbols, evidence text and diagnostics lexically, if Stage 2 leaves capacity; this is the designated cut (03 §Stage 3, lexical search).

## Stage 4: selective precision

Headline: precision where real code needs it, without paying for it everywhere.

You will be able to:

- Resolve Express middleware chains, event-emitter callbacks and other function-valued dispatch in TypeScript precisely, because the top 5 percent of functions by callback fan-in get object sensitivity while the rest stay cheap (03 §Stage 4, selective object sensitivity; 02 §3.4).
- Trade a small, reported amount of recall for a large speedup on huge JavaScript codebases by bounding pointer indirection depth, the principled scale knob, instead of a blunt token cap (03 §Stage 4, indirection-bounded propagation; 02 §3.4).
- Get sharper Go interface and function-value dispatch measured on real Go repositories (03 §Stage 4, VTA-grade narrowing).
- Write a policy that tracks a nil or a configuration constant across function calls, not only inside one function (03 §Stage 4, IDE lifting; 02 §3.2).
- Enforce "acquire then release" and "check before use" policies as proofs over every path and every exit, including error exits, instead of same-function ordering (03 §Stage 4, typestate and dominance-based guards; 02 §3.2).
- Let an agent write a framework model for a library polint has never seen, with the recall and precision delta measured before it is trusted, and build extensions through a public author surface and `polint extension` commands (03 §Stage 4, agent-authored models and extension CLI; 02 §6).
- Read function effects (reads, writes, calls, external effects) through a public SDK view once the gates pass (03 §Stage 4, `Effects` view).

## Stage 5: feasibility and publication

Headline: the engine stops reporting the impossible, review mode reports only witnessed problems, and the numbers go public.

You will be able to:

- Stop seeing findings that require two branch outcomes that cannot both happen, because the engine checks feasibility over nullness, constants and intervals using the branch predicates bound in Stage 0 (03 §Stage 5, path feasibility; 02 §3.5).
- Use a review mode that reports only findings with a feasibility-checked witness path, the right bias for diff-time review where a false alarm costs more than a miss (03 §Stage 5, under-approximate review mode; 02 §3.5).
- Hand a skeptic a published head-to-head against CodeQL, Semgrep and Opengrep on public repositories at pinned commits, with adjudicated disagreements, both oracle lanes, cost columns and every engine's raw output (03 §Stage 5; 04 §5 and §7).
- Run the mutation suite whenever you choose: it injects bugs each rung must catch and applies rewrites that must not change the findings, so a soundness regression is caught before users see it (03 §Stage 5; 04 §6).
- Read a written go-or-no-go on cross-language flow, a TypeScript client calling a Go handler over an HTTP route, the one problem no incumbent solves well (03 §Stage 5, boundary spike).

## What this plan deliberately does not give you

- More languages. Python, Java and external-index frontends are excluded by the founder's constraint; the engine is designed so they can be added later, but none is scheduled (03 §7).
- A query language. Rules stay typed Rust; the compiler is the verifier (02 §8).
- Machine-learning detection. Detection stays symbolic; any ML sits at the edges and proposes, never decides (02 §8; 03 §7).
- A remote summary registry, GPU or distributed solving, or a required daemon (03 §7).

## How to use this document

- Use the summary table when explaining the plan to someone with five minutes.
- Use a stage section when deciding whether that stage's exit criteria are worth its cost; every bullet is a user-visible capability, so the answer is a product answer.
- When a bullet's citation points at report 03, the exit criterion there is the proof that the capability exists; when it points at report 02, the gap table there is the evidence that the capability is missing today.

## Where the numbers in this document come from

- The 86,527-line excalidraw run that was killed at 12 GB is recorded in `research/evaluation-harness/baselines/scale-corpus-run.json`; the 187-second, 225-crate cold rule build is in `research/evaluation-harness/baselines/build-cost.json`; the fifteen hard-coded framework recognizers are counted from `crates/polint/src/analysis_neutral/entrypoints/`.
- The precision and recall thresholds (90 percent, 70 percent, the 40 percent memory target) are exit criteria set in report 03, not measurements; they become measurements only when the corresponding stage's gate runs.
- Every other figure is traced to a report section in the bullet that uses it; if a bullet has no citation, treat it as a summary of the cited bullets around it.
