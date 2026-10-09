package semantic

import (
	"go/token"
	"runtime"
	"testing"

	"golang.org/x/tools/go/packages"
	"golang.org/x/tools/go/ssa"
)

func TestLoadModeTypesDependenciesFromExportData(t *testing.T) {
	if loadMode(Config{})&packages.NeedDeps != 0 {
		t.Fatalf("a normal run must type dependencies from export data, not load them from source")
	}
	if loadMode(Config{EmitRTAEdges: true})&packages.NeedDeps == 0 {
		t.Fatalf("the RTA oracle comparison needs the whole program from source")
	}
}

// A main package used to pull every dependency body into the SSA program, which
// is most of the memory a whole-program load costs and nothing any row reads.
func TestBuildProgramBuildsBodiesOnlyForRootPackages(t *testing.T) {
	root := writeFixture(t, map[string]string{
		"go.mod": "module example.test/fixture\n\ngo 1.24\n",
		"main.go": `package main

import "strings"

func main() {
	println(strings.ToUpper("ok"))
}
`,
	})
	env, cleanup, err := goPackageEnv(root, []string{"."})
	if err != nil {
		t.Fatalf("goPackageEnv: %v", err)
	}
	defer cleanup()
	config := Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}}
	pkgs, err := packages.Load(&packages.Config{
		Mode: loadMode(config),
		Dir:  root,
		Fset: token.NewFileSet(),
		Env:  env,
	}, "./...")
	if err != nil {
		t.Fatalf("packages.Load: %v", err)
	}

	prog, roots := buildProgram(pkgs, config)

	if len(roots) != 1 || roots[0] == nil || roots[0].Pkg.Path() != "example.test/fixture" {
		t.Fatalf("roots = %v, want only the fixture package", roots)
	}
	stringsPkg := prog.ImportedPackage("strings")
	if stringsPkg == nil {
		t.Fatalf("the imported package must still exist in the program")
	}
	for _, member := range stringsPkg.Members {
		if fn, ok := member.(*ssa.Function); ok && fn.Blocks != nil {
			t.Fatalf("dependency function %s has an SSA body; only roots should", fn)
		}
	}
	mainFn := roots[0].Func("main")
	if mainFn == nil || mainFn.Blocks == nil {
		t.Fatalf("the root's main function must have an SSA body")
	}
	var callee string
	for _, block := range mainFn.Blocks {
		for _, instr := range block.Instrs {
			if call, ok := instr.(ssa.CallInstruction); ok {
				if static := call.Common().StaticCallee(); static != nil {
					callee = static.String()
				}
			}
		}
	}
	if callee != "strings.ToUpper" {
		t.Fatalf("a call into a dependency must still resolve statically, got %q", callee)
	}
}

func TestEmitReportsPeakResidentSetOnEveryPhase(t *testing.T) {
	if runtime.GOOS == "windows" || runtime.GOOS == "plan9" || runtime.GOOS == "js" || runtime.GOOS == "wasip1" {
		t.Skip("no resident-set high-water mark on this platform")
	}
	root := writeFixture(t, map[string]string{
		"go.mod":  "module example.test/fixture\n\ngo 1.24\n",
		"main.go": "package main\n\nfunc main() {}\n",
	})
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}
	seen := 0
	for _, row := range rows {
		kind := row["kind"]
		if kind != "phase" && kind != "session_end" {
			continue
		}
		seen++
		rss, ok := row["peak_rss_bytes"].(uint64)
		if !ok || rss == 0 {
			t.Fatalf("%v row carries no peak_rss_bytes: %#v", kind, row)
		}
	}
	if seen == 0 {
		t.Fatalf("no phase or session_end rows in %#v", rows)
	}
}
