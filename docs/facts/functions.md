# Function Facts

`FunctionFact` describes a function-like declaration found by the Go or TS/JS
adapter. Rules request the `Functions<'_>` typed fact view on a
`#[polint::rule]` function. The same parameter is how polint derives the
`syntax` capability.

## Fields

| Field | Meaning |
|-------|---------|
| `id` | Stable ID for this analysis run. Use it to relate function-owned facts such as branch obligations. |
| `file` | Stable `FileId` for the source file. |
| `name` | Extracted function, method, arrow/function variable, or method name. Go methods are normalized as `Receiver.Method`. |
| `span` | Source span for the function declaration or best available function syntax. |
| `language` | Source language family. |
| `is_test` | `true` for Go top-level test-like functions harvested from `_test.go` files. |
| `is_exported` | Syntax-level exported/public signal where available. |
| `cyclomatic_complexity` | Heuristic syntax-level complexity count. It does not use type or semantic analysis. |
| `calls` | Direct syntactic call names found in the function body. This is not a resolved call graph. |

For reusable size and complexity signals, prefer `FunctionMetrics<'_>` and
`ComplexityMetrics<'_>` from [Metric Facts](metrics.md). `FunctionFact` remains
the lower-level syntax fact.

## Limits

- Calls are names from syntax, not resolved symbols.
- Complexity is syntax-level and intentionally conservative.
- Invalid parse trees may reduce or omit facts for a file.
- TS/JS facts do not use TypeScript type checking.
- A TS/JS function with no static name (an arrow passed as an argument, an
  anonymous `export default` function or class, a method whose key is computed
  at run time such as `[Symbol.iterator]()`) is named by its span,
  `<polint:anonymous:START:END>`.
- TS/JS functions are read from every statement and expression that can hold
  one: blocks, `try`/`catch`/`finally`, labelled and `with` statements,
  parameter and destructuring defaults, and the operands of unary, binary,
  template and type-assertion expressions.

## Small Rule Shape

```rust
use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/high-complexity",
    description = "Warn on functions over the configured complexity threshold.",
    severity = "warn"
)]
fn check_complexity(ctx: &mut RuleCtx<'_>, functions: Functions<'_>) -> RuleResult {
    let max = ctx.options().max.unwrap_or(10);
    let mut diagnostics = Vec::new();
    for function in functions.iter() {
        let file = ctx.file_path(function.file);
        if file_in_scope(ctx.options(), &file) && function.cyclomatic_complexity > max {
            diagnostics.push(
                Diagnostic::warning(
                    ctx.rule_id(),
                    file,
                    function.span.diagnostic_range(),
                    "Function is over the configured complexity threshold.",
                )
                .with_evidence("function", function.name.clone())
                .with_evidence("complexity", function.cyclomatic_complexity.to_string()),
            );
        }
    }
    for diagnostic in diagnostics {
        ctx.report(diagnostic);
    }
    Ok(())
}
```
