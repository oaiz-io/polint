# polint

**Repo-local lint rules for the policies only your team knows.**

polint is an [OAIZ Labs](https://oaiz.io/) open-source project maintained by OAIZ.

polint is a Rust framework for writing static-analysis rules that live in your
repository. It gives those rules fast file discovery, parsers, typed facts,
diagnostics, caching, CI output, and an SDK. You bring the policy.

Use it for conventions that generic linters cannot know: internal API usage,
security guardrails, migration review rules, design-token rules, test-quality
expectations, and the project-specific checks you keep repeating in prompts and
review comments.

polint is not a replacement for ESLint, Biome, Ruff, golangci-lint, or
formatters. It is the layer for rules that belong to your codebase.

## Why polint

Engineering teams are putting more work through AI coding agents, but agents do
not reliably remember local conventions from `AGENTS.md`, prompts, or review
comments. polint turns the parts that are statically checkable into executable
feedback.

Say your frontend must use design tokens instead of raw colors. A rule in your
repository catches the violation and tells the agent how to fix it:

![polint diagnostic for a raw-color literal in Button.tsx](https://raw.githubusercontent.com/oaiz-io/polint/main/docs/img/example-no-raw-colors.svg)

The rule does not just fail the code. It puts the missing project context back in
front of whoever is repairing the change, human or agent.

The rule code stays in your repository, next to the code it protects. That makes
the policy reviewable, testable, versioned, and runnable locally or in CI. polint
ships no built-in policy rules.

## Install

```bash
cargo install polint --locked
```

Or from GitHub Releases:

```bash
curl -sSfL https://raw.githubusercontent.com/oaiz-io/polint/main/scripts/install.sh | bash
```

The default build includes Go and TypeScript/JavaScript analysis. Slim installs
can select one frontend without compiling the other parser family:

```bash
cargo install polint --no-default-features --features lang-go
cargo install polint --no-default-features --features lang-typescript
```

If a repository contains a language whose feature is disabled, polint reports a
`polint/capability` diagnostic instead of running rules against placeholder
facts. See [docs/CONSUMER-SETUP.md](docs/CONSUMER-SETUP.md) for the full matrix.

## Quick start

Run a self-contained example:

```bash
git clone https://github.com/oaiz-io/polint.git
cd polint/examples/config-denied-literal
polint check --color always --fail-on none
```

![polint check on examples/config-denied-literal showing a denied literal diagnostic](https://raw.githubusercontent.com/oaiz-io/polint/main/docs/img/example-config-denied-literal.svg)

In your own repository:

```bash
polint init                        # scaffold .polint.toml and .polint/rules/
polint add-skill                   # install the agent skill
polint new-rule ts no-raw-colors   # add a rule module
polint test                        # run the rule fixtures
polint check                       # run every rule
```

`polint init` creates `.polint.toml`, `.polint/rules/src/`, `.polint/cache/`,
`.polint/output/`, `.polint/.gitignore`, and a root `rust-toolchain.toml` when
missing.

## Write a rule

A rule declares the facts it needs in its signature. polint derives the analysis
capabilities from that signature.

```rust
use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/no-raw-colors",
    description = "Require design tokens instead of raw color literals.",
    severity = "error"
)]
pub(crate) fn no_raw_colors(
    ctx: &mut RuleCtx<'_>,
    literals: StringLiterals<'_>,
) -> RuleResult {
    for literal in literals.iter() {
        if literal.value.starts_with('#') {
            ctx.report(
                Diagnostic::error(
                    ctx.rule_id(),
                    ctx.file_path(literal.file),
                    literal.span.diagnostic_range(),
                    "Use a design token instead of a raw color literal.",
                )
                .with_evidence("literal", literal.value.clone()),
            );
        }
    }
    Ok(())
}
```

Rule packs live in your repository:

```text
.polint.toml
.polint/rules/Cargo.toml
.polint/rules/src/main.rs          # calls polint::runner::run_cli(vec![...])
.polint/rules/src/no_raw_colors.rs
```

`polint new-rule <go|ts|js|generic> <name>` scaffolds a module and wires it into
`main.rs`, with positive and negative fixtures under `.polint/tests/rules/`. Use
`--template <id>` for a policy starter you edit to your own APIs, such as
`secret-to-log`, `ssrf`, or `pii-to-analytics`. These are scaffolds, not bundled
rules.

The facts available to rules are documented in [docs/facts/](docs/facts/).

## Two ways to run

- **`polint check`** runs every rule across the repository. This is where most
  rules live.
- **`polint review <ref>`** runs rules gated to a diff against a branch or
  commit, for policies that should only fire on what changed. See
  [docs/REVIEW-RULES.md](docs/REVIEW-RULES.md).

Profiles are explicit. `polint check --profile web` runs exactly
`[profiles.web]`. Unknown profiles are errors, and there is no default profile.

## Useful commands

```bash
polint check --format ai-friendly --fail-on none   # compact output for agents
polint check --format sarif                        # for CI upload
polint check --stat                                # human scan summary
polint check --jobs 50%                            # cap parallel work; default is 80% of CPUs
polint baseline create                             # adopt in an existing repo
polint ignores --stat                              # find suppressions to fix
polint cache status                                # inspect the local cache
polint inspect rule --format json                  # versioned JSON surfaces
```

`--format ai-friendly` prints counts by rule and at most ten example
diagnostics, then writes the full report to `.polint/output/latest.json`. Query
it with `jq` rather than pasting the whole file into a prompt.

## Documentation

- [Consumer setup](docs/CONSUMER-SETUP.md)
- [Fact reference](docs/facts/)
- [Review rules](docs/REVIEW-RULES.md)
- [Baselines](docs/BASELINES.md)
- [Comment ignores](docs/IGNORE-COMMENTS.md)
- [Cache](docs/CACHE.md)
- [GitHub Action](docs/GITHUB-ACTION.md)
- [Agent playbook](docs/AGENT-PLAYBOOK.md)
- [Releasing](docs/RELEASING.md)

## Versions

polint is pre-1.0. Minor versions can change the rule SDK. Pin an exact version
in `.polint/rules/Cargo.toml` and upgrade deliberately.

The minimum supported Rust version is **1.95**. `polint init` writes a matching
`rust-toolchain.toml` when your repository has none.

## Contributing and support

polint is under active development. Issues and pull requests are welcome. Read
the [contribution guide](CONTRIBUTING.md) before you submit a change. Report
security problems as described in the [security policy](SECURITY.md).

## License

polint is available under the [MIT License](LICENSE).
