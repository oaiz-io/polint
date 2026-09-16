package semantic

import (
	"os"
	"path/filepath"
	"testing"
)

func TestValidatePackagePatternsRejectsFlags(t *testing.T) {
	_, err := validatePackagePatterns([]string{"-json"})
	if err == nil || err.Error() != "package pattern \"-json\" must not start with -" {
		t.Fatalf("expected flag-like pattern rejection, got %v", err)
	}
}

func TestEmitLoadsPackagesAndBuildsSSA(t *testing.T) {
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.test/fixture\n\ngo 1.24\n",
		"main.go": `package main

func helper() {}

func main() {
	helper()
}
`,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, IncludeTests: true})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}
	assertKind(t, rows, "package")
	assertKind(t, rows, "function")
	assertKind(t, rows, "callsite")
}

func TestEmitNDJSONRows(t *testing.T) {
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.test/fixture\n\ngo 1.24\n",
		"main.go": `package main

type T struct{}

func (T) M() {}
func main() {
	var t T
	t.M()
}
`,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, IncludeTests: true})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}
	if rows[0]["kind"] != "session_begin" {
		t.Fatalf("first row kind = %v", rows[0]["kind"])
	}
	if rows[len(rows)-1]["kind"] != "session_end" {
		t.Fatalf("last row kind = %v", rows[len(rows)-1]["kind"])
	}
	assertKind(t, rows, "package")
	assertKind(t, rows, "function")
	assertKind(t, rows, "callsite")
	assertKind(t, rows, "method_set")
}

func TestEmitCoversMethodsReceiversInitAndUnsupported(t *testing.T) {
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.test/fixture\n\ngo 1.24\n",
		"main.go": `package main

type I interface { M() }
type T struct{}

func init() {}
func (T) M() {}
func call(i I) { i.M() }
func main() {
	var t T
	call(t)
}
`,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, IncludeTests: true})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}
	assertKind(t, rows, "method")
	assertKind(t, rows, "receiver_type")
	assertKind(t, rows, "init_function")
	assertKind(t, rows, "method_set")
	assertKind(t, rows, "callsite")
	assertCallsiteStatus(t, rows, "unresolved_dynamic")
}

func TestEmitHarvestsRTASignals(t *testing.T) {
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.test/fixture\n\ngo 1.24\n",
		"main.go": `package main

type I interface { M() }
type T struct{}

func (T) M() {}

func apply(f func()) { f() }

func call(i I) { i.M() }

func main() {
	var t T
	call(t)
	apply(func() { t.M() })
	apply(t.M)
}
`,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, IncludeTests: false})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}
	assertKind(t, rows, "instantiated_type")
	assertKind(t, rows, "address_taken")
	assertKind(t, rows, "dynamic_dispatch")
	assertSchemaVersion(t, rows, "polint-go-semantic-3")
	assertDynamicDispatchJoinsCallsite(t, rows)
}

func TestEmitDirectRTAEdgesForFunctionValues(t *testing.T) {
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.com\n\ngo 1.24\n",
		"main.go": `package main

func A1() {
	A2(0)
}

func A2(int) {}

var (
	C = func(int) {}
	D = func(int) {}
)

func main() {
	A1()
	pfn := C
	pfn(0)
}
`,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, IncludeTests: false, EmitRTAEdges: true})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}

	assertRTAEdge(t, rows, "main", "dynamic function call", "init$1")
	assertRTAEdge(t, rows, "main", "dynamic function call", "init$2")
}

func TestEmitHarvestsRTASignalsInsideClosureBodies(t *testing.T) {
	// Regression for WR-01: the concrete type Dog is converted to an interface
	// (*ssa.MakeInterface) ONLY inside the closure passed to run(...). Its method
	// Notify is invoked dynamically ONLY inside that same closure. The harvest must
	// walk fn.AnonFuncs (closures live in parent.AnonFuncs, not pkg.Members), or these
	// signals are invisible and interface dispatch misses a real target.
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.test/fixture\n\ngo 1.24\n",
		"main.go": `package main

type Notifier interface { Notify() }
type Dog struct{}

func (Dog) Notify() {}

func run(f func()) { f() }

func main() {
	run(func() {
		var n Notifier = Dog{}
		n.Notify()
	})
}
`,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, IncludeTests: false})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}

	// The instantiated type Dog (converted to Notifier inside the closure) is harvested.
	assertInstantiatedType(t, rows, "example.test/fixture.Dog")
	// The dynamic interface invoke of Notify inside the closure is harvested, joined to
	// its callsite (which lives in the anonymous-function body).
	assertDynamicDispatchMethod(t, rows, "Notify")
	assertDynamicDispatchJoinsCallsite(t, rows)
}

func TestEmitAddressTakenExcludesStaticCallees(t *testing.T) {
	// FINDING 2: a statically-called function is NOT address-taken. For a *ssa.Call /
	// Go / Defer the callee is the first operand, so the address-taken operand loop must
	// EXCLUDE the static-callee operand — otherwise `defer cleanup()`, `go worker()`, and
	// a plain `helper()` call would each spuriously mark their callee address-taken and
	// flood the func-value RTA candidate set. Genuine value uses (a function assigned to a
	// variable / stored in a slice) MUST still be captured.
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.test/fixture\n\ngo 1.24\n",
		"main.go": `package main

func cleanup() {}
func worker()  {}
func helper()  {}
func genuine()  {}

func main() {
	defer cleanup()
	go worker()
	helper()
	// genuine is used as a VALUE (assigned, then stored in a slice) — a real
	// address-taken use that must still be harvested.
	f := genuine
	fns := []func(){f}
	_ = fns
}
`,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, IncludeTests: false})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}

	// The genuine value-use IS captured.
	assertAddressTaken(t, rows, "example.test/fixture.genuine")
	// The statically-called functions are NOT address-taken (call/go/defer of a func).
	assertNotAddressTaken(t, rows, "example.test/fixture.cleanup")
	assertNotAddressTaken(t, rows, "example.test/fixture.worker")
	assertNotAddressTaken(t, rows, "example.test/fixture.helper")
}

func TestEmitMethodSetKeyedByInstantiatedGenericType(t *testing.T) {
	// FINDING A: a method on a GENERIC type satisfying an interface, dispatched via the
	// interface, must resolve to the INSTANTIATED type's method. x/tools records the
	// instantiated named type Box[int] in the SSA runtime-type set with a real method
	// value (Box[int]).Speak, while the syntactic declaration is Box[T any]. emitMethodSets
	// keyed the set by the GENERIC name (Box[T any]) only, so the Rust resolver's
	// method_sets.get("Box[int]") missed and the interface invoke lost its edge. The fix
	// emits the method-set AND the concrete method keyed by the INSTANTIATED identity.
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.test/fixture\n\ngo 1.24\n",
		"main.go": `package main

import "fmt"

type Speaker interface{ Speak() string }

type Box[T any] struct{ v T }

func (b Box[T]) Speak() string { return fmt.Sprint(b.v) }

func main() {
	var s Speaker = Box[int]{v: 1}
	fmt.Println(s.Speak())
}
`,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, IncludeTests: false})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}

	// The instantiated type Box[int] is in the rapid-type set.
	assertInstantiatedType(t, rows, "example.test/fixture.Box[int]")
	// A method-set is emitted keyed by the INSTANTIATED identity Box[int] (not only the
	// generic Box[T any]), and it carries the satisfying method Speak.
	assertMethodSetContains(t, rows, "example.test/fixture.Box[int]", "Speak")
	// The concrete instantiated method is harvested with the bare method name and the
	// instantiated receiver (the pointer method set is used, so the receiver is
	// `*...Box[int]`; the Rust `normalize_type` strips the leading `*` so the join indexes
	// methods_by_receiver["...Box[int]"] with "Speak" and a target node).
	assertMethodWithReceiver(t, rows, "Box.Speak", "*example.test/fixture.Box[int]")
}

func TestEmitDeduplicatesMethodSetForSamePackageGenericAlias(t *testing.T) {
	// FIX-07 (review #4): a SAME-PACKAGE type alias to a GENERIC INSTANTIATION
	// (`type IntBox = Box[int]`) is harvested by TWO independent method_set emitters that
	// produce the IDENTICAL canonical stable_key when `Box[int]` is reachable both ways:
	//   - emitInstantiatedMethodSets sees the reachable RuntimeTypes() instantiation Box[int]
	//     (from the DIRECT `Box[int]{...}` conversion below) and emits a method_set keyed
	//     `stableKey(pkg,"method_set","...Box[int]")`.
	//   - emitMethodSets walks package scope, finds the alias TypeName IntBox, and (since
	//     FIX-05) canonicalizes it through types.Unalias to the SAME underlying `...Box[int]`,
	//     emitting a method_set with the IDENTICAL stable_key.
	// They use SEPARATE per-function `seen` maps, so BOTH rows survive — a duplicate
	// non-empty stable_key. Downstream `validate_unique("method_set", ...)` then rejects the
	// whole Go fact set (zero functions/callsites/edges repo-wide). The emit must produce AT
	// MOST ONE method_set row per canonical stable_key across BOTH emitters.
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.test/fixture\n\ngo 1.24\n",
		"main.go": `package main

import "fmt"

type Speaker interface{ Speak() string }

type Box[T any] struct{ v T }

func (b Box[T]) Speak() string { return fmt.Sprint(b.v) }

// IntBox is a SAME-PACKAGE type ALIAS for the generic INSTANTIATION Box[int].
type IntBox = Box[int]

// useDirect makes Box[int] reachable via its NON-alias spelling too, so the SSA
// RuntimeTypes() set records the instantiation and emitInstantiatedMethodSets fires on
// Box[int] — colliding with emitMethodSets' alias-canonicalized Box[int] row.
func useDirect() Speaker { return Box[int]{v: 2} }

func main() {
	var s Speaker = IntBox{v: 1}
	fmt.Println(s.Speak())
	fmt.Println(useDirect().Speak())
}
`,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, IncludeTests: false})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}

	// No two method_set rows may share a stable_key (the alias↔instantiation collision must
	// be coordinated to a single keep-first row).
	assertNoDuplicateMethodSetStableKey(t, rows)
	// Exactly one method_set row is keyed by the canonical instantiated identity Box[int].
	assertSingleMethodSetKeyedBy(t, rows, "example.test/fixture.Box[int]")
	// The surviving row still carries the satisfying method.
	assertMethodSetContains(t, rows, "example.test/fixture.Box[int]", "Speak")
	// The concrete instantiated method VALUE is STILL emitted (gating the duplicate method_set
	// row must not suppress the method row emitInstantiatedMethodSets also produces).
	assertMethodWithReceiver(t, rows, "Box.Speak", "*example.test/fixture.Box[int]")
	// The SECOND collision the alias+direct scenario surfaces: the instantiated method VALUE
	// `(*Box[int]).Speak` is harvested via BOTH the ssaFunctions walk AND
	// emitInstantiatedMethodSets, so its function/method row must be emitted exactly ONCE — a
	// duplicate function stable_key fails validate_unique (a STRUCTURAL family) and zeroes the
	// whole Go fact set just as a duplicate method_set would.
	assertNoDuplicateFunctionStableKey(t, rows)
}

func TestEmitCanonicalizesTypeAliasToUnderlyingForDispatch(t *testing.T) {
	// FIX 3: a value of a type ALIAS converted to an interface must dispatch to the
	// underlying type's method. For `type AliasDog = Dog; var s Speaker = AliasDog{}`,
	// go/types reports the MakeInterface operand and the alias's package-scope TypeName
	// keyed by the alias spelling (`...AliasDog`), but the concrete method's receiver is
	// the UNDERLYING `*...Dog`. Without canonicalization the Rust resolver looks up
	// methods_by_receiver["...AliasDog"] (None) and the edge is silently dropped. The fix
	// resolves both the instantiated_type AND the method_set key through `types.Unalias`,
	// so instantiated_type, method_set key, and the method receiver all share the
	// underlying `...Dog` identity and the join succeeds.
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.test/fixture\n\ngo 1.24\n",
		"main.go": `package main

import "fmt"

type Speaker interface{ Speak() string }

type Dog struct{}

func (Dog) Speak() string { return "woof" }

// AliasDog is a type ALIAS for Dog (not a distinct defined type).
type AliasDog = Dog

func main() {
	var s Speaker = AliasDog{}
	fmt.Println(s.Speak())
}
`,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, IncludeTests: false})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}

	// The instantiated type is harvested under the UNDERLYING identity, matching the
	// method receiver — never the alias spelling.
	assertInstantiatedType(t, rows, "example.test/fixture.Dog")
	assertNoInstantiatedType(t, rows, "example.test/fixture.AliasDog")
	// A method-set keyed by the UNDERLYING identity carries the satisfying method, so the
	// instantiated-type ⋈ method-set ⋈ receiver join resolves the dispatch.
	assertMethodSetContains(t, rows, "example.test/fixture.Dog", "Speak")
	assertNoMethodSetKeyedBy(t, rows, "example.test/fixture.AliasDog")
	// The concrete method is indexed by the underlying receiver (so the resolved edge has
	// a target node).
	assertMethodWithReceiver(t, rows, "Dog.Speak", "*example.test/fixture.Dog")
}

func TestEmitDynamicDispatchCarriesInterfaceDiscriminant(t *testing.T) {
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.test/fixture\n\ngo 1.24\n",
		"main.go": `package main

type I interface { M() }
type T struct{}

func (T) M() {}
func call(i I) { i.M() }

func main() {
	var t T
	call(t)
}
`,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, IncludeTests: false})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}
	found := false
	for _, row := range rows {
		if row["kind"] != "dynamic_dispatch" {
			continue
		}
		if row["method"] == "M" && row["interface_type"] != "" && row["interface_type"] != nil {
			found = true
		}
	}
	if !found {
		t.Fatalf("expected an interface-invoke dynamic_dispatch row with interface_type+method in %#v", rows)
	}
}

func TestEmitBuiltinCallsEmitNoDynamicDispatch(t *testing.T) {
	// FINDING 4: a builtin call (`append`, `len`, `recover`, ...) has a nil
	// StaticCallee() because its callee is a *ssa.Builtin, not an *ssa.Function. It must
	// NOT be classified unresolved_dynamic and must NOT emit a func-value dynamic_dispatch
	// row — a builtin is not a func value, and a fabricated func_value dispatch on it would
	// feed the RTA driver a bogus unresolved obligation. The package below has ONLY builtin
	// calls (no interface invoke, no func-value call), so NO dynamic_dispatch row may exist.
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.test/fixture\n\ngo 1.24\n",
		"main.go": `package main

func grow(s []int) []int {
	return append(s, len(s))
}

func guard() {
	defer func() {
		_ = recover()
	}()
}

func main() {
	_ = grow(nil)
	guard()
}
`,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, IncludeTests: false})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}
	// The builtin calls produce callsite rows but NO dynamic_dispatch rows.
	assertKind(t, rows, "callsite")
	assertNoKind(t, rows, "dynamic_dispatch")
	// No callsite is classified unresolved_dynamic (the builtins are not func values).
	for _, row := range rows {
		if row["kind"] == "callsite" && row["status"] == "unresolved_dynamic" {
			t.Fatalf("a builtin call must NOT be unresolved_dynamic: %#v", row)
		}
	}
}

func TestEmitHonorsCheckedInGoWorkForMultipleModuleRoots(t *testing.T) {
	root := writeGoWorkspaceFixture(t, true)
	rows, err := Emit(Config{
		Root:         root,
		ModuleRoots:  []string{"services/app", "libs/money"},
		Patterns:     []string{"./..."},
		IncludeTests: false,
	})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}

	assertNoKind(t, rows, "package_error")
	assertPackagePath(t, rows, "example.test/app")
	assertPackagePath(t, rows, "example.test/money")
}

func TestEmitCreatesSyntheticGoWorkWhenRootGoWorkIsMissing(t *testing.T) {
	root := writeGoWorkspaceFixture(t, false)
	rows, err := Emit(Config{
		Root:         root,
		ModuleRoots:  []string{"services/app", "libs/money"},
		Patterns:     []string{"./..."},
		IncludeTests: false,
	})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}

	assertNoKind(t, rows, "package_error")
	assertPackagePath(t, rows, "example.test/app")
	assertPackagePath(t, rows, "example.test/money")
}

func TestEmitSpansUseAstRangesForFunctionsAndCallsites(t *testing.T) {
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.test/fixture\n\ngo 1.24\n",
		"main.go": `package main

func helper() {}

func main() {
	helper()
}
`,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, IncludeTests: false})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}

	assertRowSpanIsRange(t, rows, "function")
	assertRowSpanIsRange(t, rows, "callsite")
}

func TestInitFunctionNameDoesNotMatchInitialize(t *testing.T) {
	if isInitFunctionName("initialize") {
		t.Fatalf("initialize should stay a normal function")
	}
	if !isInitFunctionName("init") || !isInitFunctionName("init#1") || !isInitFunctionName("init$1") {
		t.Fatalf("expected SSA init names to be recognized")
	}
}

func TestSetEnvReplacesExistingValue(t *testing.T) {
	env := setEnv([]string{"PATH=/bin", "GOWORK=off", "HOME=/tmp"}, "GOWORK", "/tmp/go.work")

	var values []string
	for _, entry := range env {
		if len(entry) >= len("GOWORK=") && entry[:len("GOWORK=")] == "GOWORK=" {
			values = append(values, entry)
		}
	}
	if len(values) != 1 || values[0] != "GOWORK=/tmp/go.work" {
		t.Fatalf("expected replacement GOWORK, got %#v from env %#v", values, env)
	}
}

func assertKind(t *testing.T, rows []Row, kind string) {
	t.Helper()
	for _, row := range rows {
		if row["kind"] == kind {
			return
		}
	}
	t.Fatalf("missing row kind %q in %#v", kind, rows)
}

func assertNoKind(t *testing.T, rows []Row, kind string) {
	t.Helper()
	for _, row := range rows {
		if row["kind"] == kind {
			t.Fatalf("unexpected row kind %q in %#v", kind, rows)
		}
	}
}

func assertPackagePath(t *testing.T, rows []Row, packagePath string) {
	t.Helper()
	for _, row := range rows {
		if row["kind"] == "package" && row["package_path"] == packagePath {
			return
		}
	}
	t.Fatalf("missing package path %q in %#v", packagePath, rows)
}

func assertRowSpanIsRange(t *testing.T, rows []Row, kind string) {
	t.Helper()
	for _, row := range rows {
		if row["kind"] != kind {
			continue
		}
		span, ok := row["span"].(*Span)
		if !ok {
			continue
		}
		if span.EndByte <= span.StartByte {
			t.Fatalf("%s span should be a range, got %#v", kind, span)
		}
		return
	}
	t.Fatalf("missing %s row with span in %#v", kind, rows)
}

func assertSchemaVersion(t *testing.T, rows []Row, schema string) {
	t.Helper()
	for _, row := range rows {
		if row["schema"] != schema {
			t.Fatalf("row %#v has schema %v, expected %q", row, row["schema"], schema)
		}
	}
}

func assertDynamicDispatchJoinsCallsite(t *testing.T, rows []Row) {
	t.Helper()
	callsiteKeys := make(map[string]bool)
	for _, row := range rows {
		if row["kind"] == "callsite" {
			if key, ok := row["stable_key"].(string); ok {
				callsiteKeys[key] = true
			}
		}
	}
	for _, row := range rows {
		if row["kind"] != "dynamic_dispatch" {
			continue
		}
		key, ok := row["callsite_stable_key"].(string)
		if !ok || key == "" {
			t.Fatalf("dynamic_dispatch row missing callsite_stable_key: %#v", row)
		}
		if !callsiteKeys[key] {
			t.Fatalf("dynamic_dispatch callsite_stable_key %q has no matching callsite row", key)
		}
	}
}

func assertCallsiteStatus(t *testing.T, rows []Row, status string) {
	t.Helper()
	for _, row := range rows {
		if row["kind"] == "callsite" && row["status"] == status {
			return
		}
	}
	t.Fatalf("missing callsite status %q in %#v", status, rows)
}

func assertAddressTaken(t *testing.T, rows []Row, function string) {
	t.Helper()
	for _, row := range rows {
		if row["kind"] == "address_taken" && row["function"] == function {
			return
		}
	}
	t.Fatalf("missing address_taken function %q in %#v", function, rows)
}

func assertNotAddressTaken(t *testing.T, rows []Row, function string) {
	t.Helper()
	for _, row := range rows {
		if row["kind"] == "address_taken" && row["function"] == function {
			t.Fatalf("function %q must NOT be address_taken (statically called): %#v", function, row)
		}
	}
}

func assertInstantiatedType(t *testing.T, rows []Row, typeName string) {
	t.Helper()
	for _, row := range rows {
		if row["kind"] == "instantiated_type" && row["type"] == typeName {
			return
		}
	}
	t.Fatalf("missing instantiated_type %q in %#v", typeName, rows)
}

func assertNoInstantiatedType(t *testing.T, rows []Row, typeName string) {
	t.Helper()
	for _, row := range rows {
		if row["kind"] == "instantiated_type" && row["type"] == typeName {
			t.Fatalf("unexpected instantiated_type %q (must canonicalize to underlying): %#v", typeName, row)
		}
	}
}

func assertNoMethodSetKeyedBy(t *testing.T, rows []Row, typeName string) {
	t.Helper()
	for _, row := range rows {
		if row["kind"] == "method_set" && row["type"] == typeName {
			t.Fatalf("unexpected method_set keyed by %q (must canonicalize to underlying): %#v", typeName, row)
		}
	}
}

func assertNoDuplicateMethodSetStableKey(t *testing.T, rows []Row) {
	t.Helper()
	seen := make(map[string]bool)
	for _, row := range rows {
		if row["kind"] != "method_set" {
			continue
		}
		key, ok := row["stable_key"].(string)
		if !ok || key == "" {
			t.Fatalf("method_set row missing stable_key: %#v", row)
		}
		if seen[key] {
			t.Fatalf("duplicate method_set stable_key %q (alias↔instantiation collision): %#v", key, row)
		}
		seen[key] = true
	}
}

func assertNoDuplicateFunctionStableKey(t *testing.T, rows []Row) {
	t.Helper()
	seen := make(map[string]bool)
	for _, row := range rows {
		switch row["kind"] {
		case "function", "method", "init_function":
		default:
			continue
		}
		key, ok := row["stable_key"].(string)
		if !ok || key == "" {
			t.Fatalf("%v row missing stable_key: %#v", row["kind"], row)
		}
		if seen[key] {
			t.Fatalf("duplicate %v stable_key %q (cross-harvest-path collision): %#v", row["kind"], key, row)
		}
		seen[key] = true
	}
}

func assertSingleMethodSetKeyedBy(t *testing.T, rows []Row, typeName string) {
	t.Helper()
	count := 0
	for _, row := range rows {
		if row["kind"] == "method_set" && row["type"] == typeName {
			count++
		}
	}
	if count != 1 {
		t.Fatalf("expected exactly one method_set keyed by %q, got %d in %#v", typeName, count, rows)
	}
}

func assertMethodSetContains(t *testing.T, rows []Row, typeName string, method string) {
	t.Helper()
	for _, row := range rows {
		if row["kind"] != "method_set" || row["type"] != typeName {
			continue
		}
		methods, ok := row["methods"].([]string)
		if !ok {
			t.Fatalf("method_set %q has non-[]string methods: %#v", typeName, row["methods"])
		}
		for _, m := range methods {
			if m == method {
				return
			}
		}
		t.Fatalf("method_set %q missing method %q: %#v", typeName, method, methods)
	}
	t.Fatalf("missing method_set keyed by instantiated type %q in %#v", typeName, rows)
}

func assertMethodWithReceiver(t *testing.T, rows []Row, name string, receiver string) {
	t.Helper()
	for _, row := range rows {
		if row["kind"] == "method" && row["name"] == name && row["receiver"] == receiver {
			return
		}
	}
	t.Fatalf("missing method name=%q receiver=%q in %#v", name, receiver, rows)
}

func assertDynamicDispatchMethod(t *testing.T, rows []Row, method string) {
	t.Helper()
	for _, row := range rows {
		if row["kind"] == "dynamic_dispatch" && row["method"] == method {
			return
		}
	}
	t.Fatalf("missing dynamic_dispatch with method %q in %#v", method, rows)
}

func assertRTAEdge(t *testing.T, rows []Row, caller string, kind string, callee string) {
	t.Helper()
	for _, row := range rows {
		if row["kind"] == "rta_edge" &&
			row["caller"] == caller &&
			row["edge_kind"] == kind &&
			row["callee"] == callee {
			return
		}
	}
	t.Fatalf("missing rta_edge %s --%s--> %s in %#v", caller, kind, callee, rows)
}

func writeGoWorkspaceFixture(t *testing.T, checkedInWork bool) string {
	t.Helper()
	files := map[string]string{
		"services/app/go.mod": "module example.test/app\n\ngo 1.24\n\nrequire example.test/money v0.0.0\n",
		"services/app/main.go": `package main

import "example.test/money"

func main() {
	money.Pay()
}
`,
		"libs/money/go.mod": "module example.test/money\n\ngo 1.24\n",
		"libs/money/money.go": `package money

func Pay() {}
`,
	}
	if checkedInWork {
		files["go.work"] = "go 1.24\n\nuse (\n\t./services/app\n\t./libs/money\n)\n"
	}
	return writeFixture(t, files)
}

func writeFixture(t *testing.T, files map[string]string) string {
	t.Helper()
	root := t.TempDir()
	for relative, contents := range files {
		path := filepath.Join(root, filepath.FromSlash(relative))
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			t.Fatalf("mkdir fixture: %v", err)
		}
		if err := os.WriteFile(path, []byte(contents), 0o644); err != nil {
			t.Fatalf("write fixture: %v", err)
		}
	}
	return root
}

func TestScopeFilesDropOnlyFileAnchoredRowsOutsideTheScan(t *testing.T) {
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.com\n\ngo 1.24\n",
		"kept/kept.go": `package kept

type Speaker interface{ Speak() string }

type Dog struct{}

func (Dog) Speak() string { return "woof" }

func Kept() string {
	var s Speaker = Dog{}
	return s.Speak()
}
`,
		"dropped/dropped.go": `package dropped

func Dropped() int { return helper() }

func helper() int { return 1 }
`,
	})

	config := Config{
		Root:         root,
		ModuleRoots:  []string{"."},
		Patterns:     []string{"./..."},
		IncludeTests: false,
	}
	unscoped, err := Emit(config)
	if err != nil {
		t.Fatalf("emit unscoped: %v", err)
	}

	config.ScopeFiles = map[string]bool{"kept/kept.go": true}
	scoped, err := Emit(config)
	if err != nil {
		t.Fatalf("emit scoped: %v", err)
	}

	// A file-anchored row naming an undiscovered file is what the kernel drops on
	// receipt, so the sidecar must not produce it.
	for _, row := range scoped {
		file, _ := row["file"].(string)
		if fileAnchoredKinds[row["kind"].(string)] && file != "" && file != "kept/kept.go" {
			t.Fatalf("scoped emit kept an out-of-scope row: %#v", row)
		}
	}
	assertKind(t, scoped, "function")
	assertKind(t, scoped, "callsite")

	// The whole-program families are the rapid-type set polint's own RTA runs on.
	// Narrowing them would change answers, so the scope must not touch them.
	for _, kind := range []string{"method_set", "instantiated_type"} {
		if countKind(unscoped, kind) != countKind(scoped, kind) {
			t.Fatalf(
				"scope changed the %s count: %d unscoped, %d scoped",
				kind,
				countKind(unscoped, kind),
				countKind(scoped, kind),
			)
		}
	}
}

func TestScopeFilesKeepRowsThatNameNoFile(t *testing.T) {
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.com\n\ngo 1.24\n",
		"a/a.go": "package a\n\nfunc A() {}\n",
	})

	// A scope that matches nothing still keeps every row without a file, because
	// `lower_optional_file_span` lowers those to a location-less fact rather than
	// dropping them. This filter has to stay a strict subset of the kernel's.
	rows, err := Emit(Config{
		Root:         root,
		ModuleRoots:  []string{"."},
		Patterns:     []string{"./..."},
		IncludeTests: false,
		ScopeFiles:   map[string]bool{"nothing/matches.go": true},
	})
	if err != nil {
		t.Fatalf("emit: %v", err)
	}
	for _, row := range rows {
		file, _ := row["file"].(string)
		kind, _ := row["kind"].(string)
		if fileAnchoredKinds[kind] && file != "" {
			t.Fatalf("a located row survived an empty scope: %#v", row)
		}
	}
	assertKind(t, rows, "session_end")
}

func countKind(rows []Row, kind string) int {
	count := 0
	for _, row := range rows {
		if row["kind"] == kind {
			count++
		}
	}
	return count
}
