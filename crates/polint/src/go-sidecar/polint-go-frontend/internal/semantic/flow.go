package semantic

import (
	"go/constant"
	"go/token"
	"go/types"
	"sort"
	"strconv"

	"golang.org/x/tools/go/ssa"
)

// The flow program of a function body: what the data-flow solver needs from SSA,
// and nothing else. Every value the body computes, every parameter and free
// variable, every global it addresses and every constant it passes is a slot;
// each statement moves values between slots.
//
// Statement kinds (field `o`):
//   - copy:    d gets everything a has (phi edges, conversions, interface boxing,
//     slicing, arithmetic and string concatenation operands).
//   - load:    d gets what is at a's path step s.
//   - addr:    d is the address of a's path s, a dotted list of steps: `*.f:Name`
//     for a field of what a points to, `*.[]` for an element of an array a
//     points to, `[]` for an element of the slice a.
//   - store:   what a points to gets b; with s `[]`, an element of the map or
//     channel a gets b.
//   - alias:   d's path p is the same place as a's path s (a slice of an array,
//     an array pointer to a slice's backing array).
//   - alloc:   d is a new memory location (Alloc, MakeSlice, MakeMap, MakeChan).
//   - closure: d is a closure of function f binding the slots in args to its
//     free variables in order.
//   - call:    d is the result (a tuple when the callee has several results) of
//     calling the candidates in callees with args; alg says how they were found:
//     `static` (the call names its callee), `vta` (variable-type analysis),
//     `cha` (class hierarchy) or `unknown`; for a call of a closure value fv is
//     that value, whose bindings are the callee's free variables; for an
//     interface call iface names the interface method.
//   - ret:     the body returns args (one per result).
//
// Path steps: `f:Name` a struct field, `[]` an element of a slice, array, map,
// string or channel, `*` the value a pointer points to, `r:N` the Nth value of
// a tuple.
//
// `classes` has one character per slot naming what kind of value its type holds,
// for queries that declare some kinds unable to carry the data they track: `c` a
// context.Context, `b` a boolean, `n` a number, `.` anything else. It is omitted
// when every slot is `.`.
//
// A slot is an index into the body's slots. Operands (`a`, `b`, `args`) are slots,
// or -1-k for the constant k of `consts`; an omitted `a` or `b` is slot 0. The
// destination `d` and the closure value `fv` are encoded as slot+1, so that an
// omitted one means there is none.
type flowBody struct {
	Params  []int       `json:"params"`
	Free    []int       `json:"free,omitempty"`
	Results int         `json:"results,omitempty"`
	Slots   int         `json:"slots"`
	Names   []flowName  `json:"names,omitempty"`
	PTypes  []string    `json:"ptypes,omitempty"`
	Globals []flowName  `json:"globals,omitempty"`
	Consts  []string    `json:"consts,omitempty"`
	Classes string      `json:"classes,omitempty"`
	Stmts   []*flowStmt `json:"stmts"`
}

// flowName names a slot: a parameter, a free variable, an address-taken local,
// or the address of a global.
type flowName struct {
	Slot int    `json:"s"`
	Name string `json:"n"`
}

type flowStmt struct {
	Op        string   `json:"o"`
	Dst       int      `json:"d,omitempty"`
	A         int      `json:"a,omitempty"`
	B         int      `json:"b,omitempty"`
	Step      string   `json:"s,omitempty"`
	P         string   `json:"p,omitempty"`
	Fn        string   `json:"f,omitempty"`
	Args      []int    `json:"args,omitempty"`
	Callees   []string `json:"callees,omitempty"`
	Iface     string   `json:"iface,omitempty"`
	Algorithm string   `json:"alg,omitempty"`
	Builtin   string   `json:"builtin,omitempty"`
	Mode      string   `json:"mode,omitempty"`
	FV        int      `json:"fv,omitempty"`
	Line      int      `json:"l,omitempty"`
	Col       int      `json:"c,omitempty"`
	StartByte int      `json:"sb,omitempty"`
	EndByte   int      `json:"eb,omitempty"`
}

// flowBuilder numbers one body's slots and collects its statements.
type flowBuilder struct {
	e      *emitter
	fn     *ssa.Function
	cg     *callGraphAnalysis
	index  *callIndex
	slots  map[ssa.Value]int
	consts map[string]int
	body   flowBody
}

// emitFlowBodies emits one `flow_body` row per function body of the program:
// the analysed packages' functions, methods, closures and generic instances,
// and the wrappers SSA synthesizes for them. Bodies are emitted whatever the
// scan scope, because a flow between two in-scope places can pass through code
// outside it.
func (e *emitter) emitFlowBodies(prog *ssa.Program) {
	cg := e.callGraphAnalysis(prog)
	var functions []*ssa.Function
	for fn := range cg.funcs {
		if len(fn.Blocks) > 0 {
			functions = append(functions, fn)
		}
	}
	sort.Slice(functions, func(i, j int) bool {
		left, right := functionName(functions[i]), functionName(functions[j])
		if left != right {
			return left < right
		}
		return functions[i].Pos() < functions[j].Pos()
	})
	seen := make(map[string]bool, len(functions))
	for _, fn := range functions {
		name := functionName(fn)
		if seen[name] {
			continue
		}
		seen[name] = true
		e.emitFlowBody(fn, cg)
	}
}

func (e *emitter) emitFlowBody(fn *ssa.Function, cg *callGraphAnalysis) {
	b := &flowBuilder{
		e:      e,
		fn:     fn,
		cg:     cg,
		index:  newCallIndex(fn.Syntax()),
		slots:  make(map[ssa.Value]int),
		consts: make(map[string]int),
	}
	for _, param := range fn.Params {
		slot := b.slot(param)
		b.body.Params = append(b.body.Params, slot)
		b.body.Names = append(b.body.Names, flowName{Slot: slot, Name: param.Name()})
		b.body.PTypes = append(b.body.PTypes, canonicalTypeString(param.Type()))
	}
	for _, free := range fn.FreeVars {
		slot := b.slot(free)
		b.body.Free = append(b.body.Free, slot)
		b.body.Names = append(b.body.Names, flowName{Slot: slot, Name: free.Name()})
	}
	if fn.Signature != nil {
		b.body.Results = fn.Signature.Results().Len()
	}
	for _, block := range fn.Blocks {
		for _, instr := range block.Instrs {
			b.instr(instr)
		}
	}
	b.body.Slots = len(b.slots)
	b.body.Classes = b.classes()
	pkg := flowPackagePath(fn)
	row := Row{
		"kind":         "flow_body",
		"package_id":   pkg,
		"package_path": pkg,
		"function":     functionName(fn),
		"flow":         &b.body,
		"stable_key":   stableKey(pkg, functionName(fn), "flow"),
	}
	if syntax := fn.Syntax(); syntax != nil {
		if span := e.positionSpan(syntax.Pos(), syntax.End()); span != nil {
			row["file"] = posFile(e.fset, syntax.Pos(), e.root)
			row["span"] = span
		}
	}
	e.add(row)
}

// flowPackagePath is the package a body belongs to: its own, or, for a body SSA
// synthesizes (a method wrapper, a bound-method closure, a generic instance),
// that of the function it stands for.
func flowPackagePath(fn *ssa.Function) string {
	if path := packagePath(fn.Pkg); path != "" {
		return path
	}
	if origin := fn.Origin(); origin != nil {
		if path := packagePath(origin.Pkg); path != "" {
			return path
		}
	}
	if obj := fn.Object(); obj != nil && obj.Pkg() != nil {
		return obj.Pkg().Path()
	}
	return ""
}

// classes is the body's `classes` text, or "" when no slot has a class.
func (b *flowBuilder) classes() string {
	classes := make([]byte, len(b.slots))
	any := false
	for value, slot := range b.slots {
		classes[slot] = valueClass(value.Type())
		any = any || classes[slot] != '.'
	}
	if !any {
		return ""
	}
	return string(classes)
}

// valueClass names the kind of value a type holds: `c` context.Context, `b` a
// boolean, `n` a number, `.` anything else.
func valueClass(typ types.Type) byte {
	typ = types.Unalias(typ)
	if named, ok := typ.(*types.Named); ok {
		if obj := named.Obj(); obj != nil && obj.Pkg() != nil && obj.Pkg().Path() == "context" && obj.Name() == "Context" {
			return 'c'
		}
	}
	if basic, ok := typ.Underlying().(*types.Basic); ok {
		switch info := basic.Info(); {
		case info&types.IsBoolean != 0:
			return 'b'
		case info&types.IsNumeric != 0:
			return 'n'
		}
	}
	return '.'
}

// slot numbers a value of the body. A global gets one slot per body holding its
// address, named after it.
func (b *flowBuilder) slot(value ssa.Value) int {
	if slot, ok := b.slots[value]; ok {
		return slot
	}
	slot := len(b.slots)
	b.slots[value] = slot
	if global, ok := value.(*ssa.Global); ok {
		name := global.Name()
		if global.Pkg != nil && global.Pkg.Pkg != nil {
			name = global.Pkg.Pkg.Path() + "." + global.Name()
		}
		b.body.Globals = append(b.body.Globals, flowName{Slot: slot, Name: name})
	}
	return slot
}

// operand is the slot of a value, or -1-k for constant k. A constant keeps its
// text when it is a short string, which is what models and secret names match.
func (b *flowBuilder) operand(value ssa.Value) int {
	if c, ok := value.(*ssa.Const); ok {
		text := ""
		if c.Value != nil && c.Value.Kind() == constant.String {
			if s := constant.StringVal(c.Value); len(s) <= 128 {
				text = s
			}
		}
		key := text
		if text == "" {
			key = "\x00"
		}
		index, ok := b.consts[key]
		if !ok {
			index = len(b.body.Consts)
			b.consts[key] = index
			b.body.Consts = append(b.body.Consts, text)
		}
		return -1 - index
	}
	return b.slot(value)
}

func (b *flowBuilder) operands(values []ssa.Value) []int {
	out := make([]int, len(values))
	for i, value := range values {
		out[i] = b.operand(value)
	}
	return out
}

func (b *flowBuilder) add(stmt *flowStmt, pos token.Pos) {
	if pos.IsValid() {
		position := b.e.fset.Position(pos)
		stmt.Line, stmt.Col = position.Line, position.Column
	}
	b.body.Stmts = append(b.body.Stmts, stmt)
}

// slotRef encodes a slot so that 0 stays distinguishable from an absent field:
// destination and closure-value fields carry slot+1, and 0 means none.
func slotRef(slot int) int {
	if slot < 0 {
		return 0
	}
	return slot + 1
}

func (b *flowBuilder) copy(dst ssa.Value, src ssa.Value, pos token.Pos) {
	b.add(&flowStmt{Op: "copy", Dst: slotRef(b.slot(dst)), A: b.operand(src)}, pos)
}

func (b *flowBuilder) load(dst ssa.Value, base ssa.Value, step string, pos token.Pos) {
	b.add(&flowStmt{Op: "load", Dst: slotRef(b.slot(dst)), A: b.operand(base), Step: step}, pos)
}

func fieldStep(typ types.Type, index int) string {
	if pointer, ok := typ.Underlying().(*types.Pointer); ok {
		typ = pointer.Elem()
	}
	if structure, ok := typ.Underlying().(*types.Struct); ok && index < structure.NumFields() {
		return "f:" + structure.Field(index).Name()
	}
	return "f:" + strconv.Itoa(index)
}

func (b *flowBuilder) instr(instr ssa.Instruction) {
	switch in := instr.(type) {
	case *ssa.Alloc:
		b.add(&flowStmt{Op: "alloc", Dst: slotRef(b.slot(in))}, in.Pos())
		if in.Comment != "" && !in.Heap {
			b.body.Names = append(b.body.Names, flowName{Slot: b.slot(in), Name: in.Comment})
		}
	case *ssa.MakeSlice, *ssa.MakeMap, *ssa.MakeChan:
		value := in.(ssa.Value)
		b.add(&flowStmt{Op: "alloc", Dst: slotRef(b.slot(value))}, in.Pos())
	case *ssa.BinOp:
		switch in.Op {
		case token.EQL, token.NEQ, token.LSS, token.LEQ, token.GTR, token.GEQ:
			return
		}
		b.copy(in, in.X, in.Pos())
		b.copy(in, in.Y, in.Pos())
	case *ssa.UnOp:
		switch in.Op {
		case token.MUL:
			b.load(in, in.X, "*", in.Pos())
		case token.ARROW:
			b.load(in, in.X, "[]", in.Pos())
		default:
			b.copy(in, in.X, in.Pos())
		}
	case *ssa.Phi:
		for _, edge := range in.Edges {
			b.copy(in, edge, in.Pos())
		}
	case *ssa.ChangeType:
		b.copy(in, in.X, in.Pos())
	case *ssa.ChangeInterface:
		b.copy(in, in.X, in.Pos())
	case *ssa.Convert:
		b.copy(in, in.X, in.Pos())
	case *ssa.MultiConvert:
		b.copy(in, in.X, in.Pos())
	case *ssa.MakeInterface:
		b.copy(in, in.X, in.Pos())
	case *ssa.SliceToArrayPointer:
		// The array the result points to is the slice's backing array.
		b.add(&flowStmt{Op: "alias", Dst: slotRef(b.slot(in)), P: "*.[]", A: b.operand(in.X), Step: "[]"}, in.Pos())
	case *ssa.Slice:
		// Slicing an array (through its pointer) makes a slice of that array;
		// slicing a slice or a string shares or copies what the operand has.
		if _, ok := in.X.Type().Underlying().(*types.Pointer); ok {
			b.add(&flowStmt{Op: "alias", Dst: slotRef(b.slot(in)), P: "[]", A: b.operand(in.X), Step: "*.[]"}, in.Pos())
		} else {
			b.copy(in, in.X, in.Pos())
		}
	case *ssa.TypeAssert:
		b.copy(in, in.X, in.Pos())
	case *ssa.Range:
		b.copy(in, in.X, in.Pos())
	case *ssa.Extract:
		b.load(in, in.Tuple, "r:"+strconv.Itoa(in.Index), in.Pos())
	case *ssa.Field:
		b.load(in, in.X, fieldStep(in.X.Type(), in.Field), in.Pos())
	case *ssa.FieldAddr:
		b.add(&flowStmt{Op: "addr", Dst: slotRef(b.slot(in)), A: b.operand(in.X), Step: "*." + fieldStep(in.X.Type(), in.Field)}, in.Pos())
	case *ssa.Index:
		b.load(in, in.X, "[]", in.Pos())
	case *ssa.IndexAddr:
		// An element of a slice is reached through the slice value itself; one
		// of an array, through the pointer to it.
		step := "[]"
		if _, ok := in.X.Type().Underlying().(*types.Pointer); ok {
			step = "*.[]"
		}
		b.add(&flowStmt{Op: "addr", Dst: slotRef(b.slot(in)), A: b.operand(in.X), Step: step}, in.Pos())
	case *ssa.Lookup:
		b.load(in, in.X, "[]", in.Pos())
	case *ssa.Next:
		b.load(in, in.Iter, "[]", in.Pos())
	case *ssa.Select:
		for _, state := range in.States {
			if state.Dir == types.RecvOnly {
				b.load(in, state.Chan, "[]", in.Pos())
			}
		}
	case *ssa.Store:
		b.add(&flowStmt{Op: "store", A: b.operand(in.Addr), B: b.operand(in.Val)}, in.Pos())
	case *ssa.MapUpdate:
		b.add(&flowStmt{Op: "store", A: b.operand(in.Map), B: b.operand(in.Key), Step: "[]"}, in.Pos())
		b.add(&flowStmt{Op: "store", A: b.operand(in.Map), B: b.operand(in.Value), Step: "[]"}, in.Pos())
	case *ssa.Send:
		b.add(&flowStmt{Op: "store", A: b.operand(in.Chan), B: b.operand(in.X), Step: "[]"}, in.Pos())
	case *ssa.MakeClosure:
		fn, _ := in.Fn.(*ssa.Function)
		name := ""
		if fn != nil {
			name = functionName(fn)
		}
		b.add(&flowStmt{Op: "closure", Dst: slotRef(b.slot(in)), Fn: name, Args: b.operands(in.Bindings)}, in.Pos())
	case *ssa.Return:
		b.add(&flowStmt{Op: "ret", Args: b.operands(in.Results)}, in.Pos())
	case ssa.CallInstruction:
		b.call(in)
	}
}

// call records a call: its candidate callees with the algorithm that found
// them, its arguments (a method's receiver first) and its result.
func (b *flowBuilder) call(call ssa.CallInstruction) {
	common := call.Common()
	stmt := &flowStmt{Op: "call", Mode: callMode(call)}
	if value := call.Value(); value != nil {
		stmt.Dst = slotRef(b.slot(value))
	}
	args := common.Args
	switch {
	case common.IsInvoke():
		args = append([]ssa.Value{common.Value}, common.Args...)
		stmt.Callees, stmt.Algorithm = b.dynamicCallees(call)
		if common.Method != nil {
			stmt.Iface = "(" + canonicalTypeString(common.Value.Type()) + ")." + common.Method.Name()
		}
	default:
		if builtin, ok := common.Value.(*ssa.Builtin); ok {
			stmt.Builtin = builtin.Name()
			break
		}
		if callee := common.StaticCallee(); callee != nil {
			stmt.Callees = []string{functionName(callee)}
			stmt.Algorithm = "static"
			if _, ok := common.Value.(*ssa.MakeClosure); ok {
				stmt.FV = slotRef(b.slot(common.Value))
			}
			break
		}
		stmt.Callees, stmt.Algorithm = b.dynamicCallees(call)
		stmt.FV = slotRef(b.slot(common.Value))
	}
	stmt.Args = b.operands(args)
	pos := call.Pos()
	if syntax := b.index.syntaxFor(call); syntax != nil {
		if span := b.e.positionSpan(syntax.Pos(), syntax.End()); span != nil {
			stmt.StartByte, stmt.EndByte = span.StartByte, span.EndByte
		}
		pos = syntax.Pos()
	}
	b.add(stmt, pos)
}

// dynamicCallees are the candidates variable-type analysis gives a call, or,
// when it gives none, class-hierarchy analysis up to chaCandidateLimit.
func (b *flowBuilder) dynamicCallees(call ssa.CallInstruction) ([]string, string) {
	targets := b.cg.callees(b.fn, call)
	algorithm := "vta"
	if len(targets) == 0 {
		targets = b.cg.hierarchyCallees(call)
		algorithm = "cha"
		if len(targets) > chaCandidateLimit {
			return nil, "unknown"
		}
	}
	if len(targets) == 0 {
		return nil, "unknown"
	}
	names := make([]string, 0, len(targets))
	seen := make(map[string]bool, len(targets))
	for _, target := range targets {
		name := functionName(target)
		if !seen[name] {
			seen[name] = true
			names = append(names, name)
		}
	}
	sort.Strings(names)
	return names, algorithm
}
