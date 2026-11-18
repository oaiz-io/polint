# Embedded JavaScript platform models

The private JavaScript analysis loads `crates/polint/models/platform/javascript.toml`
with `include_str!`. The complete file participates in the semantic-graph parameter
digest. Changing a selector, callback argument, effect, or return invalidates cached
graphs. This is engine data; it adds no configuration or SDK contract.

The schema has `schema_version = 1` and `[[methods]]` entries. A method lists
`names` selectors (`global:JSON.stringify`, `member:map`, `new:Promise`), optional
`allocate`, `calls`, `stores`, `returns`, and an optional `operation`.

```toml
[[methods]]
names = ["member:filter"]
allocate = "array"
calls = [{ param = 0, args = ["$this.$elements", "$prim", "$this"], this = "$param1" }]
stores = [{ base = "$result", field = "$elements", value = "$this.$elements" }]
returns = ["$result"]
```

Values name `$this`, `$paramN`, a preceding `$callN` result, `$result`, `$prim`, or
`$open`; dotted paths read validated fields. Internal `$elements`, `$keys`,
`$fulfilled`, and `$rejected` slots describe collection and promise values. The
loader rejects unsupported versions, duplicate or malformed selectors, unknown
fields, invalid paths and forward references to callback results.

`stores` makes returned heap shapes explicit. The closed `operation` vocabulary
handles relations that a list of returned values cannot express: forwarding an
argument list, binding, cardinality-sensitive reduction, copying object fields,
applying descriptors, event-name storage, and promise executors. These primitives
operate on values and shapes; builtin names are selected only by the data file.
This extends the simple callback/return sketch because heap updates and variable
argument forwarding need destinations and argument-list operations.

Callback candidates become ordinary argument/return constraints on private call
sites and validated `ModelEdge` constraints on source call sites. The refined
join reports model edges at Conservative precision. A model edge never asserts
that its source site is complete. Global matching respects lexical shadowing;
known own methods take precedence, and member matching retains ordinary method calls.
The direct resolver continues to abstain from resolving builtin receivers to
same-named user functions.

The model is an over-approximation. Member selectors do not prove receiver brands;
results remain open when a behavior may be overridden. Bind forwarding carries
possible targets and bound arguments conservatively, rather than claiming a total
bound-function implementation. Collection keys and elements are merged; order,
callback predicates, promise settlement, and event removal are not simulated.
Object field/descriptor operations use syntactically available shapes; unavailable
shapes escape. Executor settlement extraction follows local binding copies and
leaves fulfilled and rejected slots open. Getter return flow is conservative and
does not claim the getter runs during descriptor installation. Array spread and
generator iteration retain aggregate element flow; unknown lengths remain open.

Async function results allocate a promise-shaped value; `await` loads its fulfilled
slot and also permits a non-promise operand. Generator yields write iterator element
slots. Reduce skips its callback for empty literal arrays and for a one-element
literal without an initial value; unknown cardinalities retain all possible
accumulator sources.
