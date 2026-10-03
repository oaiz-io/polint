package semantic

import (
	"fmt"
	"strings"
	"testing"
)

const flowFixture = `package flow

type carrier struct{ field string }

type store interface{ Save(value string) }

type memory struct{ last string }

func (m *memory) Save(value string) { m.last = value }

func sink(value string) {}

func clean(value string) string { return "safe" }

func carry(value string) string { return value }

func fields(token string) {
	c := carrier{field: token}
	sink(c.field)
}

func helpers(token string) string {
	return carry(token) + clean(token)
}

func closures(token string) {
	run := func() { sink(token) }
	run()
}

func dispatch(s store, token string) {
	s.Save(token)
}

func maps(token string) map[string]string {
	values := map[string]string{}
	values["key"] = token
	return values
}

func dead(token string) {
	if false {
		sink(token)
	}
}

func use() {
	dispatch(&memory{}, "x")
}
`

func emitFlowFixture(t *testing.T) map[string]*flowBody {
	t.Helper()
	root := writeFixture(t, map[string]string{
		"go.mod":  "module example.test/flow\n\ngo 1.24\n",
		"flow.go": flowFixture,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, Dataflow: true})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}
	bodies := make(map[string]*flowBody)
	for _, row := range rowsOfKind(rows, "flow_body") {
		body, ok := row["flow"].(*flowBody)
		if !ok {
			t.Fatalf("flow_body row without a body: %v", row)
		}
		bodies[row["function"].(string)] = body
	}
	return bodies
}

// render prints a body's statements with slots named by the body's names and
// constants by their text, so a test reads like the program.
func render(body *flowBody) []string {
	names := make(map[int]string)
	for _, name := range body.Names {
		names[name.Slot] = name.Name
	}
	for _, global := range body.Globals {
		names[global.Slot] = "&" + global.Name
	}
	operand := func(slot int) string {
		if slot < 0 {
			return fmt.Sprintf("%q", body.Consts[-1-slot])
		}
		if name, ok := names[slot]; ok {
			return name
		}
		return fmt.Sprintf("t%d", slot)
	}
	ref := func(encoded int) string {
		if encoded == 0 {
			return "_"
		}
		return operand(encoded - 1)
	}
	var out []string
	for _, stmt := range body.Stmts {
		var line string
		switch stmt.Op {
		case "copy":
			line = fmt.Sprintf("%s = %s", ref(stmt.Dst), operand(stmt.A))
		case "load":
			line = fmt.Sprintf("%s = %s.%s", ref(stmt.Dst), operand(stmt.A), stmt.Step)
		case "addr":
			line = fmt.Sprintf("%s = &%s.%s", ref(stmt.Dst), operand(stmt.A), stmt.Step)
		case "store":
			target := "*" + operand(stmt.A)
			if stmt.Step != "" {
				target = operand(stmt.A) + "." + stmt.Step
			}
			line = fmt.Sprintf("%s <- %s", target, operand(stmt.B))
		case "alloc":
			line = fmt.Sprintf("%s = alloc", ref(stmt.Dst))
		case "closure":
			args := make([]string, len(stmt.Args))
			for i, arg := range stmt.Args {
				args[i] = operand(arg)
			}
			line = fmt.Sprintf("%s = closure %s [%s]", ref(stmt.Dst), stmt.Fn, strings.Join(args, ", "))
		case "call":
			args := make([]string, len(stmt.Args))
			for i, arg := range stmt.Args {
				args[i] = operand(arg)
			}
			callee := strings.Join(stmt.Callees, "|")
			if stmt.Builtin != "" {
				callee = "builtin " + stmt.Builtin
			}
			line = fmt.Sprintf("%s = call %s %s(%s)", ref(stmt.Dst), stmt.Algorithm, callee, strings.Join(args, ", "))
			if stmt.FV != 0 {
				line += " fv=" + ref(stmt.FV)
			}
		case "ret":
			args := make([]string, len(stmt.Args))
			for i, arg := range stmt.Args {
				args[i] = operand(arg)
			}
			line = "return " + strings.Join(args, ", ")
		default:
			line = stmt.Op
		}
		out = append(out, line)
	}
	return out
}

func TestFlowBodiesDescribeEachValueMove(t *testing.T) {
	bodies := emitFlowFixture(t)
	cases := map[string][]string{
		// The local is a memory cell (its field is addressed); the composite
		// literal is built in a cell of its own and copied in.
		"example.test/flow.fields": {
			"c = alloc",
			"complit = alloc",
			"t3 = &complit.*.f:field",
			"*t3 <- token",
			"t4 = complit.*",
			"*c <- t4",
			"t5 = &c.*.f:field",
			"t6 = t5.*",
			"t7 = call static example.test/flow.sink(t6)",
			"return ",
		},
		"example.test/flow.helpers": {
			"t1 = call static example.test/flow.carry(token)",
			"t2 = call static example.test/flow.clean(token)",
			"t3 = t1",
			"t3 = t2",
			"return t3",
		},
		// A captured parameter moves to a heap cell the closure binds; calling
		// the closure through its variable is value flow.
		"example.test/flow.closures": {
			"t1 = alloc",
			"*t1 <- token",
			"t2 = closure example.test/flow.closures$1 [t1]",
			"t3 = call vta example.test/flow.closures$1() fv=t2",
			"return ",
		},
		"example.test/flow.dispatch": {
			"t2 = call vta (*example.test/flow.memory).Save(s, token)",
			"return ",
		},
		"example.test/flow.maps": {
			"t1 = alloc",
			`t1.[] <- "key"`,
			"t1.[] <- token",
			"return t1",
		},
		"example.test/flow.dead": {
			"return ",
		},
	}
	for name, want := range cases {
		body := bodies[name]
		if body == nil {
			t.Fatalf("no flow body for %s; have %d bodies", name, len(bodies))
		}
		got := render(body)
		if strings.Join(got, "\n") != strings.Join(want, "\n") {
			t.Errorf("%s:\n%s\nwant:\n%s", name, strings.Join(got, "\n"), strings.Join(want, "\n"))
		}
	}
	closure := bodies["example.test/flow.closures$1"]
	if closure == nil || len(closure.Free) != 1 {
		t.Fatalf("the closure body has its free variable: %+v", closure)
	}
	if got := strings.Join(render(closure), "\n"); got != "t1 = token.*\nt2 = call static example.test/flow.sink(t1)\nreturn " {
		t.Errorf("closure body:\n%s", got)
	}
}

// Without --dataflow no flow body is emitted.
func TestFlowBodiesOnlyOnRequest(t *testing.T) {
	root := writeFixture(t, map[string]string{
		"go.mod":  "module example.test/flow\n\ngo 1.24\n",
		"flow.go": flowFixture,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, CallGraph: true})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}
	if bodies := rowsOfKind(rows, "flow_body"); len(bodies) != 0 {
		t.Fatalf("flow bodies without --dataflow: %d", len(bodies))
	}
}

// Each slot carries the kind of value its type holds, so a query can declare
// contexts, booleans or numbers unable to carry what it tracks.
func TestFlowBodiesClassifySlotsByValueKind(t *testing.T) {
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.test/kinds\n\ngo 1.24\n",
		"kinds.go": `package kinds

import (
	"context"
	"time"
)

type level int

func kinds(ctx context.Context, ok bool, n int, d time.Duration, l level, s string, p *int) {}

func plain(s string) string { return s }
`,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, Dataflow: true})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}
	bodies := make(map[string]*flowBody)
	for _, row := range rowsOfKind(rows, "flow_body") {
		bodies[row["function"].(string)] = row["flow"].(*flowBody)
	}
	kinds := bodies["example.test/kinds.kinds"]
	if kinds == nil {
		t.Fatalf("no flow body for kinds: %v", bodies)
	}
	var got strings.Builder
	for _, slot := range kinds.Params {
		got.WriteByte(kinds.Classes[slot])
	}
	if got.String() != "cbnnn.." {
		t.Errorf("parameter classes = %q, want %q", got.String(), "cbnnn..")
	}
	if plain := bodies["example.test/kinds.plain"]; plain == nil || plain.Classes != "" {
		t.Errorf("a body without classed slots omits its classes: %+v", plain)
	}
}

// A body SSA synthesizes belongs to the package of the function it stands for.
func TestFlowBodiesOfSynthesizedFunctionsNameTheirPackage(t *testing.T) {
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.test/wrap\n\ngo 1.24\n",
		"wrap.go": `package wrap

type namer interface{ Name() string }

type inner struct{ name string }

func (i *inner) Name() string { return i.name }

type outer struct{ *inner }

func use(o outer) func() string {
	var n namer = o
	_ = n.Name()
	return o.Name
}
`,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, Dataflow: true})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}
	synthesized := 0
	for _, row := range rowsOfKind(rows, "flow_body") {
		name := row["function"].(string)
		if row["package_path"] != "example.test/wrap" {
			t.Errorf("%s belongs to %q", name, row["package_path"])
		}
		if strings.Contains(name, "$bound") || strings.HasPrefix(name, "(example.test/wrap.outer)") {
			synthesized++
		}
	}
	if synthesized == 0 {
		t.Fatalf("the fixture has no synthesized body: %v", rowsOfKind(rows, "flow_body"))
	}
}

// A call through a function value whose one value SSA knows is resolved by value
// flow: its callsite row says so, and its flow statement is `vta`. A call that
// names its function or method is `static`.
func TestCallsThroughFunctionValuesAreMarked(t *testing.T) {
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.test/values\n\ngo 1.24\n",
		"values.go": `package values

type holder struct{}

func (holder) method() {}

func target() {}

func direct() { target() }

func viaVariable() {
	run := target
	run()
}

func viaMethodValue() {
	run := holder{}.method
	run()
}

func viaMethod() { holder{}.method() }
`,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, Dataflow: true})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}
	viaValue := make(map[string]bool)
	for _, row := range rowsOfKind(rows, "callsite") {
		if row["status"] != "resolved_static" {
			continue
		}
		caller := row["caller"].(string)
		marked, _ := row["via_value"].(bool)
		viaValue[caller] = viaValue[caller] || marked
	}
	want := map[string]bool{
		"example.test/values.direct":         false,
		"example.test/values.viaVariable":    true,
		"example.test/values.viaMethodValue": true,
		"example.test/values.viaMethod":      false,
	}
	for caller, marked := range want {
		got, ok := viaValue[caller]
		if !ok {
			t.Fatalf("no static callsite row for %s: %v", caller, viaValue)
		}
		if got != marked {
			t.Errorf("%s via_value = %v, want %v", caller, got, marked)
		}
	}
	algorithms := make(map[string]string)
	for _, row := range rowsOfKind(rows, "flow_body") {
		body := row["flow"].(*flowBody)
		for _, stmt := range body.Stmts {
			if stmt.Op == "call" && stmt.Algorithm != "" {
				algorithms[row["function"].(string)] = stmt.Algorithm
			}
		}
	}
	if algorithms["example.test/values.viaVariable"] != "vta" || algorithms["example.test/values.direct"] != "static" {
		t.Errorf("flow call algorithms: %v", algorithms)
	}
}
