# Vendored tree-sitter-go grammar

This directory is tree-sitter-go **0.25.0** (crates.io checksum in `UPSTREAM`)
with one grammar change: `special_argument_list` accepts an expression as well
as a type in the first argument so Go 1.26 `new(expr)` parses.

`make(T, …)` and existing `new(T)` programs keep the 0.25.0 tree shape.
Bare identifiers still parse as `type_identifier` (`new(x)`, `new(int)`),
because the first alternative is `prec.dynamic(2, $._type)` — enough to beat
`_type_identifier`'s dynamic `-1` in the existing `[_simple_type, _expression]`
conflict. A dynamic of `1` ties that conflict and expressions win, which would
change trees for every previously valid `new(x)`.

`src/parser.c` and `src/grammar.json` are generated. `src/node-types.json` is
unchanged from 0.25.0 (no new node types).

## Regenerate

Requires tree-sitter CLI **0.25.8** (the version that produced the upstream
`parser.c`, ABI 15):

```bash
cargo install tree-sitter-cli --version 0.25.8 --locked
cd crates/polint/vendor/tree-sitter-go
tree-sitter generate
```

Do not bump the CLI until the tree-sitter crate pin (`=0.26.8`) and this
generated C are moved together.

## Upstream

The same `special_argument_list` widening belongs in
https://github.com/tree-sitter/tree-sitter-go once that repository is
accepting grammar changes again. Until then polint compiles this copy so
crates.io builds include the Go 1.26 form.
