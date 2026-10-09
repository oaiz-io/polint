# Go Types

`GoTypes<'_>` is a preview SDK view over what the Go type checker knows about
the scanned Go packages: function signatures and receivers, parameters, struct
fields with their tags, generic instantiations, method sets, and which types
implement which interfaces. Requesting it derives the `go_types` capability.

The answers come from the typed Go frontend, which loads each module with the
Go toolchain. They complement the syntax-level
[`GoTypeDecls<'_>`](go-types.md), which needs no toolchain but knows only what
each file spells out.

## Example

```rust
use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/gorm-constructors-have-tests",
    description = "Constructors that take a *gorm.DB need adapter tests.",
    severity = "warn"
)]
fn gorm_constructors(
    ctx: &mut RuleCtx<'_>,
    functions: Functions<'_>,
    types: GoTypes<'_>,
) -> RuleResult {
    for function in functions.iter().filter(|function| function.name.starts_with("New")) {
        let takes_db = types
            .parameters(function.id)
            .any(|parameter| parameter.type_name == "*gorm.io/gorm.DB");
        if takes_db {
            ctx.report(Diagnostic::warning(
                ctx.rule_id(),
                ctx.file_path(function.file),
                function.span.diagnostic_range(),
                format!("{} takes a *gorm.DB", function.name),
            ));
        }
    }
    Ok(())
}
```

## Type names

Types are named by the type checker's canonical strings:

- a named type by its package path and name: `example.com/app/store.Repo`;
- a pointer with a leading `*`: `*gorm.io/gorm.DB`;
- an instantiated generic with its type arguments:
  `example.com/app/box.Box[string]`;
- composite and predeclared types as Go spells them: `[]string`,
  `map[string]int`, `func(int) error`, `error`.

Aliases are resolved to the type they name.

## Query methods

| Method | Meaning |
|--------|---------|
| `function(id)` | The typed view of a function from `Functions<'_>`: `qualified` name (`example.com/app/store.NewRepo`, `(*example.com/app/store.Repo).Save`), `signature`, and `receiver`. |
| `receiver(id)` | The receiver type of a method; `None` for a function. |
| `parameters(id)` | The parameters of a function or method in order: `index`, `name` (empty when unnamed), `type_name`, `variadic`. |
| `fields_of(owner)` | The fields of struct type `owner` in declaration order: `name`, `index`, `type_name`, `embedded`, the raw `tag`, and the declaration's `file` and `span`. |
| `fields()` | Every struct field of the analyzed packages. |
| `instantiations_of(generic)` | The instantiations of generic type or function `generic` written in the scanned sources: `target` (type or function), `type_arguments`, the `instantiated` type, `file` and `span`. |
| `instantiations()` | Every generic instantiation written in the scanned sources. |
| `method_set(type_name)` | The method names callable on a `*T` of a type declared in the analyzed packages (its value and pointer methods), sorted. |
| `implements(type_name)` | The interfaces a concrete type implements, each with `via_pointer` when only `*T` does. |
| `implementers(interface)` | The concrete types that implement an interface. |

Every method returns its results in a deterministic order.

## Coverage and limits

- Declarations (fields, method sets, parameters) cover the packages the
  frontend loaded from the scanned sources. Dependencies are loaded from
  export data and contribute types to name, not declarations to list.
- Implementation pairs cover every non-generic concrete type the analyzed
  packages declare, checked against every interface with methods that those
  packages declare or name (so `error`, `io.Writer` and `http.Handler` are
  included when the code uses them). Generic types are not paired.
- Parameters are listed for functions and methods declared in the scanned
  sources; function literals are not.
- `_test.go` files are type-checked only when `[languages.go] include_tests` is
  `true`.

## Setup

The capability needs the Go toolchain and a `go.mod` module root for the
scanned Go files (the nearest `go.mod`, or the `[languages.go]` `module_roots`;
see [the consumer setup guide](../CONSUMER-SETUP.md)). When the frontend
loaded no package for them — for example, Go files outside every module root —
`go_types` is unavailable: a rule that requests `GoTypes<'_>` is not run and
`polint check` reports a `polint/capability` diagnostic with status
`setup_missing`.

A rule that can work without types requests `Option<GoTypes<'_>>` instead. It
runs either way, with `None` when the capability is unavailable.
