package semantic

import (
	"encoding/json"
	"reflect"
	"sort"
	"strings"
	"testing"
)

const typedFixture = `package main

import "fmt"

type Speaker interface{ Speak() string }

type Dog struct {
	Name  string ` + "`json:\"name\"`" + `
	Owner *Dog
}

func (d Dog) Speak() string { return d.Name }

type Cat struct{}

func (*Cat) Speak() string { return "meow" }

type Failure struct{}

func (*Failure) Error() string { return "failed" }

type Box[T any] struct{ value T }

func Map[T, U any](in []T, f func(T) U) []U {
	out := make([]U, 0, len(in))
	for _, v := range in {
		out = append(out, f(v))
	}
	return out
}

type Count int

func run(s Speaker) string { return s.Speak() }

func lonely(s Speaker) string { return s.Speak() }

func apply(f func(int) int) int { return f(1) }

func double(v int) int { return v * 2 }

func main() {
	run(Dog{Name: "rex"})
	_ = Box[int]{value: 1}
	_ = Map([]int{1}, func(v int) string { return fmt.Sprint(v) })
	_ = Count(3)
	_ = apply(double)
	go fmt.Println("async")
	defer fmt.Println("later")
	var err error = &Failure{}
	_ = err
	_ = &Cat{}
}
`

func emitTypedFixture(t *testing.T) []Row {
	t.Helper()
	root := writeFixture(t, map[string]string{
		"go.mod":  "module example.test/typed\n\ngo 1.24\n",
		"main.go": typedFixture,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}
	return rows
}

func rowsOfKind(rows []Row, kind string) []Row {
	var out []Row
	for _, row := range rows {
		if row["kind"] == kind {
			out = append(out, row)
		}
	}
	return out
}

func TestCallsiteRowsCarryCalleeOriginReceiverAndMode(t *testing.T) {
	rows := emitTypedFixture(t)
	var sawGeneric, sawReceiver, sawGo, sawDefer bool
	for _, row := range rowsOfKind(rows, "callsite") {
		if row["static_callee_origin"] == "example.test/typed.Map" {
			sawGeneric = true
		}
		if row["caller"] == "example.test/typed.run" && row["static_callee"] != nil {
			t.Fatalf("an interface call must not be static: %#v", row)
		}
		if row["caller"] == "example.test/typed.run" && row["receiver_type"] == "example.test/typed.Speaker" {
			sawReceiver = true
		}
		switch row["mode"] {
		case "go", "defer":
			span, ok := row["span"].(*Span)
			if !ok || span.EndByte <= span.StartByte {
				t.Fatalf("%v call has no call-expression span: %#v", row["mode"], row)
			}
			if !strings.HasPrefix(typedFixture[span.StartByte:span.EndByte], "fmt.Println(") {
				t.Fatalf("%v span covers %q, want the call expression", row["mode"], typedFixture[span.StartByte:span.EndByte])
			}
			if row["mode"] == "go" {
				sawGo = true
			} else {
				sawDefer = true
			}
		}
	}
	if !sawGeneric || !sawReceiver || !sawGo || !sawDefer {
		t.Fatalf("generic origin %v, interface receiver %v, go %v, defer %v", sawGeneric, sawReceiver, sawGo, sawDefer)
	}
}

func TestCallEdgesUseVariableTypeAnalysisThenClassHierarchy(t *testing.T) {
	rows := emitTypedFixture(t)
	callees := map[string][]string{}
	algorithms := map[string]string{}
	for _, row := range rowsOfKind(rows, "call_edge") {
		caller := row["caller"].(string)
		callees[caller] = append(callees[caller], row["callee"].(string))
		algorithms[caller] = row["algorithm"].(string)
	}
	for caller := range callees {
		sort.Strings(callees[caller])
	}
	// Only a Dog reaches run's parameter.
	if got, want := callees["example.test/typed.run"], []string{"(example.test/typed.Dog).Speak"}; !reflect.DeepEqual(got, want) || algorithms["example.test/typed.run"] != "vta" {
		t.Fatalf("run edges = %v (%s), want %v by vta", got, algorithms["example.test/typed.run"], want)
	}
	// Nothing calls lonely, so no type reaches its parameter: every implementer.
	if got, want := callees["example.test/typed.lonely"], []string{"(*example.test/typed.Cat).Speak", "(example.test/typed.Dog).Speak"}; !reflect.DeepEqual(got, want) || algorithms["example.test/typed.lonely"] != "cha" {
		t.Fatalf("lonely edges = %v (%s), want %v by cha", got, algorithms["example.test/typed.lonely"], want)
	}
	if got := callees["example.test/typed.apply"]; !reflect.DeepEqual(got, []string{"example.test/typed.double"}) {
		t.Fatalf("apply edges = %v, want the function value passed to it", got)
	}
}

func TestTypeFactsEmitDeclarationsImplementsInstantiationsAndConversions(t *testing.T) {
	rows := emitTypedFixture(t)
	interfaces := rowsOfKind(rows, "interface")
	if len(interfaces) != 1 || interfaces[0]["type"] != "example.test/typed.Speaker" || !reflect.DeepEqual(interfaces[0]["methods"], []string{"Speak"}) {
		t.Fatalf("interface rows = %#v", interfaces)
	}
	var tagged bool
	for _, row := range rowsOfKind(rows, "field") {
		if row["type"] == "example.test/typed.Dog" && row["name"] == "Name" && row["tag"] == `json:"name"` && row["field_type"] == "string" {
			tagged = true
		}
	}
	if !tagged {
		t.Fatalf("Dog.Name field with its tag missing from %#v", rowsOfKind(rows, "field"))
	}
	implements := map[string]bool{}
	for _, row := range rowsOfKind(rows, "implements") {
		implements[row["type"].(string)+" "+row["interface"].(string)] = row["via_pointer"].(bool)
	}
	for pair, viaPointer := range map[string]bool{
		"example.test/typed.Dog example.test/typed.Speaker": false,
		"example.test/typed.Cat example.test/typed.Speaker": true,
		"example.test/typed.Failure error":                  true,
	} {
		got, ok := implements[pair]
		if !ok || got != viaPointer {
			t.Fatalf("implements %q = %v (present %v), want via_pointer %v; all: %v", pair, got, ok, viaPointer, implements)
		}
	}
	if _, ok := implements["example.test/typed.Count example.test/typed.Speaker"]; ok {
		t.Fatalf("Count has no Speak method: %v", implements)
	}
	var boxInt, mapInstance bool
	for _, row := range rowsOfKind(rows, "instantiation") {
		args := row["type_args"].([]string)
		if row["generic"] == "example.test/typed.Box" && reflect.DeepEqual(args, []string{"int"}) {
			boxInt = true
		}
		if row["generic"] == "example.test/typed.Map" && row["generic_kind"] == "func" && reflect.DeepEqual(args, []string{"int", "string"}) {
			mapInstance = true
		}
	}
	if !boxInt || !mapInstance {
		t.Fatalf("instantiations Box[int] %v, Map[int,string] %v in %#v", boxInt, mapInstance, rowsOfKind(rows, "instantiation"))
	}
	conversions := rowsOfKind(rows, "conversion")
	if len(conversions) != 1 || conversions[0]["type"] != "example.test/typed.Count" {
		t.Fatalf("conversion rows = %#v", conversions)
	}
	builtins := map[string]bool{}
	for _, row := range rowsOfKind(rows, "builtin_call") {
		builtins[row["name"].(string)] = true
	}
	for _, name := range []string{"make", "len", "append"} {
		if !builtins[name] {
			t.Fatalf("builtin call %q missing from %#v", name, rowsOfKind(rows, "builtin_call"))
		}
	}
	var param bool
	for _, row := range rowsOfKind(rows, "param") {
		if row["function"] == "example.test/typed.run" && row["name"] == "s" && row["type"] == "example.test/typed.Speaker" {
			param = true
		}
	}
	if !param {
		t.Fatalf("run's parameter missing from %#v", rowsOfKind(rows, "param"))
	}
}

// The typed rows feed stable keys and digests, so two runs must agree byte for
// byte apart from the timing rows.
func TestTypedRowsAreDeterministic(t *testing.T) {
	encode := func(rows []Row) []string {
		var out []string
		for _, row := range rows {
			switch row["kind"] {
			case "phase", "session_begin", "session_end":
				continue
			}
			encoded, err := json.Marshal(row)
			if err != nil {
				t.Fatalf("marshal: %v", err)
			}
			out = append(out, string(encoded))
		}
		return out
	}
	first := encode(emitTypedFixture(t))
	for i := 0; i < 3; i++ {
		if again := encode(emitTypedFixture(t)); !reflect.DeepEqual(first, again) {
			t.Fatalf("run %d differs from the first run", i+2)
		}
	}
}

const valueReceiverFixture = `package main

import "io"

type Greeter struct{ Name string }

func helper(s string) string { return s }

func (g Greeter) Greet() string { return helper(g.Name) }

func write(w io.Writer) { _, _ = w.Write(nil) }

func main() {
	_ = Greeter{Name: "a"}.Greet()
	write(nil)
}
`

func emitValueReceiverFixture(t *testing.T) []Row {
	t.Helper()
	root := writeFixture(t, map[string]string{
		"go.mod":  "module example.test/values\n\ngo 1.24\n",
		"main.go": valueReceiverFixture,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}
	return rows
}

func TestValueReceiverMethodBodiesAreEmitted(t *testing.T) {
	rows := emitValueReceiverFixture(t)
	const declared = "(example.test/values.Greeter).Greet"
	var sawFunction, sawCall bool
	for _, row := range rows {
		switch row["kind"] {
		case "method":
			if row["qualified"] == declared {
				span, ok := row["span"].(*Span)
				if !ok || span.EndByte <= span.StartByte {
					t.Fatalf("the declared value method has no declaration span: %#v", row)
				}
				sawFunction = true
			}
		case "callsite":
			if row["caller"] == declared && row["static_callee"] == "example.test/values.helper" {
				sawCall = true
			}
		}
	}
	if !sawFunction || !sawCall {
		t.Fatalf("declared value method row %v, call inside its body %v", sawFunction, sawCall)
	}
}

func TestInterfaceCallWithOnlyDependencyImplementationsNamesTheInterfaceMethod(t *testing.T) {
	rows := emitValueReceiverFixture(t)
	var edges []Row
	for _, row := range rowsOfKind(rows, "call_edge") {
		if row["caller"] == "example.test/values.write" {
			edges = append(edges, row)
		}
	}
	if len(edges) != 1 {
		t.Fatalf("want one edge for the io.Writer call, got %#v", edges)
	}
	if edges[0]["algorithm"] != "type_hierarchy" || edges[0]["callee"] != "io.Writer.Write" {
		t.Fatalf("want a type_hierarchy edge to io.Writer.Write, got %#v", edges[0])
	}
}
