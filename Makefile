CARGO ?= cargo
PYTHON ?= python3
BUILD_COST_LABEL ?= local
BUILD_COST_RUNS ?= 1

# `make bench-run` knobs. The defaults are exactly what
# .github/workflows/bench-run.yml runs, so a local run and a CI run compare.
BENCH_OUT ?= .context/bench-run
BENCH_RUNS ?= 3
BENCH_TIMEOUT_SECONDS ?= 1200
SCALE ?= 0
GRAFANA ?= 0
DEEP_TARGETS ?= jelly
ONLY ?=
ACCURACY ?= 1
NPM_JELLY ?= 0
BUILD_COST ?= 0

.PHONY: install test lint doc install-smoke deny check readme-assets fetch-scale-repos scale-corpus-run eval-gate bench-run build-cost build-cost-baseline js-bench-fetch js-bench-record-observed js-bench

install:
	$(CARGO) install --locked --path crates/polint --force

test:
	$(CARGO) test --workspace --all-features --locked

lint:
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --workspace --all-targets --all-features --locked -- -D warnings

# `rustdoc` same as CI doc job (workspace, -D warnings).
doc:
	RUSTDOCFLAGS='-D warnings' $(CARGO) doc --workspace --all-features --no-deps --locked

# Mirrors CI: release `cargo install` + `polint --version` (slow; uses --ignored test).
install-smoke:
	$(CARGO) test -p polint --test cargo_install_smoke --locked -- --ignored

# Supply-chain / policy (install: `cargo install cargo-deny`).
deny:
	$(CARGO) deny check all

# Full local gate aligned with `.github/workflows/ci.yml` (all jobs on one machine).
check: lint test doc install-smoke deny

# Materialize the three golden-corpus scale repositories at the commit SHAs
# pinned in research/evaluation-harness/suites/*-scale.toml (never floating).
# Checkouts are gitignored under research/evaluation-harness/repos/.
fetch-scale-repos:
	$(PYTHON) scripts/fetch-scale-repos.py

# Fetch pinned scale repos and publish LOC / peak RSS / wall-clock into
# research/evaluation-harness/baselines/scale-corpus-run.json (opt-in; not CI).
scale-corpus-run:
	$(PYTHON) scripts/run-scale-corpus.py

# External graph-accuracy gate: the Jelly JS/TS callgraph micro suite and the Go
# x/tools RTA callgraph suite, scored against
# research/evaluation-harness/baselines/persisted-graph-accuracy.json. Materializes
# the pinned checkouts first, then runs the same test as
# .github/workflows/eval-gate.yml. Missing checkouts fail instead of skipping.
# Needs the Go toolchain.
# Writing reports also refreshes that baseline file with the measured numbers, so
# read `git diff` on it before keeping the change.
eval-gate:
	$(PYTHON) scripts/fetch-scale-repos.py --suites callgraph
	POLINT_REQUIRE_BENCH_CORPUS=1 POLINT_WRITE_GRAPH_BENCH=1 POLINT_GRAPH_BENCH_TIER=release \
		$(CARGO) test -p polint --lib --all-features --locked \
		eval::external::tests::external_graph_baseline_reports_can_be_generated -- --nocapture
	@echo "accuracy and speed summary: .context/graph-benchmarks/summary.md"
	@echo "per-suite reports: .context/graph-benchmarks/*-baseline.{json,md}"

# Benchmark runner: the same script .github/workflows/bench-run.yml runs, so a
# local number and a CI number come from one code path. Logs the machine, fetches
# the pinned public corpora (public OSS only), measures wall clock / peak RSS /
# cache size / output md5 across warm, cold, and no-cache tiers, scores the two
# call-graph oracles, and writes a readable report plus every raw sample.
#
#   make bench-run                       CI default selection
#   make bench-run SCALE=1               adds excalidraw and hugo
#   make bench-run SCALE=1 GRAFANA=1 DEEP_TARGETS=all
#                                        everything, local only (1.5M LOC, hours)
#   make bench-run ACCURACY=0            speed only, no oracle scoring
#   make bench-run NPM_JELLY=1           install the Jelly npm tree: resolves the
#                                        342-edge helloworld case, costs ~42 min
#                                        for that one case. Off by default, and
#                                        the report calls recall a lower bound.
#   make bench-run BUILD_COST=1          also measures the rule-host build cost
#
# Report: $(BENCH_OUT)/summary.md, also printed to stdout. Needs Go and Node.
bench-run:
	BENCH_OUT=$(BENCH_OUT) \
	BENCH_RUNS=$(BENCH_RUNS) \
	BENCH_TIMEOUT_SECONDS=$(BENCH_TIMEOUT_SECONDS) \
	BENCH_SCALE=$(SCALE) \
	BENCH_GRAFANA=$(GRAFANA) \
	BENCH_DEEP_TARGETS=$(DEEP_TARGETS) \
	BENCH_ONLY=$(ONLY) \
	BENCH_ACCURACY=$(ACCURACY) \
	BENCH_NPM_JELLY=$(NPM_JELLY) \
	BENCH_BUILD_COST=$(BUILD_COST) \
		scripts/bench-runner/bench-run.sh

# Measure what a repo-local rule host costs to build today - Cargo invocations,
# compiled units, wall-clock, rule-host peak RSS, and bytes written/retained -
# and print the ratio against the committed baseline (opt-in; not CI).
build-cost:
	$(CARGO) build --release --locked -p polint -p polint-bench
	$(CARGO) run --release --locked -p polint-bench -- build-cost \
		--label $(BUILD_COST_LABEL) \
		--runs $(BUILD_COST_RUNS) \
		--baseline research/evaluation-harness/baselines/build-cost.json

# Same matrix, rewriting research/evaluation-harness/baselines/build-cost.json.
# Run only when the recorded numbers are meant to move, on an otherwise idle
# machine, and set BUILD_COST_LABEL to name it. BUILD_COST_RUNS raises the runs
# per cell so the recorded value is a median rather than one sample; wall-clock
# needs that, the counts do not.
build-cost-baseline:
	$(CARGO) build --release --locked -p polint -p polint-bench
	$(CARGO) run --release --locked -p polint-bench -- build-cost \
		--label $(BUILD_COST_LABEL) \
		--runs $(BUILD_COST_RUNS) \
		--out research/evaluation-harness/baselines/build-cost.json

# JS/TS analysis benchmark (tests/js-analysis-bench/README.md). Opt-in; not CI.
# js-bench-fetch downloads the pinned package tarballs once and checks their
# hashes. js-bench-record-observed runs third-party code under the call recorder
# inside a throwaway container with no network and no credentials, and rewrites
# the committed observed calls; it is never part of `cargo test`. js-bench reads
# polint's facts, scores them and rewrites tests/js-analysis-bench/baseline/, so
# read `git diff` on it before keeping the change. JS_BENCH_ARGS passes options
# through, for example JS_BENCH_ARGS="--no-cost".
js-bench-fetch:
	$(PYTHON) tests/js-analysis-bench/bench.py fetch

js-bench-record-observed: js-bench-fetch
	$(PYTHON) tests/js-analysis-bench/bench.py record-observed

js-bench: js-bench-fetch
	$(PYTHON) tests/js-analysis-bench/bench.py report $(JS_BENCH_ARGS)

# Regenerate the colored-output SVGs embedded in README.md from the tracked
# ANSI fixtures under docs/img/. Run after changing diagnostic colors/format.
readme-assets:
	$(PYTHON) scripts/render-ansi-to-svg.py \
		--title "polint check --color always" \
		-i docs/img/example-config-denied-literal.ansi \
		-o docs/img/example-config-denied-literal.svg
	$(PYTHON) scripts/render-ansi-to-svg.py \
		--title "polint check --color always" \
		-i docs/img/example-no-raw-colors.ansi \
		-o docs/img/example-no-raw-colors.svg
