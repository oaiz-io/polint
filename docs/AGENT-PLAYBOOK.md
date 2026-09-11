# Agent playbook (polint)

Use this when wiring coding agents or CI to polint.

## Setup

1. Install polint (`cargo install polint --locked` or build from source).
2. Run `polint init` once per repo; add rules with `polint new-rule …`.
3. Optional: `polint add-skill` so the agent finds local usage docs.

## Machine-readable output

- **AI-friendly report**: `polint check --format ai-friendly --fail-on none`
- **AI-friendly schema**: [`docs/schemas/polint-ai-friendly-v1.json`](schemas/polint-ai-friendly-v1.json)
- **JSON report**: `polint check --format json`
- **Schema**: [`docs/schemas/polint-report-v1.json`](schemas/polint-report-v1.json)
- **Stable ordering**: diagnostics are sorted by file, range, rule id, message, fingerprint; use `--max-diagnostics N` to truncate emitted reports **after** that sort. The cap does not change `--fail-on`.

Use `--format ai-friendly` by default when you are an AI coding agent. It prints
counts by triggered rule, shows at most 10 example diagnostics, and saves the
full report to `.polint/output/latest.json`. Do not read that whole file into
context. Query it selectively:

```bash
jq '.summary.by_rule' .polint/output/latest.json
jq '[.diagnostics[] | select(.rule_id=="local/no-raw-colors")][0:20]' .polint/output/latest.json
jq '.diagnostics[] | select(.file=="src/Button.tsx") | {rule_id, range, message}' .polint/output/latest.json | head -c 12000
```

Validate against expectations by deserializing with your agent's JSON stack; do
not rely on stderr prose for pass/fail when JSON is available.

## Focused runs

- **One rule pattern**: `--only-rule PATTERN` (same matching as profiles: exact id, `prefix/*`, or `*`).
- **Cap noise**: `--max-diagnostics N`
- **Severity gate**: `--fail-on warn|error|none`
- **Scan summary**: `--shortstat` for one line, `--stat` for grouped human stats.

Example:

```bash
polint check --format ai-friendly --fail-on none --only-rule 'local/*' path/to/dir
```

`--stat` and `--shortstat` are human-output helpers. They do not append prose to
JSON or SARIF output.

## Baseline ratchets

Use a compact YAML baseline when a repo has existing findings but CI should fail
only on newly introduced policy violations. The baseline always lives at
`.polint/baseline.yaml`:

```bash
polint baseline create
polint check --baseline --new-only
polint baseline update
```

The file uses one string per entry:

```yaml
version: 1

baseline:
  - "local/backend-context-propagation e337fbb73d44b2b7 backend/app/handler.go"
ignore:
  - "local/no-raw-colors 1b7c9a00e493aa21 frontend/Button.tsx"
```

`baseline` means existing debt; it does not fail CI. `ignore` means accepted
central suppression; it is hidden from output and failure. Baseline matching uses
`rule_id + fingerprint` and refreshes unambiguous moved paths; ignore matching
is file-specific so unrelated findings with the same fingerprint stay visible.

## Ignore cleanup

polint supports comment ignores such as
`// polint-ignore-next-line local/no-raw-colors -- legacy fixture`. Ignores
suppress policy diagnostics only; parser, internal, capability, and `polint/*`
diagnostics stay visible.

Use `polint ignores` to find suppressions that an agent should fix:

```bash
polint ignores --shortstat
polint ignores --stat --filter 'local/no-raw-colors,local/*'
polint ignores --format json --filter local/no-raw-colors
```

The JSON shape is documented in
[`docs/schemas/polint-ignores-v1.json`](schemas/polint-ignores-v1.json). See
[`docs/IGNORE-COMMENTS.md`](IGNORE-COMMENTS.md) for syntax and health
diagnostics (`polint/unused-ignore`, `polint/malformed-ignore`,
`polint/ignore-missing-reason`).

## CI snippet

```yaml
- uses: actions/checkout@v6
- uses: dtolnay/rust-toolchain@stable
- run: cargo install polint --locked
- run: polint check --baseline --new-only --format json --fail-on error
```

SARIF for GitHub Code Scanning: `polint check --format sarif` then
`github/codeql-action/upload-sarif` (see root `.github/workflows/ci.yml`).

## Reading `summary`: an empty report is not a proof

`--format json` and `--format ai-friendly` both carry a `summary` object that
says what the run actually did. Read it before concluding that zero diagnostics
means zero problems.

`summary.rules[]` — one row per registered rule:

| Field | Says |
|---|---|
| `outcome` | `analyzed`, `capability_blocked`, or `not_planned`. |
| `observed_events` | Operations the rule's policy queries examined. |
| `blocking_providers` | Providers whose failure blocked the rule. |
| `files_in_scope` | Analyzed files the rule's `files` filter kept. |

`outcome = analyzed` with `observed_events = 0` means the rule ran and matched
nothing. `outcome = capability_blocked` means it never ran, and
`blocking_providers` names why. Those are different states and only the first one
is evidence of a clean repository.

`summary.providers[]` — one row per analysis provider, with `status`,
`elapsed_ms`, cache counters, and provider-specific `counts` (the Go semantic
sidecar reports its per-stage timings and workload there). This is where a slow
or failed run is attributed.

`summary.budgets[]` — one row per budget the run exhausted, so "polint bounded
itself" is distinguishable from "polint found nothing".

## Prompt starter (copy-paste)

> Use the polint JSON report (`polint check --format json`). The schema is in
> `docs/schemas/polint-report-v1.json`. If you are an AI agent, start with
> `polint check --format ai-friendly --fail-on none` and query
> `.polint/output/latest.json` instead of reading the whole file. Parse
> `diagnostics[]`; each item has `rule_id`, `severity`, `file`, `range`,
> `message`, optional `fix`. Before trusting an empty report, read `summary`:
> a rule row with `outcome = capability_blocked` never ran, and one with
> `observed_events = 0` matched nothing. Apply fixes and re-run until the report
> is empty or only allowed severities remain. To ratchet adoption, run
> `polint check --baseline --new-only`. To remove suppressed debt, run
> `polint ignores --stat --filter RULE_ID`, fix the underlying code, remove the
> ignore comment, and rerun `polint check`.

## Troubleshooting

See [CONSUMER-SETUP.md](CONSUMER-SETUP.md) for rules-host errors, env vars, and SARIF help URIs.

## Cookbook patterns

Small, composable rule ideas (implement in `.polint/rules`):

1. **Require `t.Run`** for table-style tests — request `GoTests<'_>` and check `subtest_count` / `subtest_names`.
2. **Substring in test name** — request `GoTests<'_>` and check `test.name.contains("Integration")`.
3. **Forbid import path** — request `Imports<'_>` and scan import paths for a prefix.

Golden JSON: use `examples/ts-design-tokens` or similar; integration tests in
`crates/polint/tests/cli.rs` assert filters and report shape.
