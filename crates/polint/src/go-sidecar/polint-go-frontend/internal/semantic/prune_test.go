package semantic

import (
	"fmt"
	"sort"
	"strings"
	"testing"
)

const pruneFixture = `package main

const debug = false

func danger() {}

func safe() {}

func helper() { danger() }

func generic[T any](value T) T {
	if false {
		danger()
	}
	return value
}

func main() {
	if false {
		helper()
	}
	if debug {
		danger()
	}
	run := safe
	if false {
		run = danger
	}
	run()
	if true {
		safe()
	} else {
		danger()
	}
	_ = generic(1)
}
`

func emitPruneFixture(t *testing.T) []Row {
	t.Helper()
	root := writeFixture(t, map[string]string{
		"go.mod":  "module example.test/prune\n\ngo 1.24\n",
		"main.go": pruneFixture,
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, CallGraph: true})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}
	return rows
}

// spanLine is the 1-based start line of a row's span.
func spanLine(row Row) int {
	span, ok := row["span"].(*Span)
	if !ok || span == nil {
		return 0
	}
	return span.StartLine
}

// A call in a branch a constant condition rules out is not a call: it gets a
// `dead_call` row instead of a `callsite` row, and a value assigned there does
// not reach variable-type analysis.
func TestConstantBranchesArePrunedBeforeCallsAndCallGraph(t *testing.T) {
	rows := emitPruneFixture(t)

	var callsites []string
	for _, row := range rowsOfKind(rows, "callsite") {
		if row["caller"] == "example.test/prune.main" || row["caller"] == "example.test/prune.generic[int]" {
			callee, _ := row["static_callee"].(string)
			callsites = append(callsites, fmt.Sprintf("%d %s %s", spanLine(row), row["caller"], callee))
		}
	}
	sort.Strings(callsites)
	for _, line := range callsites {
		if strings.HasSuffix(line, ".danger") || strings.HasSuffix(line, ".helper") {
			t.Fatalf("a call in a pruned branch is still a callsite: %s\nall: %v", line, callsites)
		}
	}

	var dead []string
	for _, row := range rowsOfKind(rows, "dead_call") {
		dead = append(dead, fmt.Sprintf("%d %s", spanLine(row), row["caller"]))
	}
	sort.Strings(dead)
	want := []string{
		"13 example.test/prune.generic",
		"13 example.test/prune.generic[int]",
		"20 example.test/prune.main",
		"23 example.test/prune.main",
		"33 example.test/prune.main",
	}
	if strings.Join(dead, "\n") != strings.Join(want, "\n") {
		t.Fatalf("dead calls:\n%s\nwant:\n%s", strings.Join(dead, "\n"), strings.Join(want, "\n"))
	}

	var runTargets []string
	for _, row := range rowsOfKind(rows, "call_edges") {
		if row["caller"] == "example.test/prune.main" {
			names, _ := row["callees"].([]string)
			runTargets = append(runTargets, names...)
		}
	}
	if fmt.Sprint(runTargets) != "[example.test/prune.safe]" {
		t.Fatalf("the function value assigned in a pruned branch reached the call graph: %v", runTargets)
	}
}

// A body without a constant branch is left exactly as go/ssa built it.
func TestPruningLeavesBodiesWithoutConstantBranchesAlone(t *testing.T) {
	rows := emitPruneFixture(t)
	for _, row := range rowsOfKind(rows, "dead_call") {
		if row["caller"] == "example.test/prune.helper" {
			t.Fatalf("helper has no constant branch: %v", row)
		}
	}
	var helperCalls int
	for _, row := range rowsOfKind(rows, "callsite") {
		if row["caller"] == "example.test/prune.helper" {
			helperCalls++
		}
	}
	if helperCalls != 1 {
		t.Fatalf("want helper's one call, got %d", helperCalls)
	}
}
