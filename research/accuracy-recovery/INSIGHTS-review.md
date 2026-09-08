# Skeptical review of recovery evidence

Agent / angle: independent evidence and implementation-plan review  
Question answered: which conclusions are measured, which are conditional, and what could cause tonight's implementation plan to miss its gate?  
Date: 2026-09-08  
Status: reviewed; no product modifications or new corpus execution

## Sources reviewed

Local primary evidence: preserved `accuracy-restoration-RESEARCH.md` and `FINAL-REPORT.md`; this directory's `INSIGHTS-collector-architecture.md`; `.context/accuracy-research/taxonomy/{inputs,family-summary,cases,edges}.json`; `.context/accuracy-research/caller-counterfactual.json` and its generating script; pinned Jelly source at `b799ed4f0d68c670fe398830aaa51dd5c628cf74`; `crates/polint-eval/src/harness/external/mod.rs`; `crates/polint-eval/src/lib.rs`. Access date: 2026-09-08. Rust test review used the repository Rust skill and its testing chapter. Research scope excludes a new restoration experiment and does not require current online ecosystem claims.

## High-confidence findings

- Independently re-counted union edge rows: HEAD TP/FP/FN = 249/8/1230; measured rejected port = 940/36/539. Every copied report SHA-256 matches `inputs.json`. The port retains all 249 HEAD true positives and recovers 691 additional expected edges.
- Family totals partition 76 cases and 1,479 expected edges exactly. They are **fixture-family counts**, not causal ablations. A class fixture can contain callbacks, arrays, or imports; recovering its FNs does not prove every recovered edge came from a class model. Keep this distinction immediately beside the final taxonomy table.
- Exactly 342 residual expected edges target missing source files, all in `helloworld/app`; these involve **34 distinct target files**. Do not call them 342 missing files. The zero-FP bound of 0.869266 and FP=36 bound of 0.857466 assume these same 342 targets remain unavailable and every other expected edge is recovered. These are conditional environment/source-coverage bounds, not intrinsic limits of any engine architecture. Retain this case and its oracle in the acceptance gate.
- The constructor counterfactual accounts for eight observed FP edges whose target is already correct and whose caller can be paired to an existing FN. If all eight are corrected without collateral effects, TP/FP/FN become 948/28/531, F1 0.772301. This arithmetic is not a test result. At FP=28 the gate needs TP>=975, so it still needs **27 additional TPs**. At FP=36 it needs TP>=980 (+40 from the measured port).
- CommonJS `client4` has seven residual FNs. Three are calls to the `__importDefault` helper itself (two call2fun and one fun2fun). Four lead to imported class construction/method invocation. Do not credit the wrapper-shape repair with all seven without demonstrating helper invocation handling too. The archived focused wrapper fixture is not itself a scored corpus case.
- The gate tests `POLINT_WRITE_GRAPH_BENCH` with `var_os(...).is_some()`. **Unset it; setting it to `0` still enables baseline writes once the test passes.** Leaving it unset keeps reports in a retained temporary directory. Preserve those JSON reports after every run.
- The gate implementation physically lives under `crates/polint-eval/src/harness/`, but it is included into `polint` as `crate::eval` under `cfg(test)`. Keep the command's package as `-p polint`; `polint-eval` is an intentionally empty library target. Require the log to show the actual named test executed, not merely Cargo exit success.

## Medium-confidence findings and residual fixture candidates

These counts are actual residual FNs; mechanisms below are source-informed investigation leads, not promised gains. Distinct rows have distinct edge sets, but one general implementation fix can affect several rows and regress others.

| Case | Residual FN | Source behavior requiring a fixture |
| --- | ---: | --- |
| `micro/arrays3` | 6 | `reduce` returns initial/first callable accumulator with empty/single-element arrays |
| `micro/arrays4` | 4 | A passed callback invokes a callable array element; callback parameter is overwritten after `forEach` |
| `micro/arrays5` | 8 | Array builtin reassignment (`forEach = sort`), callback parameter propagation, later parameter overwrite |
| `micro/default-parameter` | 4 | Nested default invocation (`g(x=f())`) and callable default argument |
| `micro/destructuring` | 3 | Destructuring assignment invokes setter callback; object rest/property flow |
| `approx/srcLoc` | 12 | Parenthesized function creation/calls and computed-member calls |
| `micro/client4` | 7 | Three helper-invocation edges plus four imported constructor/method edges |

The source-location case label does not prove a coordinate bug: both fun2fun and call2fun edges are missing, so extraction/shape flow can also be responsible. `arrays5` deliberately aliases builtins and is a more complex/riskier fix than a plain missing native callback model.

## Recommendation and estimate review

Continue the private port, keeping one shared edge-producing solver. The textual reuse evidence rules out treating a fresh historical port as an independent unexplored source of thousands of missing language-model lines. The archive nevertheless includes an unverified later identifier repair and must be reconstructed and corpus-run before its actual starting score is known.

10–16 engineer-hours / approximately 8–12 elapsed hours for a coordinated two-person crew is a reasonable **conditional planning estimate**, with 2–4 hours contingency. It is not a measured promise to pass: after the strongest eight-edge caller explanation, another 27 TPs remain unproven. Constructor ownership is cross-layer work; parallelize corpus triage/fixtures with integration, not competing edits to the same projection files. Reserve full unchanged debug/release gate runs and required checks inside the estimate. If build assets are absent, resource contention or compilation can add further elapsed time.

Do not treat 0.785193–0.80 as an established attainable ceiling. It is the proposed near-term target range. The available evidence shows a measured 0.765784 state and opportunities exceeding the arithmetic deficit; it does not demonstrate their independent recoverability at stable precision.

## Corrections for the final plan

1. Label the table as FN counts by fixture family, and distinguish a missing-target-source block from regression recovery opportunity.
2. Give the eight caller swaps as a counterfactual sub-bucket of the class family, not an additional set to sum into its FN total. Budget the remaining 27 TPs separately and capture actual per-case edge deltas after each fix.
3. Specify the unchanged named gate in debug and release, full corpus tier, corpus-required mode, explicitly unset baseline-writer variable, pinned corpus identities, normal temporary directory behavior, baseline/tolerance hashes before and after, and retained per-case JSON. A green F1 alone does not excuse the gate's cost-column assertions or unrelated newly introduced failures.

## Open validation gaps

No implementation or full corpus experiment was run by this review. Caller corrections may expose canonical ownership/provenance issues, candidate pools may overlap in implementation cause, and the source-coverage bound assumes unchanged dependencies. The final corpus score of the latest archived code is unknown until tonight's first reconstruction run.
