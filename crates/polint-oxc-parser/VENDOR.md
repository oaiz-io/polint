# Forked Oxc parser

This crate is `oxc_parser` **0.129.0** (crates.io checksum and upstream commit
in `UPSTREAM`) with one change: the parser stops at a fixed nesting depth.

Oxc's parser is recursive descent. A source nested a few tens of thousands of
levels deep (parentheses, arrays, blocks, nested functions, JSX elements,
nested types and more) recurses once per level and overflows any thread
stack. A stack overflow aborts the process; it cannot be caught as a panic.
polint parses every file of a repository and of its dependencies, so a single
such file would stop a whole run.

The fork counts nesting in the functions every recursive path of the parser
passes through: each such function runs inside `nested`, which counts one
level for as long as it runs. Chains the parser reads in a loop (`a.b.c`,
`a + b + c`, `f()()`, `T[][]`, `a.b.C` in types and JSX tag names) build a
syntax tree as deep as the chain is long, and everything that later walks the
tree recurses once per link, so each link counts one level too
(`chain_link_too_deep`) until the production that read the chain returns.
When the count reaches `MAX_NESTING_DEPTH`, the parser records a fatal error
(`Nesting is deeper than N parser levels`) and returns an empty program, the
same way it does for any other unrecoverable syntax error. The limit is a
count, not a measurement of the stack, so whether a file parses depends only
on its contents. Rewinding a speculative parse does not clear the error, and
it is the error the parse reports even where another fatal error came first.

Two helpers that walked the syntax tree recursively while parsing (marking a
`/*#__PURE__*/` call at the left end of an operator chain, and comparing JSX
closing tag names) are written as loops.

Everything else under `src/` is byte-identical to the 0.129.0 release.
`rustfmt.toml` is upstream Oxc's, so formatting does not move upstream lines.
The crate's library name stays `oxc_parser`, so code using it imports it as
upstream.

## Upstream

No Oxc release up to 0.159 limits nesting depth. The change belongs upstream;
until it lands there, polint builds this copy and publishes it as
`polint-oxc-parser` so crates.io builds of polint include it.

## Updating

To move to a newer Oxc, copy that release's `src/` over this one, reapply the
changes above (`MAX_NESTING_DEPTH`, `nested`, `chain_link_too_deep` and
`report_nesting_exceeded` in `src/lib.rs`, their callers, the two loops and
the `nesting_too_deep` diagnostic), pin every `oxc_*` dependency in
`Cargo.toml` to the same release and update `UPSTREAM`. Move the root
workspace's `oxc_*` pins in the same change.
