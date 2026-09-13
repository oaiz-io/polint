package semantic

import (
	"fmt"
	"go/ast"
	"go/token"
	"go/types"
	"io"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"strconv"
	"strings"
	"time"

	"golang.org/x/mod/modfile"
	"golang.org/x/tools/go/callgraph"
	"golang.org/x/tools/go/callgraph/rta"
	"golang.org/x/tools/go/packages"
	"golang.org/x/tools/go/ssa"
	"golang.org/x/tools/go/ssa/ssautil"
)

const SchemaVersion = "polint-go-semantic-3"
const XToolsVersion = "v0.49.0"
const topologyManifestMaxBytes int64 = 1_048_576

type Config struct {
	Root         string
	ModuleRoots  []string
	Patterns     []string
	IncludeTests bool
	BuildTags    []string
}

type Row map[string]any

// phaseTimer records one stage's wall time and the workload it saw, so a slow
// run can be attributed to a stage instead of guessed at.
//
// peakHeapBytes is the largest HeapAlloc observed at a stage boundary, not a
// true high-water mark: Go does not expose one without continuous sampling.
type phaseTimer struct {
	started       time.Time
	sessionStart  time.Time
	peakHeapBytes uint64
	rows          []Row
}

func newPhaseTimer() *phaseTimer {
	now := time.Now()
	return &phaseTimer{started: now, sessionStart: now}
}

func (t *phaseTimer) sampleHeap() uint64 {
	var stats runtime.MemStats
	runtime.ReadMemStats(&stats)
	if stats.HeapAlloc > t.peakHeapBytes {
		t.peakHeapBytes = stats.HeapAlloc
	}
	return t.peakHeapBytes
}

// finish closes the current stage, records it, and starts the next one.
func (t *phaseTimer) finish(phase string, workload phaseWorkload) {
	now := time.Now()
	t.rows = append(t.rows, Row{
		"kind":              "phase",
		"schema":            SchemaVersion,
		"phase":             phase,
		"elapsed_ms":        now.Sub(t.started).Milliseconds(),
		"packages":          workload.packages,
		"compiled_go_files": workload.compiledGoFiles,
		"deps_with_types":   workload.depsWithTypes,
		"rows_emitted":      workload.rowsEmitted,
		"peak_heap_bytes":   t.sampleHeap(),
	})
	t.started = now
}

func (t *phaseTimer) totalMillis() int64 {
	return time.Since(t.sessionStart).Milliseconds()
}

// phaseWorkload is what a stage saw, counted at the stage boundary.
type phaseWorkload struct {
	packages        int
	compiledGoFiles int
	depsWithTypes   int
	rowsEmitted     int
}

// countWorkload walks the loaded roots and their whole dependency graph, so
// deps_with_types shows the type-checking cost NeedDeps imposes rather than
// only the packages the patterns named.
func countWorkload(pkgs []*packages.Package) phaseWorkload {
	workload := phaseWorkload{packages: len(pkgs)}
	for _, pkg := range pkgs {
		workload.compiledGoFiles += len(pkg.CompiledGoFiles)
	}
	packages.Visit(pkgs, nil, func(pkg *packages.Package) {
		if pkg.Types != nil {
			workload.depsWithTypes++
		}
	})
	return workload
}

type Span struct {
	StartByte   int `json:"start_byte"`
	EndByte     int `json:"end_byte"`
	StartLine   int `json:"start_line"`
	StartColumn int `json:"start_column"`
	EndLine     int `json:"end_line"`
	EndColumn   int `json:"end_column"`
}

type emitter struct {
	root string
	fset *token.FileSet
	rows []Row
	// emittedMethodSetKeys coordinates the TWO method_set emitters (emitMethodSets and
	// emitInstantiatedMethodSets) so AT MOST ONE method_set row is emitted per canonical
	// stable_key across BOTH. A SAME-PACKAGE alias to a generic instantiation
	// (`type IntBox = Box[int]`) is canonicalized by emitMethodSets through types.Unalias to
	// `...Box[int]`, while emitInstantiatedMethodSets independently harvests the reachable
	// RuntimeTypes() instantiation `Box[int]` — two rows with the IDENTICAL stable_key. The
	// store does NOT dedup method_sets (they are keyed by unique declaration identity), so a
	// duplicate non-empty key is a hard `validate_unique` conflict that zeroes the ENTIRE Go
	// fact set (review #4). This emitter-scoped set suppresses the SECOND row (keep-first):
	// for the alias↔instantiation collision both rows' method lists are identical, so
	// keep-first is lossless. A package with no such collision never triggers a skip, so its
	// emitted rows are byte-identical to before this set existed.
	emittedMethodSetKeys map[string]bool
	// emittedFunctionKeys makes emitFunction idempotent per (package_id, fn.String()) stable
	// key. A concrete method VALUE of a reachable generic instantiation can be harvested via
	// TWO entry points — the per-function `ssaFunctions` walk in emitSSAPackage AND
	// emitInstantiatedMethodSets' `emitFunction` on the instantiated method-set — yielding
	// two function/method rows with the IDENTICAL stable_key. When the SAME instantiation is
	// reachable via BOTH a same-package alias (`type IntBox = Box[int]`) AND its direct
	// spelling, x/tools surfaces `(*Box[int]).Speak` through both paths, so without this guard
	// the row duplicates → `validate_unique("function", ...)` rejects the ENTIRE Go fact set
	// (a STRUCTURAL family is, correctly, NOT row-resilient) → RTA derives zero edges
	// repo-wide (review #4). Keep-first at the SOURCE keeps the validator strict while
	// suppressing the spurious duplicate. `fn.String()` is the official SSA identity, so two
	// genuinely-distinct functions never share a key; a package without the cross-path
	// collision never repeats one, so its rows stay byte-identical.
	emittedFunctionKeys map[string]bool
}

func Emit(config Config) ([]Row, error) {
	root, err := filepath.Abs(config.Root)
	if err != nil {
		return nil, fmt.Errorf("resolve root: %w", err)
	}
	moduleRoots, err := normalizeModuleRoots(config.ModuleRoots)
	if err != nil {
		return nil, err
	}
	if err := validateModuleRootFiles(root, moduleRoots); err != nil {
		return nil, err
	}
	patterns, err := validatePackagePatterns(config.Patterns)
	if err != nil {
		return nil, err
	}
	patterns = rootedPackagePatterns(moduleRoots, patterns)

	env, cleanup, err := goPackageEnv(root, moduleRoots)
	if err != nil {
		return nil, err
	}
	defer cleanup()

	fset := token.NewFileSet()
	loadConfig := &packages.Config{
		Mode: packages.NeedName |
			packages.NeedFiles |
			packages.NeedCompiledGoFiles |
			packages.NeedImports |
			// NeedDeps loads the full transitive dependency graph with type
			// info. Without it, indirectly-imported packages (e.g. reflect via
			// fmt) are type-checked only from export data, so ssautil.AllPackages
			// builds an incomplete SSA package for them. rta.Analyze then panics
			// on `reflectPkg.Members["Value"].(*ssa.Type)` when Members["Value"]
			// is nil. See golang.org/x/tools/go/callgraph/rta/rta.go.
			packages.NeedDeps |
			packages.NeedSyntax |
			packages.NeedTypes |
			packages.NeedTypesInfo |
			packages.NeedTypesSizes |
			packages.NeedModule,
		Dir:   root,
		Fset:  fset,
		Tests: config.IncludeTests,
		Env:   env,
	}
	loadConfig.BuildFlags = goBuildFlags(config.BuildTags)

	timer := newPhaseTimer()

	pkgs, err := packages.Load(loadConfig, patterns...)
	if err != nil {
		return nil, err
	}
	sort.Slice(pkgs, func(i, j int) bool { return pkgs[i].ID < pkgs[j].ID })
	workload := countWorkload(pkgs)
	timer.finish("packages_load", workload)

	prog, ssaPkgs := ssautil.AllPackages(pkgs, ssa.SanityCheckFunctions|ssa.InstantiateGenerics)
	prog.Build()
	sort.Slice(ssaPkgs, func(i, j int) bool {
		return packageID(ssaPkgs[i]) < packageID(ssaPkgs[j])
	})
	timer.finish("ssa_build", workload)

	e := &emitter{
		root:                 root,
		fset:                 fset,
		emittedMethodSetKeys: make(map[string]bool),
		emittedFunctionKeys:  make(map[string]bool),
	}
	e.add(Row{
		"kind":            "session_begin",
		"schema":          SchemaVersion,
		"go_version":      runtime.Version(),
		"x_tools_version": XToolsVersion,
	})
	for _, row := range timer.rows {
		e.add(row)
	}
	for _, pkg := range pkgs {
		e.emitPackage(pkg)
		e.emitPackageErrors(pkg)
	}
	for _, pkg := range ssaPkgs {
		e.emitSSAPackage(pkg)
	}
	e.addPhase(timer, "emit_rows", workload)
	e.emitRTAEdges(ssaPkgs)
	e.addPhase(timer, "rta_analyze", workload)
	e.add(Row{
		"kind":              "session_end",
		"schema":            SchemaVersion,
		"row_count":         len(e.rows) + 1,
		"go_version":        runtime.Version(),
		"elapsed_ms":        timer.totalMillis(),
		"packages":          workload.packages,
		"compiled_go_files": workload.compiledGoFiles,
		"deps_with_types":   workload.depsWithTypes,
		"peak_heap_bytes":   timer.sampleHeap(),
	})
	return e.rows, nil
}

// addPhase closes a stage that ran after the emitter existed, so its row can
// report how many rows the stage had produced.
func (e *emitter) addPhase(timer *phaseTimer, phase string, workload phaseWorkload) {
	workload.rowsEmitted = len(e.rows)
	before := len(timer.rows)
	timer.finish(phase, workload)
	for _, row := range timer.rows[before:] {
		e.add(row)
	}
}

func (e *emitter) add(row Row) {
	if _, ok := row["schema"]; !ok {
		row["schema"] = SchemaVersion
	}
	e.rows = append(e.rows, row)
}

func (e *emitter) emitPackage(pkg *packages.Package) {
	modulePath := ""
	if pkg.Module != nil {
		modulePath = pkg.Module.Path
	}
	files := append([]string{}, pkg.CompiledGoFiles...)
	sort.Strings(files)
	for i, file := range files {
		files[i] = e.relative(file)
	}
	e.add(Row{
		"kind":         "package",
		"package_id":   pkg.ID,
		"package_path": pkg.PkgPath,
		"package_name": pkg.Name,
		"module_path":  modulePath,
		"test_variant": testVariant(pkg.ID),
		"files":        files,
		"stable_key":   stableKey("package", pkg.ID, pkg.PkgPath),
	})
}

func (e *emitter) emitPackageErrors(pkg *packages.Package) {
	for index, err := range pkg.Errors {
		e.add(Row{
			"kind":         "package_error",
			"package_id":   pkg.ID,
			"package_path": pkg.PkgPath,
			"message":      err.Msg,
			"stable_key":   stableKey("package_error", pkg.ID, strconv.Itoa(index), err.Msg),
		})
	}
}

func (e *emitter) emitSSAPackage(pkg *ssa.Package) {
	if pkg == nil || pkg.Pkg == nil {
		return
	}
	functions := ssaFunctions(pkg)
	for _, fn := range functions {
		e.emitFunction(pkg, fn)
		e.emitCallsites(pkg, fn)
		e.emitInstantiatedTypes(pkg, fn)
		e.emitAddressTaken(pkg, fn)
	}
	e.emitMethodSets(pkg)
	e.emitInstantiatedMethodSets(pkg)
}

func (e *emitter) emitRTAEdges(pkgs []*ssa.Package) {
	mainPkgs := make([]*ssa.Package, 0)
	for _, pkg := range pkgs {
		if pkg == nil || pkg.Pkg == nil || pkg.Pkg.Name() != "main" {
			continue
		}
		mainPkgs = append(mainPkgs, pkg)
	}
	sort.Slice(mainPkgs, func(i, j int) bool {
		return packageID(mainPkgs[i]) < packageID(mainPkgs[j])
	})

	for _, mainPkg := range mainPkgs {
		roots := make([]*ssa.Function, 0, 2)
		if mainFn := mainPkg.Func("main"); mainFn != nil {
			roots = append(roots, mainFn)
		}
		if initFn := mainPkg.Func("init"); initFn != nil {
			roots = append(roots, initFn)
		}
		if len(roots) == 0 {
			continue
		}
		// rta.Analyze can panic on incomplete SSA for indirect dependencies
		// (the NeedDeps load mode prevents the known reflect.Members["Value"]
		// case). Recover defensively so one package's failure degrades to
		// "no RTA edges for this package" instead of crashing the whole
		// frontend and losing every other fact.
		result := func() (res *rta.Result) {
			defer func() {
				if r := recover(); r != nil {
					fmt.Fprintf(os.Stderr,
						"polint-go-frontend: rta.Analyze recovered from panic for %s: %v\n",
						packagePath(mainPkg), r)
					res = nil
				}
			}()
			return rta.Analyze(roots, true)
		}()
		if result == nil || result.CallGraph == nil {
			continue
		}

		var rows []Row
		callgraph.GraphVisitEdges(result.CallGraph, func(edge *callgraph.Edge) error {
			if edge == nil || edge.Caller == nil || edge.Callee == nil || edge.Caller.Func == nil || edge.Callee.Func == nil {
				return nil
			}
			from := edge.Caller.Func.RelString(mainPkg.Pkg)
			to := edge.Callee.Func.RelString(mainPkg.Pkg)
			description := edge.Description()
			if from == "" || to == "" || description == "" {
				return nil
			}
			rows = append(rows, Row{
				"kind":         "rta_edge",
				"package_id":   packageID(mainPkg),
				"package_path": packagePath(mainPkg),
				"caller":       from,
				"callee":       to,
				"edge_kind":    description,
				"stable_key":   stableKey(packageID(mainPkg), "rta_edge", from, description, to),
			})
			return nil
		})
		sort.Slice(rows, func(i, j int) bool {
			return fmt.Sprint(rows[i]["stable_key"]) < fmt.Sprint(rows[j]["stable_key"])
		})
		for _, row := range rows {
			e.add(row)
		}
	}
}

func (e *emitter) emitFunction(pkg *ssa.Package, fn *ssa.Function) {
	if fn == nil {
		return
	}
	isInit := isInitFunctionName(fn.Name())
	isMethod := fn.Signature != nil && fn.Signature.Recv() != nil
	if fn.Synthetic != "" && !isInit && !isMethod && !fn.Pos().IsValid() {
		e.add(Row{
			"kind":         "unsupported",
			"package_id":   packageID(pkg),
			"package_path": packagePath(pkg),
			"name":         fn.String(),
			"reason":       "synthetic function without stable source identity",
		})
		return
	}
	// Keep-first per (package_id, fn.String()) so a method VALUE reachable via two harvest
	// paths (the ssaFunctions walk AND emitInstantiatedMethodSets) emits exactly ONE
	// function/method row — never a duplicate stable_key that fails validate_unique and
	// zeroes the whole Go fact set (review #4). Gating here also suppresses the duplicate's
	// sibling receiver_type row (its key would likewise collide). A package without the
	// cross-path collision never repeats a key, so this is a no-op and rows stay
	// byte-identical. The `unsupported`-synthetic early return above is intentionally
	// ungated (no stable_key, distinct kind, not a validate_unique family).
	functionKey := stableKey(packageID(pkg), fn.String())
	if e.emittedFunctionKeys[functionKey] {
		return
	}
	e.emittedFunctionKeys[functionKey] = true
	kind := "function"
	if isInit {
		kind = "init_function"
	} else if isMethod {
		kind = "method"
	}
	name := fn.Name()
	if isInit {
		name = "init"
	} else if isMethod && fn.Signature != nil && fn.Signature.Recv() != nil {
		// For a method on an INSTANTIATED generic type, the SSA method value's `Name()`
		// can carry a type-argument suffix (a pointer-receiver instantiation wrapper is
		// named `Inc[int]`, while a value-receiver promotion is named `Inc`). The core
		// (tree-sitter) facts name the method by its bare identifier with the generics
		// stripped from the receiver (`Box.Inc`), so strip the type-arg suffix here too
		// or the SSA↔core join (`matching_core_function`, file+name+span) would miss the
		// instantiated method's node and the generic-dispatch edge would be lost
		// (FINDING A). A non-generic method name has no `[...]` suffix and is unchanged.
		if receiver := receiverTypeName(fn.Signature.Recv().Type().String()); receiver != "" {
			name = receiver + "." + stripMethodTypeArgs(fn.Name())
		}
	}
	row := Row{
		"kind":         kind,
		"package_id":   packageID(pkg),
		"package_path": packagePath(pkg),
		"name":         name,
		"qualified":    fn.String(),
		"signature":    signatureString(fn.Signature),
		"stable_key":   functionKey,
	}
	if syntax := fn.Syntax(); syntax != nil {
		if pos := e.positionSpan(syntax.Pos(), syntax.End()); pos != nil {
			row["file"] = posFile(e.fset, syntax.Pos(), e.root)
			row["span"] = pos
		}
	} else if pos := e.positionSpan(fn.Pos(), fn.Pos()); pos != nil {
		row["file"] = posFile(e.fset, fn.Pos(), e.root)
		row["span"] = pos
	}
	if fn.Signature != nil && fn.Signature.Recv() != nil {
		row["receiver"] = fn.Signature.Recv().Type().String()
		e.add(Row{
			"kind":         "receiver_type",
			"package_id":   packageID(pkg),
			"package_path": packagePath(pkg),
			"method":       fn.String(),
			"receiver":     fn.Signature.Recv().Type().String(),
			"stable_key":   stableKey(packageID(pkg), "recv", fn.String(), fn.Signature.Recv().Type().String()),
		})
	}
	e.add(row)
}

func (e *emitter) emitCallsites(pkg *ssa.Package, fn *ssa.Function) {
	if fn == nil {
		return
	}
	for _, block := range fn.Blocks {
		for _, instr := range block.Instrs {
			call, ok := instr.(ssa.CallInstruction)
			if !ok {
				continue
			}
			common := call.Common()
			row := Row{
				"kind":         "callsite",
				"package_id":   packageID(pkg),
				"package_path": packagePath(pkg),
				"caller":       fn.String(),
			}
			dynamic := false
			switch {
			case common != nil && common.StaticCallee() != nil:
				row["static_callee"] = common.StaticCallee().String()
				row["status"] = "resolved_static"
			case common != nil && isBuiltinCall(common):
				// FINDING 4: a builtin call (`len`, `append`, `recover`, ...) has a nil
				// StaticCallee() because its callee is a *ssa.Builtin, not an
				// *ssa.Function. It is NOT a func-value dynamic dispatch — treat it as
				// unsupported (no ssa.Function identity to resolve to) and emit NO
				// dynamic_dispatch row, so the RTA driver never sees a bogus func-value
				// obligation for a builtin.
				row["status"] = "unsupported"
				row["reason"] = "builtin call (no ssa.Function callee)"
			default:
				row["status"] = "unresolved_dynamic"
				row["reason"] = "interface or func-value dynamic dispatch"
				dynamic = true
			}
			stableParts := []string{packageID(pkg), fn.String(), fmt.Sprint(call.Pos())}
			if syntax := callSyntax(fn, call); syntax != nil {
				if pos := e.positionSpan(syntax.Pos(), syntax.End()); pos != nil {
					file := posFile(e.fset, syntax.Pos(), e.root)
					row["file"] = file
					row["span"] = pos
					stableParts = []string{
						packageID(pkg),
						fn.String(),
						file,
						strconv.Itoa(pos.StartByte),
						strconv.Itoa(pos.EndByte),
					}
				}
			} else if pos := e.positionSpan(call.Pos(), call.Pos()); pos != nil {
				row["file"] = posFile(e.fset, call.Pos(), e.root)
				row["span"] = pos
			}
			callsiteKey := stableKey(stableParts...)
			row["stable_key"] = callsiteKey
			e.add(row)
			if dynamic {
				e.emitDynamicDispatch(pkg, fn, common, callsiteKey)
			}
		}
	}
}

// emitDynamicDispatch emits the dispatch discriminant Plan 2's RTA driver needs to
// resolve an UnresolvedDynamic callsite by method-set matching (D-05). For an interface
// invoke it carries the interface type + invoked method name; for a func-value call it
// carries the called value's signature. The row joins back to its sibling callsite row
// via callsite_stable_key. Honest representation (D-15): if no discriminant can
// be derived, no dispatch-detail row is emitted rather than a fabricated identity.
func (e *emitter) emitDynamicDispatch(pkg *ssa.Package, fn *ssa.Function, common *ssa.CallCommon, callsiteKey string) {
	if common == nil {
		return
	}
	row := Row{
		"kind":                "dynamic_dispatch",
		"package_id":          packageID(pkg),
		"package_path":        packagePath(pkg),
		"caller":              fn.String(),
		"callsite_stable_key": callsiteKey,
	}
	var discriminant string
	if common.IsInvoke() && common.Method != nil {
		interfaceType := ""
		if common.Value != nil && common.Value.Type() != nil {
			interfaceType = common.Value.Type().String()
		}
		method := common.Method.Name()
		row["interface_type"] = interfaceType
		row["method"] = method
		discriminant = "invoke:" + interfaceType + ":" + method
	} else {
		signature := ""
		if sig := common.Signature(); sig != nil {
			signature = sig.String()
		}
		if signature == "" {
			return
		}
		row["signature"] = signature
		discriminant = "func_value:" + signature
	}
	row["stable_key"] = stableKey(packageID(pkg), "dynamic_dispatch", callsiteKey, discriminant)
	e.add(row)
}

// emitInstantiatedTypes harvests the RTA "rapid type" set: each concrete type converted
// to an interface via *ssa.MakeInterface in the reachable SSA program (D-05). x/tools RTA
// adds a type to the rapid-type set precisely when it is converted to an interface, so
// MakeInterface is the faithful and sufficient source. We deliberately do NOT harvest the
// *ssa.Alloc / *ssa.MakeMap / *ssa.MakeSlice / *ssa.MakeChan families: allocating a value
// does not by itself make its type dynamically dispatchable under RTA — only an
// interface conversion does — so adding them would over-approximate the rapid-type set and
// flood precision without lifting recall. Types lacking a stable .String() identity emit an
// "unsupported" row rather than a fabricated identity (D-15). De-duplicated within
// a function by the concrete type identity.
func (e *emitter) emitInstantiatedTypes(pkg *ssa.Package, fn *ssa.Function) {
	if fn == nil {
		return
	}
	seen := make(map[string]bool)
	for _, block := range fn.Blocks {
		for _, instr := range block.Instrs {
			mi, ok := instr.(*ssa.MakeInterface)
			if !ok {
				continue
			}
			if mi.X == nil || mi.X.Type() == nil {
				e.add(Row{
					"kind":         "unsupported",
					"package_id":   packageID(pkg),
					"package_path": packagePath(pkg),
					"name":         fn.String(),
					"reason":       "MakeInterface operand without stable type identity",
				})
				continue
			}
			// Canonicalize through any type alias to the underlying type (FIX 3) so a
			// value of `type AliasDog = Dog` is harvested as `...Dog` — matching the
			// concrete method's receiver — not the alias spelling `...AliasDog`.
			concrete := canonicalTypeString(mi.X.Type())
			if concrete == "" || seen[concrete] {
				continue
			}
			seen[concrete] = true
			e.add(Row{
				"kind":         "instantiated_type",
				"package_id":   packageID(pkg),
				"package_path": packagePath(pkg),
				"type":         concrete,
				"stable_key":   stableKey(packageID(pkg), "instantiated_type", concrete),
			})
		}
	}
}

// emitAddressTaken harvests the set of functions whose address is taken in the reachable
// SSA program (D-05) — an RTA dispatch input for func-value callsites. Sources: *ssa.MakeClosure
// over an *ssa.Function (closures and bound method values), and any *ssa.Function used as a
// GENUINE value operand of an instruction (function references passed as args, stored to
// globals, returned, or assigned). De-duplicated within a function by the function identity;
// builtins/synthetic functions without stable identity are skipped rather than fabricated
// (D-15).
//
// FINDING 2: a STATICALLY-called function is NOT address-taken. For a *ssa.Call / Go / Defer
// the callee is the call's `common.Value` operand, and when that is the static callee it
// appears in `instr.Operands(...)` — naively harvesting it would mark every statically-called
// function (every `helper()`, `defer cleanup()`, `go worker()`) address-taken and flood the
// func-value RTA candidate set. So for a call instruction we EXCLUDE the operand that equals
// the static-callee value; a func VALUE used in a `go`/`defer`/call (no static callee) is a
// genuine value use and is still captured by the generic operand scan.
func (e *emitter) emitAddressTaken(pkg *ssa.Package, fn *ssa.Function) {
	if fn == nil {
		return
	}
	seen := make(map[string]bool)
	emit := func(target *ssa.Function) {
		if target == nil {
			return
		}
		identity := target.String()
		if identity == "" || seen[identity] {
			return
		}
		seen[identity] = true
		e.add(Row{
			"kind":         "address_taken",
			"package_id":   packageID(pkg),
			"package_path": packagePath(pkg),
			"function":     identity,
			"stable_key":   stableKey(packageID(pkg), "address_taken", identity),
		})
	}
	for _, block := range fn.Blocks {
		for _, instr := range block.Instrs {
			if closure, ok := instr.(*ssa.MakeClosure); ok {
				if target, ok := closure.Fn.(*ssa.Function); ok {
					emit(target)
				}
			}
			// The static-callee value of a call/go/defer is NOT a value use: skip it so a
			// statically-called function is not marked address-taken (FINDING 2). A
			// dynamic dispatch has no static callee, so this excludes nothing there.
			var staticCallee ssa.Value
			if call, ok := instr.(ssa.CallInstruction); ok {
				if common := call.Common(); common != nil && common.StaticCallee() != nil {
					staticCallee = common.Value
				}
			}
			operands := instr.Operands(nil)
			for _, operand := range operands {
				if operand == nil || *operand == nil {
					continue
				}
				if staticCallee != nil && *operand == staticCallee {
					continue
				}
				if target, ok := (*operand).(*ssa.Function); ok {
					emit(target)
				}
			}
		}
	}
}

// isBuiltinCall reports whether a call's callee is a Go builtin (`len`, `append`,
// `recover`, `make`, ...). In go/ssa a builtin call has a non-interface CallCommon whose
// Value is a *ssa.Builtin and whose StaticCallee() is nil (a builtin has no
// *ssa.Function). It is NOT a func-value dynamic dispatch (FINDING 4).
func isBuiltinCall(common *ssa.CallCommon) bool {
	if common == nil || common.IsInvoke() {
		return false
	}
	_, ok := common.Value.(*ssa.Builtin)
	return ok
}

func callSyntax(fn *ssa.Function, call ssa.CallInstruction) ast.Node {
	if fn == nil || !call.Pos().IsValid() {
		return nil
	}
	syntax := fn.Syntax()
	if syntax == nil {
		return nil
	}
	pos := call.Pos()
	var best ast.Node
	ast.Inspect(syntax, func(node ast.Node) bool {
		if node == nil {
			return true
		}
		expr, ok := node.(*ast.CallExpr)
		if !ok {
			return true
		}
		if expr.Pos() <= pos && pos < expr.End() {
			if best == nil || expr.End()-expr.Pos() < best.End()-best.Pos() {
				best = expr
			}
		}
		return true
	})
	return best
}

// addMethodSet emits a single `method_set` row for `identity` (already the canonical,
// alias-resolved type string) carrying `methods`, coordinating the TWO method_set emitters
// so AT MOST ONE row survives per canonical stable_key across BOTH (review #4). The two
// emitters compute the IDENTICAL stable_key for the same canonical identity, so a
// SAME-PACKAGE alias to a generic instantiation (`type IntBox = Box[int]`) would otherwise
// emit two rows with the same key — a hard `validate_unique` conflict that zeroes the whole
// Go fact set. Keep-first: if the key was already emitted (by either emitter) the row is
// SKIPPED. This gates ONLY the method_set row; emitInstantiatedMethodSets still emits its
// concrete method VALUE rows regardless. A package without such a collision never repeats a
// key, so this is a no-op there and the emitted rows are byte-identical to before.
func (e *emitter) addMethodSet(pkg *ssa.Package, identity string, methods []string) {
	if identity == "" {
		return
	}
	key := stableKey(packageID(pkg), "method_set", identity)
	if e.emittedMethodSetKeys[key] {
		return
	}
	e.emittedMethodSetKeys[key] = true
	e.add(Row{
		"kind":         "method_set",
		"package_id":   packageID(pkg),
		"package_path": packagePath(pkg),
		"type":         identity,
		"methods":      methods,
		"stable_key":   key,
	})
}

func (e *emitter) emitMethodSets(pkg *ssa.Package) {
	scope := pkg.Pkg.Scope()
	// De-duplicate by the CANONICAL (alias-resolved) identity (FIX 3). Package scope can
	// hold both a defined type `Dog` and a type alias `type AliasDog = Dog`; both resolve
	// through `types.Unalias` to the same underlying `...Dog`, and a method_set is keyed by
	// unique declaration identity (the store does NOT treat method_sets as set facts — it
	// REJECTS a duplicate stable key). Emitting the same canonical identity twice would be a
	// hard validation conflict that zeroes the whole Go fact set, so keep the FIRST
	// occurrence (its method-set is identical — go/types resolves both through the alias). A
	// package with no aliases has a unique canonical identity per type, so this is a no-op
	// and byte-identity is preserved. The keep-first guard now lives in `addMethodSet`'s
	// emitter-scoped set, which ALSO coordinates with emitInstantiatedMethodSets so an alias
	// to a generic INSTANTIATION (`type IntBox = Box[int]`) cannot collide with the
	// instantiated harvest (review #4).
	for _, name := range scope.Names() {
		obj := scope.Lookup(name)
		typeName, ok := obj.(*types.TypeName)
		if !ok {
			continue
		}
		methodSet := types.NewMethodSet(types.NewPointer(typeName.Type()))
		if methodSet.Len() == 0 {
			continue
		}
		// Key by the underlying identity so an alias and its target collapse to one row.
		identity := canonicalTypeString(typeName.Type())
		if identity == "" {
			continue
		}
		var methods []string
		for i := 0; i < methodSet.Len(); i++ {
			// The method-set carries the bare method NAME (the identifier, e.g.
			// "Speak"), not the full signature string. RTA interface-invoke
			// resolution intersects the INVOKED method name (the dynamic-dispatch
			// discriminant) with this set, so a signature string would never match
			// the invoked name and interface dispatch would resolve nothing. Verification
			// surfaced this. `Obj().Name()` is the method identifier.
			methods = append(methods, methodSet.At(i).Obj().Name())
		}
		sort.Strings(methods)
		// `identity` is the canonical (alias-resolved) type string computed above, so the
		// method_set, the canonicalized instantiated_type, and the concrete method's
		// receiver all share one identity and the RTA join succeeds. addMethodSet keeps the
		// FIRST row per canonical stable_key across BOTH method_set emitters.
		e.addMethodSet(pkg, identity, methods)
	}
}

// emitInstantiatedMethodSets harvests method-sets AND concrete method values keyed by the
// INSTANTIATED `*types.Named` identity of each generic type instantiation reachable in the
// program (FINDING A). x/tools RTA records the instantiated named type (e.g. `Box[int]`) in
// the program's runtime-type set — `*ssa.MakeInterface` of a `Box[int]` value adds exactly
// `Box[int]` to the rapid-type set, and `emitInstantiatedTypes` emits that identity — but
// `emitMethodSets` walks package-scope `*types.TypeName`s, which for a generic type is the
// GENERIC declaration (`Box[T any]`), whose `*ssa.Type` member has a nil method VALUE. So
// the method-set the Rust dispatch resolver looks up by the instantiated identity
// (`method_sets.get("Box[int]")`) was absent and the interface invoke lost its edge.
//
// For each instantiated named type declared in THIS package, this emits (a) a `method_set`
// row keyed by the instantiated type's `.String()` (`Box[int]`) carrying its method names,
// and (b) the concrete method VALUES as `method` rows (via `emitFunction`) so the resolved
// edge has a target node. Both use the POINTER method set (mirroring `emitMethodSets`), so
// each method appears exactly once (no value/pointer-wrapper duplication). Methods without a
// stable source identity are skipped rather than fabricated (D-15). De-duplicated
// by the instantiated type identity so a type instantiated in two functions is harvested
// once.
func (e *emitter) emitInstantiatedMethodSets(pkg *ssa.Package) {
	prog := pkg.Prog
	if prog == nil {
		return
	}
	seen := make(map[string]bool)
	// RuntimeTypes() order is not specified; sort the harvested identities so the emitted
	// rows are deterministic regardless of x/tools' internal iteration order.
	type instantiation struct {
		key   string
		named *types.Named
	}
	var instantiations []instantiation
	for _, runtimeType := range prog.RuntimeTypes() {
		named, ok := runtimeType.(*types.Named)
		if !ok {
			continue
		}
		// Only generic INSTANTIATIONS (type args present), declared in THIS package, are
		// harvested here. A non-generic named type is already covered by emitMethodSets.
		if named.TypeArgs() == nil || named.TypeArgs().Len() == 0 {
			continue
		}
		if named.Obj() == nil || named.Obj().Pkg() == nil || named.Obj().Pkg().Path() != pkg.Pkg.Path() {
			continue
		}
		key := named.String()
		if key == "" || seen[key] {
			continue
		}
		seen[key] = true
		instantiations = append(instantiations, instantiation{key: key, named: named})
	}
	sort.Slice(instantiations, func(i, j int) bool { return instantiations[i].key < instantiations[j].key })

	for _, inst := range instantiations {
		methodSet := prog.MethodSets.MethodSet(types.NewPointer(inst.named))
		if methodSet.Len() == 0 {
			continue
		}
		var methods []string
		for i := 0; i < methodSet.Len(); i++ {
			// The bare method identifier (`sel.Obj().Name()` is always the clean name,
			// e.g. "Speak", never an instantiation-suffixed "Speak[int]"). RTA
			// interface-invoke resolution intersects the invoked method name with this
			// set (see emitMethodSets), so the bare name is required.
			methods = append(methods, methodSet.At(i).Obj().Name())
			// Emit the concrete method VALUE so the resolved edge has a target node. A
			// promotion/instantiation wrapper has a valid source position pointing back
			// at the generic declaration, so emitFunction can join it to the core node.
			// This is emitted UNCONDITIONALLY (before the method_set gate below) so a
			// skipped duplicate method_set row never suppresses the method value — the gate
			// covers ONLY the method_set row (review #4).
			if fn := prog.MethodValue(methodSet.At(i)); fn != nil {
				e.emitFunction(pkg, fn)
			}
		}
		sort.Strings(methods)
		// addMethodSet keeps the FIRST method_set row per canonical stable_key across BOTH
		// emitters. For a SAME-PACKAGE alias to this instantiation (`type IntBox = Box[int]`),
		// emitMethodSets already emitted the IDENTICAL canonical `...Box[int]` key (its method
		// list is identical), so this row is skipped keep-first — never a duplicate that
		// zeroes the whole Go fact set. `inst.named.String()` is the canonical instantiated
		// identity (no alias to resolve).
		e.addMethodSet(pkg, inst.named.String(), methods)
	}
}

// stripMethodTypeArgs removes a trailing type-argument suffix (`[...]`) from an SSA method
// identifier so an instantiated generic method's bare name matches the core (tree-sitter)
// identity (FINDING A). The SSA value `Name()` of a pointer-receiver instantiation wrapper
// is `Inc[int]`; the bare identifier is `Inc`. A Go method identifier cannot otherwise
// contain `[`, so a name with no trailing `[...]` is returned unchanged.
func stripMethodTypeArgs(name string) string {
	if !strings.HasSuffix(name, "]") {
		return name
	}
	if index := strings.IndexByte(name, '['); index >= 0 {
		return name[:index]
	}
	return name
}

func ssaFunctions(pkg *ssa.Package) []*ssa.Function {
	var functions []*ssa.Function
	// Dedup by *ssa.Function identity so a function reachable via two roots (e.g. a
	// method value that is also harvested as a top-level member) is walked once.
	seen := make(map[*ssa.Function]bool)
	for _, member := range pkg.Members {
		switch value := member.(type) {
		case *ssa.Function:
			collectWithAnon(value, seen, &functions)
		case *ssa.Type:
			methodSet := pkg.Prog.MethodSets.MethodSet(types.NewPointer(value.Type()))
			for i := 0; i < methodSet.Len(); i++ {
				if fn := pkg.Prog.MethodValue(methodSet.At(i)); fn != nil {
					collectWithAnon(fn, seen, &functions)
				}
			}
		}
	}
	sort.Slice(functions, func(i, j int) bool { return functions[i].String() < functions[j].String() })
	return functions
}

// collectWithAnon appends fn and, transitively, its anonymous-function bodies
// (closures, func(){...} literals, bound method-value thunks) to out. In go/ssa
// these live in parent.AnonFuncs and are NOT among pkg.Members, so without this walk
// a *ssa.MakeInterface, a dynamic callsite, or a function-value operand that appears
// inside a closure body would be invisible to the RTA harvest (review WR-01). Closures
// nest, so the walk is transitive; `seen` guards against double-visiting.
func collectWithAnon(fn *ssa.Function, seen map[*ssa.Function]bool, out *[]*ssa.Function) {
	if fn == nil || seen[fn] {
		return
	}
	seen[fn] = true
	*out = append(*out, fn)
	for _, anon := range fn.AnonFuncs {
		collectWithAnon(anon, seen, out)
	}
}

func packageID(pkg *ssa.Package) string {
	if pkg == nil || pkg.Pkg == nil {
		return ""
	}
	return pkg.Pkg.Path()
}

func packagePath(pkg *ssa.Package) string {
	return packageID(pkg)
}

func signatureString(sig *types.Signature) string {
	if sig == nil {
		return ""
	}
	return sig.String()
}

// canonicalTypeString returns the type's `.String()` resolved THROUGH any type alias to
// its underlying type (FIX 3). go/types reports a value of a type alias (`type AliasDog =
// Dog`) under the alias spelling (`...AliasDog`) for both the MakeInterface operand type
// and the alias's package-scope TypeName, but the concrete method's receiver is the
// UNDERLYING `...Dog`. Keying the instantiated_type and method_set under the alias spelling
// makes the Rust resolver's `methods_by_receiver["...AliasDog"]` lookup miss and the
// interface-dispatch edge is silently dropped. `types.Unalias` collapses the alias chain to
// the underlying type so instantiated_type, method_set key, and the method receiver all
// share one canonical identity and the join succeeds. It is a NO-OP on a non-alias type, so
// every non-alias identity's `.String()` is unchanged (byte-identity preserved).
func canonicalTypeString(t types.Type) string {
	if t == nil {
		return ""
	}
	return types.Unalias(t).String()
}

func receiverTypeName(receiver string) string {
	receiver = strings.TrimSpace(receiver)
	receiver = strings.TrimLeft(receiver, "*&")
	if receiver == "" {
		return ""
	}
	receiver = receiver[strings.LastIndex(receiver, ".")+1:]
	if index := strings.Index(receiver, "["); index >= 0 {
		receiver = receiver[:index]
	}
	return receiver
}

func isInitFunctionName(name string) bool {
	return name == "init" || strings.HasPrefix(name, "init#") || strings.HasPrefix(name, "init$")
}

func stableKey(parts ...string) string {
	var builder strings.Builder
	for _, part := range parts {
		builder.WriteString(strconv.Itoa(len(part)))
		builder.WriteString(":")
		builder.WriteString(part)
		builder.WriteString(";")
	}
	return builder.String()
}

func (e *emitter) positionSpan(start token.Pos, end token.Pos) *Span {
	if !start.IsValid() {
		return nil
	}
	startPos := e.fset.Position(start)
	endPos := e.fset.Position(end)
	return &Span{
		StartByte:   startPos.Offset,
		EndByte:     endPos.Offset,
		StartLine:   startPos.Line,
		StartColumn: startPos.Column,
		EndLine:     endPos.Line,
		EndColumn:   endPos.Column,
	}
}

func posFile(fset *token.FileSet, pos token.Pos, root string) string {
	position := fset.Position(pos)
	if position.Filename == "" {
		return ""
	}
	if rel, err := filepath.Rel(root, position.Filename); err == nil {
		return filepath.ToSlash(rel)
	}
	return filepath.ToSlash(position.Filename)
}

func (e *emitter) relative(path string) string {
	if rel, err := filepath.Rel(e.root, path); err == nil {
		return filepath.ToSlash(rel)
	}
	return filepath.ToSlash(path)
}

func testVariant(id string) string {
	switch {
	case strings.HasSuffix(id, ".test"):
		return "test_binary"
	case strings.Contains(id, " ["):
		return "external_test"
	default:
		return "regular"
	}
}

func goBuildFlags(buildTags []string) []string {
	flags := []string{"-mod=readonly"}
	if len(buildTags) > 0 {
		flags = append(flags, "-tags="+strings.Join(buildTags, ","))
	}
	return flags
}

func validatePackagePatterns(patterns []string) ([]string, error) {
	var normalized []string
	for _, raw := range patterns {
		pattern := strings.TrimSpace(raw)
		if pattern == "" {
			continue
		}
		if strings.HasPrefix(pattern, "-") {
			return nil, fmt.Errorf("package pattern %q must not start with -", pattern)
		}
		normalized = append(normalized, pattern)
	}
	if len(normalized) == 0 {
		return []string{"./..."}, nil
	}
	return normalized, nil
}

func validateModuleRootFiles(root string, moduleRoots []string) error {
	for _, moduleRoot := range moduleRoots {
		if !repoRegularFileExists(root, moduleRootManifest(moduleRoot)) {
			return fmt.Errorf("module root %q is missing an in-repository go.mod", moduleRoot)
		}
	}
	return nil
}

func moduleRootManifest(moduleRoot string) string {
	if moduleRoot == "." {
		return "go.mod"
	}
	return filepath.ToSlash(filepath.Join(filepath.FromSlash(moduleRoot), "go.mod"))
}

func normalizeModuleRoots(roots []string) ([]string, error) {
	if len(roots) == 0 {
		roots = []string{"."}
	}
	seen := make(map[string]bool)
	var normalized []string
	for _, raw := range roots {
		root := strings.TrimSpace(raw)
		if root == "" || root == "." {
			root = "."
		}
		if filepath.IsAbs(root) {
			return nil, fmt.Errorf("module root %q must be relative to repository root", raw)
		}
		clean := filepath.Clean(filepath.FromSlash(root))
		if clean == "." {
			root = "."
		} else {
			if clean == ".." || strings.HasPrefix(clean, ".."+string(filepath.Separator)) {
				return nil, fmt.Errorf("module root %q escapes repository root", raw)
			}
			root = filepath.ToSlash(clean)
		}
		if !seen[root] {
			normalized = append(normalized, root)
			seen[root] = true
		}
	}
	return normalized, nil
}

func rootedPackagePatterns(moduleRoots []string, patterns []string) []string {
	seen := make(map[string]bool)
	var rooted []string
	for _, root := range moduleRoots {
		for _, pattern := range patterns {
			rootedPattern := rootedPackagePattern(root, pattern)
			if !seen[rootedPattern] {
				rooted = append(rooted, rootedPattern)
				seen[rootedPattern] = true
			}
		}
	}
	return rooted
}

func rootedPackagePattern(root string, pattern string) string {
	if root == "." {
		return pattern
	}
	base := "./" + strings.TrimPrefix(root, "./")
	if pattern == "." {
		return base
	}
	if strings.HasPrefix(pattern, "./") {
		suffix := strings.TrimPrefix(pattern, "./")
		if suffix == "" {
			return base
		}
		return base + "/" + suffix
	}
	return pattern
}

func goPackageEnv(root string, moduleRoots []string) ([]string, func(), error) {
	env := os.Environ()
	workspace, cleanup, err := workspaceEnv(root, moduleRoots)
	if err != nil {
		return nil, nil, err
	}
	env = setEnv(env, "GOWORK", workspace)
	return env, cleanup, nil
}

func setEnv(env []string, key string, value string) []string {
	prefix := key + "="
	filtered := make([]string, 0, len(env)+1)
	for _, entry := range env {
		if !strings.HasPrefix(entry, prefix) {
			filtered = append(filtered, entry)
		}
	}
	return append(filtered, prefix+value)
}

func workspaceEnv(root string, moduleRoots []string) (string, func(), error) {
	checkedIn := filepath.Join(root, "go.work")
	if repoRegularFileExists(root, "go.work") && goWorkCoversModuleRoots(root, checkedIn, moduleRoots) {
		return checkedIn, func() {}, nil
	}
	if !needsSyntheticWorkspace(moduleRoots) {
		return "off", func() {}, nil
	}
	return writeSyntheticGoWork(root, moduleRoots)
}

func needsSyntheticWorkspace(moduleRoots []string) bool {
	return len(moduleRoots) != 1 || moduleRoots[0] != "."
}

func goWorkCoversModuleRoots(root string, workPath string, moduleRoots []string) bool {
	contents, err := readFileWithLimit(workPath, topologyManifestMaxBytes)
	if err != nil {
		return false
	}
	work, err := modfile.ParseWork(workPath, contents, nil)
	if err != nil {
		return false
	}
	rootReal, err := realCleanPath(root)
	if err != nil {
		return false
	}
	covered := make(map[string]bool)
	for _, use := range work.Use {
		usePath := filepath.Clean(filepath.FromSlash(use.Path))
		if !filepath.IsAbs(usePath) {
			usePath = filepath.Join(root, usePath)
		}
		realUse, err := realCleanPath(usePath)
		if err != nil || !isUnderRoot(rootReal, realUse) {
			continue
		}
		covered[realUse] = true
	}
	for _, moduleRoot := range moduleRoots {
		modulePath := filepath.Join(root, filepath.FromSlash(moduleRoot))
		realModule, err := realCleanPath(modulePath)
		if err != nil || !covered[realModule] {
			return false
		}
	}
	return true
}

func writeSyntheticGoWork(root string, moduleRoots []string) (string, func(), error) {
	dir, err := os.MkdirTemp("", "polint-go-work-")
	if err != nil {
		return "", nil, fmt.Errorf("create temporary go.work directory: %w", err)
	}
	cleanup := func() { _ = os.RemoveAll(dir) }
	workPath := filepath.Join(dir, "go.work")
	var contents strings.Builder
	contents.WriteString("go ")
	contents.WriteString(syntheticGoWorkVersion(root, moduleRoots))
	contents.WriteString("\n\nuse (\n")
	for _, moduleRoot := range moduleRoots {
		modulePath := filepath.Join(root, filepath.FromSlash(moduleRoot))
		contents.WriteString("\t")
		contents.WriteString(strconv.Quote(filepath.ToSlash(goWorkUsePath(dir, modulePath))))
		contents.WriteString("\n")
	}
	contents.WriteString(")\n")
	if err := os.WriteFile(workPath, []byte(contents.String()), 0o600); err != nil {
		cleanup()
		return "", nil, fmt.Errorf("write temporary go.work: %w", err)
	}
	return workPath, cleanup, nil
}

func syntheticGoWorkVersion(root string, moduleRoots []string) string {
	version := "1.24"
	for _, moduleRoot := range moduleRoots {
		modPath := filepath.Join(root, filepath.FromSlash(moduleRoot), "go.mod")
		contents, err := readFileWithLimit(modPath, topologyManifestMaxBytes)
		if err != nil {
			continue
		}
		parsed, err := modfile.Parse(modPath, contents, nil)
		if err != nil || parsed.Go == nil {
			continue
		}
		if compareGoVersion(parsed.Go.Version, version) > 0 {
			version = parsed.Go.Version
		}
	}
	return version
}

func goWorkUsePath(workDir string, modulePath string) string {
	rel, err := filepath.Rel(workDir, modulePath)
	if err != nil || rel == "" {
		return modulePath
	}
	return rel
}

func compareGoVersion(left string, right string) int {
	leftParts := versionInts(left)
	rightParts := versionInts(right)
	for i := 0; i < len(leftParts) || i < len(rightParts); i++ {
		var l, r int
		if i < len(leftParts) {
			l = leftParts[i]
		}
		if i < len(rightParts) {
			r = rightParts[i]
		}
		if l < r {
			return -1
		}
		if l > r {
			return 1
		}
	}
	return 0
}

func versionInts(version string) []int {
	var values []int
	for _, part := range strings.Split(strings.TrimPrefix(version, "go"), ".") {
		value, err := strconv.Atoi(part)
		if err != nil {
			values = append(values, 0)
			continue
		}
		values = append(values, value)
	}
	return values
}

func readFileWithLimit(path string, maxBytes int64) ([]byte, error) {
	file, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer file.Close()
	contents, err := io.ReadAll(io.LimitReader(file, maxBytes+1))
	if err != nil {
		return nil, err
	}
	if int64(len(contents)) > maxBytes {
		return nil, fmt.Errorf("%s exceeds %d bytes", path, maxBytes)
	}
	return contents, nil
}

func realCleanPath(path string) (string, error) {
	abs, err := filepath.Abs(path)
	if err != nil {
		return "", err
	}
	real, err := filepath.EvalSymlinks(abs)
	if err != nil {
		return "", err
	}
	return filepath.Clean(real), nil
}

func isUnderRoot(root string, path string) bool {
	rel, err := filepath.Rel(root, path)
	if err != nil {
		return false
	}
	return rel == "." || (rel != ".." && !strings.HasPrefix(filepath.ToSlash(rel), "../"))
}

func repoRegularFileExists(root string, relative string) bool {
	path := filepath.Join(root, filepath.FromSlash(relative))
	rel, err := filepath.Rel(root, path)
	if err != nil || strings.HasPrefix(filepath.ToSlash(rel), "../") {
		return false
	}
	info, err := os.Stat(path)
	return err == nil && info.Mode().IsRegular()
}
