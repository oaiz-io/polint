package semantic

import (
	"go/ast"
	"go/token"
	"go/types"
	"sort"
	"strconv"

	"golang.org/x/tools/go/callgraph"
	"golang.org/x/tools/go/callgraph/vta"
	"golang.org/x/tools/go/packages"
	"golang.org/x/tools/go/ssa"
	"golang.org/x/tools/go/ssa/ssautil"
	"golang.org/x/tools/go/types/typeutil"
)

// callIndex finds the call expression an SSA call instruction came from.
//
// It is built once per function body. Looking each call up with a fresh walk of
// the body made emission quadratic in the size of large functions.
type callIndex struct {
	// calls is every call expression in the body, by start ascending and, for
	// expressions that start together, by end descending.
	calls []*ast.CallExpr
	// statements maps the position of a `go` or `defer` keyword to the call the
	// statement starts. SSA reports a Go or Defer instruction at its keyword,
	// which lies outside the call expression.
	statements map[token.Pos]*ast.CallExpr
}

func newCallIndex(syntax ast.Node) *callIndex {
	index := &callIndex{statements: make(map[token.Pos]*ast.CallExpr)}
	if syntax == nil {
		return index
	}
	ast.Inspect(syntax, func(node ast.Node) bool {
		switch node := node.(type) {
		case *ast.CallExpr:
			index.calls = append(index.calls, node)
		case *ast.GoStmt:
			index.statements[node.Go] = node.Call
		case *ast.DeferStmt:
			index.statements[node.Defer] = node.Call
		}
		return true
	})
	sort.SliceStable(index.calls, func(i, j int) bool {
		if index.calls[i].Pos() != index.calls[j].Pos() {
			return index.calls[i].Pos() < index.calls[j].Pos()
		}
		return index.calls[i].End() > index.calls[j].End()
	})
	return index
}

// syntaxFor returns the call expression of `call`: the statement's call for a
// `go` or `defer`, otherwise the innermost call expression containing the
// instruction's position, or nil.
func (index *callIndex) syntaxFor(call ssa.CallInstruction) ast.Node {
	pos := call.Pos()
	if !pos.IsValid() {
		return nil
	}
	switch call.(type) {
	case *ssa.Go, *ssa.Defer:
		if expr, ok := index.statements[pos]; ok {
			return expr
		}
	}
	// Containing expressions nest, so among those that start at or before pos
	// and still contain it, the one that starts last is the innermost.
	last := sort.Search(len(index.calls), func(i int) bool { return index.calls[i].Pos() > pos }) - 1
	for i := last; i >= 0; i-- {
		if pos < index.calls[i].End() {
			return index.calls[i]
		}
	}
	return nil
}

// callMode names how an instruction starts its callee.
func callMode(call ssa.CallInstruction) string {
	switch call.(type) {
	case *ssa.Go:
		return "go"
	case *ssa.Defer:
		return "defer"
	}
	return ""
}

// dynamicSite is a call whose callee SSA does not know statically: an
// interface method call or a call through a function value.
type dynamicSite struct {
	pkg    *ssa.Package
	caller *ssa.Function
	call   ssa.CallInstruction
	key    string
	file   string
}

// chaCandidateLimit is the most candidates a class-hierarchy answer lists. An
// interface implemented across the program (an `error`, a `fmt.Stringer`) or a
// common function signature matches hundreds of functions; listing them all
// would make every such call reach every implementation, which is no more use
// to a rule than the abstract callee and far more expensive downstream. Above
// the limit the site names the abstract callee instead, with the count.
const chaCandidateLimit = 16

// emitCallEdges emits, for every dynamic call site, one `call_edges` row with
// its candidate callees.
//
// Variable type analysis (VTA) propagates the concrete types and function
// literals that reach each interface value and function value through the
// program; a site gets every callee it can reach, labelled `vta`. A site VTA
// gives no callee (typically an interface parameter of a function nothing in
// the program calls) falls back to class hierarchy analysis: every method of
// every type in the program that implements the interface, or every function
// whose signature matches the function value, labelled `cha`. Both sets are
// sound over-approximations of the program as loaded, modulo reflection and
// unsafe; VTA is the more precise. Dependencies are loaded from export data,
// so a callee whose concrete type exists only inside a dependency's code is
// not in the program and is not listed.
//
// A site neither analysis gives a callee, or one whose class-hierarchy answer
// exceeds chaCandidateLimit, is labelled `type_hierarchy` and names its
// abstract callee: the interface method, or the function-value signature.
func (e *emitter) emitCallEdges(prog *ssa.Program, sites []dynamicSite) {
	if len(sites) == 0 {
		return
	}
	cg := e.callGraphAnalysis(prog)
	for _, site := range sites {
		row := Row{
			"kind":                "call_edges",
			"package_id":          packageID(site.pkg),
			"package_path":        packagePath(site.pkg),
			"caller":              functionName(site.caller),
			"callsite_stable_key": site.key,
			"stable_key":          stableKey(packageID(site.pkg), "call_edges", site.key),
		}
		if site.file != "" {
			row["file"] = site.file
		}
		algorithm := "vta"
		targets := cg.callees(site.caller, site.call)
		if len(targets) == 0 {
			algorithm = "cha"
			targets = cg.hierarchyCallees(site.call)
		}
		names, origins := declaredCalleeNames(prog, targets)
		if len(names) == 0 || (algorithm == "cha" && len(names) > chaCandidateLimit) {
			abstract, abstractKind := abstractCallee(site.call)
			if abstract == "" {
				continue
			}
			row["algorithm"] = "type_hierarchy"
			row["callee"] = abstract
			row["callee_kind"] = abstractKind
			if len(names) > 0 {
				row["candidates"] = len(names)
			}
			e.add(row)
			continue
		}
		row["algorithm"] = algorithm
		row["callees"] = names
		row["callee_origins"] = origins
		e.add(row)
	}
}

// callGraphAnalysis is the program's variable-type-analysis call graph over
// every function SSA built, computed once per run and shared by the call-edge
// rows and the flow bodies, with the class-hierarchy answer built on demand.
type callGraphAnalysis struct {
	funcs     map[*ssa.Function]bool
	graph     *callgraph.Graph
	byCaller  map[*ssa.Function]map[ssa.CallInstruction][]*ssa.Function
	hierarchy func(ssa.CallInstruction) []*ssa.Function
}

func (e *emitter) callGraphAnalysis(prog *ssa.Program) *callGraphAnalysis {
	if e.callGraphResult == nil {
		funcs := ssautil.AllFunctions(prog)
		e.callGraphResult = &callGraphAnalysis{
			funcs:    funcs,
			graph:    vta.CallGraph(funcs, nil),
			byCaller: make(map[*ssa.Function]map[ssa.CallInstruction][]*ssa.Function),
		}
	}
	return e.callGraphResult
}

// callees are the candidates variable-type analysis gives a call of caller.
func (cg *callGraphAnalysis) callees(caller *ssa.Function, call ssa.CallInstruction) []*ssa.Function {
	bySite, ok := cg.byCaller[caller]
	if !ok {
		bySite = make(map[ssa.CallInstruction][]*ssa.Function)
		if node := cg.graph.Nodes[caller]; node != nil {
			for _, edge := range node.Out {
				if edge.Site != nil && edge.Callee != nil && edge.Callee.Func != nil {
					bySite[edge.Site] = append(bySite[edge.Site], edge.Callee.Func)
				}
			}
		}
		cg.byCaller[caller] = bySite
	}
	return bySite[call]
}

// hierarchyCallees are the class-hierarchy candidates of a dynamic call.
func (cg *callGraphAnalysis) hierarchyCallees(call ssa.CallInstruction) []*ssa.Function {
	if cg.hierarchy == nil {
		cg.hierarchy = hierarchyCallees(cg.funcs)
	}
	return cg.hierarchy(call)
}

// declaredCalleeNames names each target by its declared function, deduplicated
// and sorted, with each one's generic origin ("" when it has none) at the same
// index.
func declaredCalleeNames(prog *ssa.Program, targets []*ssa.Function) ([]string, []string) {
	byName := make(map[string]string, len(targets))
	for _, target := range targets {
		target = declaredCallee(prog, target)
		name := functionName(target)
		if _, seen := byName[name]; seen {
			continue
		}
		origin := ""
		if from := target.Origin(); from != nil {
			origin = functionName(from)
		}
		byName[name] = origin
	}
	names := make([]string, 0, len(byName))
	for name := range byName {
		names = append(names, name)
	}
	sort.Strings(names)
	origins := make([]string, len(names))
	for i, name := range names {
		origins[i] = byName[name]
	}
	return names, origins
}

// abstractCallee names what a dynamic call invokes when no concrete callee is
// listed: the interface method (`io.Writer.Write`, kind `interface_method`) or
// the function value's signature (`func(int) error`, kind `signature`).
func abstractCallee(call ssa.CallInstruction) (string, string) {
	if method := interfaceMethodName(call); method != "" {
		return method, "interface_method"
	}
	common := call.Common()
	if common == nil || common.IsInvoke() || common.StaticCallee() != nil || isBuiltinCall(common) {
		return "", ""
	}
	return canonicalTypeString(common.Signature()), "signature"
}

// interfaceMethodName names the abstract method an interface call invokes, as
// the interface type followed by the method (`io.Writer.Write`), or "" when the
// call is not an interface call.
func interfaceMethodName(call ssa.CallInstruction) string {
	common := call.Common()
	if common == nil || !common.IsInvoke() || common.Method == nil || common.Value == nil {
		return ""
	}
	return canonicalTypeString(common.Value.Type()) + "." + common.Method.Name()
}

// declaredCallee maps a synthesized wrapper — the pointer-receiver method SSA
// creates for a value-receiver method, a promoted method of an embedded field
// — to the declared method it forwards to, so an edge names a function with
// source.
func declaredCallee(prog *ssa.Program, fn *ssa.Function) *ssa.Function {
	if fn.Synthetic == "" {
		return fn
	}
	object, ok := fn.Object().(*types.Func)
	if !ok || object == nil {
		return fn
	}
	if declared := prog.FuncValue(object); declared != nil && declared.Synthetic == "" {
		return declared
	}
	return fn
}

// hierarchyCallees resolves a dynamic call by class hierarchy analysis over
// `funcs`, as golang.org/x/tools/go/callgraph/cha does: an interface call to
// every concrete method of a type implementing the interface, a function-value
// call to every function of the same signature. It computes the answer per
// site on demand instead of building the whole program's graph.
func hierarchyCallees(funcs map[*ssa.Function]bool) func(ssa.CallInstruction) []*ssa.Function {
	var bySignature typeutil.Map
	methodsByID := make(map[string][]*ssa.Function)
	for fn := range funcs {
		if fn.Signature.Recv() == nil {
			if fn.Name() == "init" && fn.Synthetic == "package initializer" {
				continue
			}
			existing, _ := bySignature.At(fn.Signature).([]*ssa.Function)
			bySignature.Set(fn.Signature, append(existing, fn))
		} else if object, ok := fn.Object().(*types.Func); ok && object != nil {
			methodsByID[object.Id()] = append(methodsByID[object.Id()], fn)
		}
	}
	type interfaceMethod struct {
		iface *types.Interface
		id    string
	}
	memo := make(map[interfaceMethod][]*ssa.Function)
	return func(call ssa.CallInstruction) []*ssa.Function {
		common := call.Common()
		if common.IsInvoke() {
			iface, ok := common.Value.Type().Underlying().(*types.Interface)
			if !ok || common.Method == nil {
				return nil
			}
			key := interfaceMethod{iface: iface, id: common.Method.Id()}
			if methods, ok := memo[key]; ok {
				return methods
			}
			var methods []*ssa.Function
			for _, fn := range methodsByID[key.id] {
				if types.Implements(fn.Signature.Recv().Type(), iface) {
					methods = append(methods, fn)
				}
			}
			memo[key] = methods
			return methods
		}
		if common.StaticCallee() != nil || isBuiltinCall(common) {
			return nil
		}
		functions, _ := bySignature.At(common.Signature()).([]*ssa.Function)
		return functions
	}
}

// emitTypeFacts emits the declarations a type view answers from: interfaces,
// struct fields, which concrete types implement which interfaces, and the
// generic instantiations and type conversions written in each root package.
func (e *emitter) emitTypeFacts(roots []*packages.Package) {
	interfaces := make(map[string]*types.Named)
	var concrete []*types.Named
	seenPackages := make(map[string]bool)
	for _, pkg := range roots {
		if pkg.Types == nil || pkg.TypesInfo == nil {
			continue
		}
		e.emitInstantiationsAndConversions(pkg)
		// A package's test variants repeat its declarations; declare them once.
		if seenPackages[pkg.PkgPath] {
			continue
		}
		seenPackages[pkg.PkgPath] = true
		scope := pkg.Types.Scope()
		for _, name := range scope.Names() {
			typeName, ok := scope.Lookup(name).(*types.TypeName)
			if !ok || typeName.IsAlias() {
				continue
			}
			named, ok := typeName.Type().(*types.Named)
			if !ok {
				continue
			}
			switch underlying := named.Underlying().(type) {
			case *types.Interface:
				e.emitInterface(pkg, named, underlying)
				if named.TypeParams().Len() == 0 && underlying.NumMethods() > 0 {
					interfaces[canonicalTypeString(named)] = named
				}
			case *types.Struct:
				e.emitFields(pkg, named, underlying)
				if named.TypeParams().Len() == 0 {
					concrete = append(concrete, named)
				}
			default:
				if named.TypeParams().Len() == 0 {
					concrete = append(concrete, named)
				}
			}
		}
		// Interfaces a root package names but does not declare (`error`,
		// `io.Writer`, `http.Handler`) are the ones its types are most often
		// checked against.
		for _, object := range pkg.TypesInfo.Uses {
			typeName, ok := object.(*types.TypeName)
			if !ok {
				continue
			}
			named, ok := types.Unalias(typeName.Type()).(*types.Named)
			if !ok || named.TypeParams().Len() > 0 {
				continue
			}
			if iface, ok := named.Underlying().(*types.Interface); ok && iface.NumMethods() > 0 {
				interfaces[canonicalTypeString(named)] = named
			}
		}
	}
	e.emitImplements(concrete, interfaces)
}

func (e *emitter) emitInterface(pkg *packages.Package, named *types.Named, iface *types.Interface) {
	methods := make([]string, 0, iface.NumMethods())
	for i := 0; i < iface.NumMethods(); i++ {
		methods = append(methods, iface.Method(i).Name())
	}
	sort.Strings(methods)
	identity := canonicalTypeString(named)
	row := Row{
		"kind":         "interface",
		"package_id":   pkg.PkgPath,
		"package_path": pkg.PkgPath,
		"type":         identity,
		"methods":      methods,
		"stable_key":   stableKey(pkg.PkgPath, "interface", identity),
	}
	e.addDeclarationPosition(row, named.Obj().Pos(), len(named.Obj().Name()))
	e.addOnce(row)
}

func (e *emitter) emitFields(pkg *packages.Package, named *types.Named, structure *types.Struct) {
	owner := canonicalTypeString(named)
	for i := 0; i < structure.NumFields(); i++ {
		field := structure.Field(i)
		row := Row{
			"kind":         "field",
			"package_id":   pkg.PkgPath,
			"package_path": pkg.PkgPath,
			"type":         owner,
			"name":         field.Name(),
			"index":        i,
			"field_type":   canonicalTypeString(field.Type()),
			"embedded":     field.Embedded(),
			"stable_key":   stableKey(pkg.PkgPath, "field", owner, strconv.Itoa(i)),
		}
		if tag := structure.Tag(i); tag != "" {
			row["tag"] = tag
		}
		e.addDeclarationPosition(row, field.Pos(), len(field.Name()))
		e.addOnce(row)
	}
}

// emitImplements emits, for every concrete root type and every interface a
// root package declares or names, whether the type (or only a pointer to it)
// implements the interface. Candidates are narrowed by method name first, so
// only pairs whose names already match pay for a signature check.
func (e *emitter) emitImplements(concrete []*types.Named, interfaces map[string]*types.Named) {
	byMethod := make(map[string][]int)
	for i, named := range concrete {
		methods := types.NewMethodSet(types.NewPointer(named))
		for j := 0; j < methods.Len(); j++ {
			byMethod[methods.At(j).Obj().Id()] = append(byMethod[methods.At(j).Obj().Id()], i)
		}
	}
	names := make([]string, 0, len(interfaces))
	for name := range interfaces {
		names = append(names, name)
	}
	sort.Strings(names)
	type pair struct {
		concrete, iface string
		pointer         bool
		pkg             string
	}
	var pairs []pair
	for _, name := range names {
		iface := interfaces[name].Underlying().(*types.Interface)
		var candidates []int
		for i := 0; i < iface.NumMethods(); i++ {
			holders := byMethod[iface.Method(i).Id()]
			if i == 0 {
				candidates = append([]int(nil), holders...)
			} else {
				candidates = intersectSorted(candidates, holders)
			}
			if len(candidates) == 0 {
				break
			}
		}
		for _, index := range candidates {
			named := concrete[index]
			pointer := false
			if !types.Implements(named, iface) {
				if !types.Implements(types.NewPointer(named), iface) {
					continue
				}
				pointer = true
			}
			pkg := ""
			if named.Obj().Pkg() != nil {
				pkg = named.Obj().Pkg().Path()
			}
			pairs = append(pairs, pair{concrete: canonicalTypeString(named), iface: name, pointer: pointer, pkg: pkg})
		}
	}
	sort.Slice(pairs, func(i, j int) bool {
		if pairs[i].concrete != pairs[j].concrete {
			return pairs[i].concrete < pairs[j].concrete
		}
		return pairs[i].iface < pairs[j].iface
	})
	for _, pair := range pairs {
		e.addOnce(Row{
			"kind":         "implements",
			"package_id":   pair.pkg,
			"package_path": pair.pkg,
			"type":         pair.concrete,
			"interface":    pair.iface,
			"via_pointer":  pair.pointer,
			"stable_key":   stableKey(pair.pkg, "implements", pair.concrete, pair.iface),
		})
	}
}

// intersectSorted intersects two ascending index lists.
func intersectSorted(left, right []int) []int {
	out := left[:0]
	i, j := 0, 0
	for i < len(left) && j < len(right) {
		switch {
		case left[i] == right[j]:
			out = append(out, left[i])
			i++
			j++
		case left[i] < right[j]:
			i++
		default:
			j++
		}
	}
	return out
}

// emitInstantiationsAndConversions emits each generic instantiation the
// package's source spells out and each call expression that is a type
// conversion. A conversion such as `ItemType(raw)` has call syntax, so a
// syntax-level lowering sees a call; SSA has no call there.
func (e *emitter) emitInstantiationsAndConversions(pkg *packages.Package) {
	type instance struct {
		ident    *ast.Ident
		instance types.Instance
	}
	instances := make([]instance, 0, len(pkg.TypesInfo.Instances))
	for ident, inst := range pkg.TypesInfo.Instances {
		instances = append(instances, instance{ident: ident, instance: inst})
	}
	sort.Slice(instances, func(i, j int) bool { return instances[i].ident.Pos() < instances[j].ident.Pos() })
	for _, item := range instances {
		object := pkg.TypesInfo.Uses[item.ident]
		if object == nil {
			object = pkg.TypesInfo.Defs[item.ident]
		}
		if object == nil {
			continue
		}
		generic := object.Name()
		if object.Pkg() != nil {
			generic = object.Pkg().Path() + "." + generic
		}
		kind := "func"
		if _, ok := object.(*types.TypeName); ok {
			kind = "type"
		}
		args := make([]string, 0, item.instance.TypeArgs.Len())
		for i := 0; i < item.instance.TypeArgs.Len(); i++ {
			args = append(args, canonicalTypeString(item.instance.TypeArgs.At(i)))
		}
		span := e.positionSpan(item.ident.Pos(), item.ident.End())
		if span == nil {
			continue
		}
		file := posFile(e.fset, item.ident.Pos(), e.root)
		e.addOnce(Row{
			"kind":         "instantiation",
			"package_id":   pkg.PkgPath,
			"package_path": pkg.PkgPath,
			"generic":      generic,
			"generic_kind": kind,
			"type_args":    args,
			"type":         canonicalTypeString(item.instance.Type),
			"file":         file,
			"span":         span,
			"stable_key":   stableKey(pkg.PkgPath, "instantiation", file, strconv.Itoa(span.StartByte)),
		})
	}
	for _, file := range pkg.Syntax {
		ast.Inspect(file, func(node ast.Node) bool {
			call, ok := node.(*ast.CallExpr)
			if !ok {
				return true
			}
			tv, ok := pkg.TypesInfo.Types[call.Fun]
			if !ok {
				return true
			}
			kind, name := "", ""
			switch {
			case tv.IsType() && len(call.Args) == 1:
				kind = "conversion"
			case tv.IsBuiltin():
				// A builtin is a call expression, but SSA lowers several of them
				// (`make`, `new`, `panic`) to instructions that are not calls, so
				// the syntax is the only place they can be recognized.
				kind = "builtin_call"
				name = builtinName(call.Fun)
			default:
				return true
			}
			span := e.positionSpan(call.Pos(), call.End())
			if span == nil {
				return true
			}
			path := posFile(e.fset, call.Pos(), e.root)
			row := Row{
				"kind":         kind,
				"package_id":   pkg.PkgPath,
				"package_path": pkg.PkgPath,
				"file":         path,
				"span":         span,
				"stable_key":   stableKey(pkg.PkgPath, kind, path, strconv.Itoa(span.StartByte), strconv.Itoa(span.EndByte)),
			}
			if kind == "conversion" {
				row["type"] = canonicalTypeString(tv.Type)
			} else {
				row["name"] = name
			}
			e.addOnce(row)
			return true
		})
	}
}

// builtinName is the identifier a builtin call is spelled with, through any
// parentheses.
func builtinName(fun ast.Expr) string {
	for {
		paren, ok := fun.(*ast.ParenExpr)
		if !ok {
			break
		}
		fun = paren.X
	}
	if ident, ok := fun.(*ast.Ident); ok {
		return ident.Name
	}
	return ""
}

// emitParams emits the declared parameters of a function with source.
func (e *emitter) emitParams(pkg *ssa.Package, fn *ssa.Function) {
	if fn == nil || fn.Syntax() == nil || fn.Signature == nil || fn.Parent() != nil {
		return
	}
	params := fn.Signature.Params()
	for i := 0; i < params.Len(); i++ {
		param := params.At(i)
		row := Row{
			"kind":         "param",
			"package_id":   packageID(pkg),
			"package_path": packagePath(pkg),
			"function":     functionName(fn),
			"index":        i,
			"name":         param.Name(),
			"type":         canonicalTypeString(param.Type()),
			"stable_key":   stableKey(packageID(pkg), "param", functionName(fn), strconv.Itoa(i)),
		}
		if fn.Signature.Variadic() && i == params.Len()-1 {
			row["variadic"] = true
		}
		e.addDeclarationPosition(row, param.Pos(), len(param.Name()))
		e.addOnce(row)
	}
}

// addDeclarationPosition anchors a declaration row at its identifier.
func (e *emitter) addDeclarationPosition(row Row, pos token.Pos, length int) {
	if !pos.IsValid() {
		return
	}
	if span := e.positionSpan(pos, pos+token.Pos(length)); span != nil {
		row["file"] = posFile(e.fset, pos, e.root)
		row["span"] = span
	}
}

// addOnce emits a row unless a row with the same stable key was already
// emitted: the declarations of a package and of its test variant share keys.
func (e *emitter) addOnce(row Row) {
	key, _ := row["stable_key"].(string)
	if key != "" {
		if e.emittedTypedKeys[key] {
			return
		}
		e.emittedTypedKeys[key] = true
	}
	e.add(row)
}
