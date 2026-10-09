package semantic

import (
	"sort"
	"testing"
)

// One generic instance can be spelled through a type alias in one package and
// through the aliased type in another; SSA names the shared instance by
// whichever spelling it met first, which the parallel builder decides.
func TestInstancesAreNamedWithAliasesResolved(t *testing.T) {
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.test/names\n\ngo 1.24\n",
		"a/a.go": `package a

import "slices"

type Item struct{}

type Box[T any] struct{ v T }

func (b *Box[T]) Get() T { return b.v }

func UseA() bool {
	_ = (&Box[Item]{}).Get()
	return slices.Contains([]Item{}, Item{})
}
`,
		"b/b.go": `package b

import (
	"slices"

	"example.test/names/a"
)

type Alias = a.Item

func UseB() bool {
	_ = (&a.Box[Alias]{}).Get()
	return slices.Contains([]Alias{}, Alias{})
}
`,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, CallGraph: true})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}
	var callees []string
	for _, row := range rowsOfKind(rows, "callsite") {
		if callee, ok := row["static_callee"].(string); ok && (callee != "slices.init" && callee != "example.test/names/a.init") {
			callees = append(callees, callee)
		}
	}
	sort.Strings(callees)
	want := []string{
		"(*example.test/names/a.Box[example.test/names/a.Item]).Get",
		"(*example.test/names/a.Box[example.test/names/a.Item]).Get",
		"slices.Contains[[]example.test/names/a.Item, example.test/names/a.Item]",
		"slices.Contains[[]example.test/names/a.Item, example.test/names/a.Item]",
	}
	if len(callees) != len(want) {
		t.Fatalf("static callees = %v, want %v", callees, want)
	}
	for i := range want {
		if callees[i] != want[i] {
			t.Fatalf("static callees = %v, want %v", callees, want)
		}
	}
}
