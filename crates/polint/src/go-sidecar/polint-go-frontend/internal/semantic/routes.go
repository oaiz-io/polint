package semantic

import (
	_ "embed"
	"encoding/json"
	"fmt"
	"go/constant"
	"go/token"
	"go/types"
	"hash/fnv"
	"os"
	"sort"
	"strconv"
	"strings"

	"golang.org/x/tools/go/ssa"
)

// RouteModel describes one framework API as data: which call it is, the part
// it plays in building a route table, and where its arguments are. polint
// passes the models (built-in defaults plus a repository's own) as a JSON
// document; nothing in this file names a framework.
//
// Roles:
//   - router: a constructor; its result is a new, empty router.
//   - route: registers a handler for a method and path on its router.
//   - group: derives a child router that copies its parent's middleware at
//     the time of the call and extends the path prefix (gin `Group`, chi
//     `Route`, `With`, `Group`); a callback argument receives the child.
//   - use: appends middleware to its router; later registrations and later
//     groups see it, earlier ones do not.
//   - mount: attaches a router passed as an argument under a path prefix.
//   - subscriber: registers a message handler for a topic.
//   - passthrough: returns one of its arguments (a decorator or adapter
//     that wraps a handler).
//   - serve: dispatches requests to its router (an `http.Handler` serve
//     call, a test server); every handler of that router is reachable from
//     the calling function.
type RouteModel struct {
	Framework string `json:"framework"`
	Role      string `json:"role"`
	// Function names a package-level function (`net/http.HandleFunc`).
	Function string `json:"function,omitempty"`
	// Receivers and Methods name methods: a call matches when its static callee
	// or invoked interface method is one of Methods declared on one of
	// Receivers (named types, pointer or not; interfaces included).
	Receivers []string `json:"receivers,omitempty"`
	Methods   []string `json:"methods,omitempty"`
	// The HTTP method of a route: fixed, the called method's name, an
	// argument, or the method prefix of a `"GET /path"` pattern. A route with
	// none of them answers every method ("*").
	HTTPMethod      string `json:"http_method,omitempty"`
	MethodFromName  bool   `json:"method_from_name,omitempty"`
	MethodArgument  *int   `json:"method_argument,omitempty"`
	MethodInPattern bool   `json:"method_in_pattern,omitempty"`
	// Argument positions, counted without the receiver.
	PathArgument     *int `json:"path_argument,omitempty"`
	HandlerArgument  *int `json:"handler_argument,omitempty"`
	HandlersFrom     *int `json:"handlers_from,omitempty"`
	MiddlewareFrom   *int `json:"middleware_from,omitempty"`
	CallbackArgument *int `json:"callback_argument,omitempty"`
	RouterArgument   *int `json:"router_argument,omitempty"`
	NameArgument     *int `json:"name_argument,omitempty"`
	TopicArgument    *int `json:"topic_argument,omitempty"`
	Argument         *int `json:"argument,omitempty"`
	// DefaultRouter names the implicit router a package-level registration
	// uses (`net/http.DefaultServeMux`).
	DefaultRouter string `json:"default_router,omitempty"`
	// InitialMiddleware is what a router constructor installs by itself.
	InitialMiddleware []string `json:"initial_middleware,omitempty"`
	// ReturnsReceiver marks a route or use call whose result is its own
	// router, so chained calls keep registering on it.
	ReturnsReceiver bool `json:"returns_receiver,omitempty"`
}

// RouteModels is a route-model document: the built-in defaults, or a
// repository's own models, which polint passes with `--route-models`.
type RouteModels struct {
	Models []RouteModel `json:"models"`
}

// builtinRouteModelsJSON describes gin, chi, net/http (with httptest) and
// Watermill.
//
//go:embed route_models.json
var builtinRouteModelsJSON []byte

// BuiltinRouteModels returns the default models.
func BuiltinRouteModels() (*RouteModels, error) {
	return parseRouteModels(builtinRouteModelsJSON)
}

// LoadRouteModels reads a route-model document.
func LoadRouteModels(path string) (*RouteModels, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	return parseRouteModels(raw)
}

// WithBuiltinRouteModels puts a repository's models before the defaults, so
// a repository model of a call takes precedence over the default one.
func WithBuiltinRouteModels(repository *RouteModels) (*RouteModels, error) {
	builtin, err := BuiltinRouteModels()
	if err != nil {
		return nil, err
	}
	merged := &RouteModels{}
	if repository != nil {
		merged.Models = append(merged.Models, repository.Models...)
	}
	merged.Models = append(merged.Models, builtin.Models...)
	return merged, nil
}

func parseRouteModels(raw []byte) (*RouteModels, error) {
	var models RouteModels
	if err := json.Unmarshal(raw, &models); err != nil {
		return nil, fmt.Errorf("parse route models: %w", err)
	}
	for i, model := range models.Models {
		switch model.Role {
		case "router", "route", "group", "use", "mount", "subscriber", "passthrough", "serve":
		default:
			return nil, fmt.Errorf("route model %d: unknown role %q", i, model.Role)
		}
		if model.Function == "" && (len(model.Receivers) == 0 || len(model.Methods) == 0) {
			return nil, fmt.Errorf("route model %d: needs a function, or receivers and methods", i)
		}
	}
	return &models, nil
}

// routeModelIndex finds the model a call matches.
type routeModelIndex struct {
	byFunction map[string]*RouteModel
	byMethod   map[string]*RouteModel
	// routerTypes are the concrete named types models register on: an
	// embedded field of one of these types is the same router as its owner.
	routerTypes map[string]bool
}

func newRouteModelIndex(models *RouteModels) *routeModelIndex {
	index := &routeModelIndex{
		byFunction:  make(map[string]*RouteModel),
		byMethod:    make(map[string]*RouteModel),
		routerTypes: make(map[string]bool),
	}
	if models == nil {
		return index
	}
	for i := range models.Models {
		model := &models.Models[i]
		if model.Function != "" {
			if _, seen := index.byFunction[model.Function]; !seen {
				index.byFunction[model.Function] = model
			}
			continue
		}
		for _, receiver := range model.Receivers {
			index.routerTypes[receiver] = true
			for _, method := range model.Methods {
				key := receiver + "." + method
				if _, seen := index.byMethod[key]; !seen {
					index.byMethod[key] = model
				}
			}
		}
	}
	return index
}

// match returns the model of a call, or nil. A static callee matches by the
// declared function (so a promoted method or a wrapper matches as the method
// it forwards to); an interface call matches by the interface that declares
// the method.
func (index *routeModelIndex) match(common *ssa.CallCommon, callee *ssa.Function) (*RouteModel, string) {
	if common.IsInvoke() {
		if common.Method == nil {
			return nil, ""
		}
		if receiver := declaredReceiverName(common.Method); receiver != "" {
			name := common.Method.Name()
			return index.byMethod[receiver+"."+name], name
		}
		return nil, ""
	}
	if callee == nil {
		return nil, ""
	}
	if origin := callee.Origin(); origin != nil {
		callee = origin
	}
	if object, ok := callee.Object().(*types.Func); ok && object != nil {
		if receiver := declaredReceiverName(object); receiver != "" {
			return index.byMethod[receiver+"."+object.Name()], object.Name()
		}
		if object.Pkg() != nil {
			return index.byFunction[object.Pkg().Path()+"."+object.Name()], object.Name()
		}
	}
	return nil, ""
}

// declaredReceiverName is `pkg/path.Type` for a method declared on a named
// type (or on a pointer to one), else "".
func declaredReceiverName(method *types.Func) string {
	signature, ok := method.Type().(*types.Signature)
	if !ok || signature.Recv() == nil {
		return ""
	}
	return namedTypeName(signature.Recv().Type())
}

func namedTypeName(typ types.Type) string {
	if pointer, ok := types.Unalias(typ).(*types.Pointer); ok {
		typ = pointer.Elem()
	}
	named, ok := types.Unalias(typ).(*types.Named)
	if !ok || named.Obj() == nil {
		return ""
	}
	if named.Obj().Pkg() == nil {
		return named.Obj().Name()
	}
	return named.Obj().Pkg().Path() + "." + named.Obj().Name()
}

// routeFunc identifies a handler or a middleware: the function or method it
// is, the literal, the function that produced it, or the struct field it was
// read from.
type routeFunc struct {
	Name  string `json:"name"`
	Kind  string `json:"kind"`
	Field string `json:"field,omitempty"`
}

// callable is a function value the interpreter can enter: a declared
// function, a closure with its captured values, or only an identity.
type callable struct {
	ref      routeFunc
	fn       *ssa.Function
	bindings []absValue
}

// absValue is what the interpreter knows about an SSA value: the routers it
// may be, its text when it is a known string, and the functions it may be.
type absValue struct {
	routers []int
	text    *string
	funcs   []callable
}

func (value absValue) join(other absValue) absValue {
	if len(other.routers) == 0 && other.text == nil && len(other.funcs) == 0 {
		return value
	}
	if len(value.routers) == 0 && value.text == nil && len(value.funcs) == 0 {
		return other
	}
	out := absValue{routers: unionInts(value.routers, other.routers)}
	if value.text != nil && other.text != nil && *value.text == *other.text {
		out.text = value.text
	}
	out.funcs = append(append([]callable{}, value.funcs...), other.funcs...)
	sort.SliceStable(out.funcs, func(i, j int) bool { return callableLess(out.funcs[i], out.funcs[j]) })
	deduped := out.funcs[:0]
	for i, function := range out.funcs {
		if i > 0 && callableKey(function) == callableKey(out.funcs[i-1]) {
			continue
		}
		deduped = append(deduped, function)
	}
	out.funcs = deduped
	return out
}

func callableKey(function callable) string {
	return function.ref.Kind + "\x00" + function.ref.Name + "\x00" + function.ref.Field
}

func callableLess(left, right callable) bool {
	return callableKey(left) < callableKey(right)
}

func unionInts(left, right []int) []int {
	if len(right) == 0 {
		return left
	}
	if len(left) == 0 {
		return right
	}
	seen := make(map[int]bool, len(left)+len(right))
	var out []int
	for _, values := range [][]int{left, right} {
		for _, value := range values {
			if !seen[value] {
				seen[value] = true
				out = append(out, value)
			}
		}
	}
	sort.Ints(out)
	return out
}

// routePath is a path with the parts the interpreter could not read marked.
type routePath struct {
	text     string
	complete bool
}

func knownPath(text string) routePath { return routePath{text: text, complete: true} }

func (path routePath) join(next routePath) routePath {
	return routePath{text: joinRoutePath(path.text, next.text), complete: path.complete && next.complete}
}

// joinRoutePath concatenates a prefix and a relative path the way routers do:
// one slash between them, no doubled slash.
func joinRoutePath(prefix, relative string) string {
	switch {
	case prefix == "":
		return relative
	case relative == "":
		return prefix
	case strings.HasSuffix(prefix, "/") && strings.HasPrefix(relative, "/"):
		return prefix + relative[1:]
	case !strings.HasSuffix(prefix, "/") && !strings.HasPrefix(relative, "/"):
		return prefix + "/" + relative
	}
	return prefix + relative
}

const unknownPathPart = "{?}"

// routerObject is one router a program builds: its path prefix and the
// middleware a route registered on it now would run.
type routerObject struct {
	id        int
	key       string
	framework string
	prefix    routePath
	// middleware is copied into a child at creation and extended in place by
	// a use call.
	middleware []routeFunc
	// open marks a router whose origin the interpreter did not see (a
	// parameter of a function nothing in the program calls): its prefix and
	// the middleware before its first use call are unknown.
	open   bool
	parent int
	// mount, when the router was attached under another one.
	mountParent     int
	mountPrefix     routePath
	mountMiddleware []routeFunc
}

type routeRecord struct {
	framework  string
	transport  string
	method     string
	path       routePath
	registered routePath
	name       string
	handlers   []routeFunc
	middleware []routeFunc
	complete   bool
	router     int
	function   string
	file       string
	span       *Span
	position   string
}

type serveRecord struct {
	router   int
	function string
	file     string
	span     *Span
	position string
}

// Route interpretation is bounded: a program whose route setup needs more
// steps than this, or calls nested deeper than this, gets the routes found so
// far and a `route_budget` row instead of an unbounded walk.
const (
	routeStepBudget = 5_000_000
	routeDepthLimit = 48
)

// routeEngine interprets the parts of a program that build routers.
//
// It runs the program's `main` and `init` functions in order, entering only
// the functions that reach a model call or receive a router, and simulates
// the router objects they create: path prefixes, middleware chains with
// their copy-at-group-time semantics, and the registrations made on them.
// Struct fields and package variables hold what was stored in them anywhere
// (one location per field of a named struct type), which is how a router
// built in a constructor reaches the methods that register routes on it.
// Functions with model calls that no root reached run afterwards with open
// routers for their parameters, so their routes are listed with what is
// unknown about them marked.
type routeEngine struct {
	e            *emitter
	prog         *ssa.Program
	models       *routeModelIndex
	objects      []*routerObject
	objectByKey  map[string]int
	fields       map[string]absValue
	routes       []routeRecord
	serves       []serveRecord
	relevant     map[*ssa.Function]bool
	direct       map[*ssa.Function]bool
	implementers map[string][]*ssa.Function
	onStack      map[*ssa.Function]bool
	interpreted  map[*ssa.Function]bool
	depth        int
	steps        int
	exhausted    bool
	callIndexes  map[*ssa.Function]*callIndex
}

func (e *emitter) emitRoutes(prog *ssa.Program, roots []*ssa.Package, models *RouteModels) {
	engine := &routeEngine{
		e:            e,
		prog:         prog,
		models:       newRouteModelIndex(models),
		objectByKey:  make(map[string]int),
		fields:       make(map[string]absValue),
		relevant:     make(map[*ssa.Function]bool),
		direct:       make(map[*ssa.Function]bool),
		implementers: make(map[string][]*ssa.Function),
		onStack:      make(map[*ssa.Function]bool),
		interpreted:  make(map[*ssa.Function]bool),
		callIndexes:  e.callIndexes,
	}
	var functions []*ssa.Function
	seen := make(map[*ssa.Function]bool)
	for _, pkg := range roots {
		if pkg == nil || pkg.Pkg == nil {
			continue
		}
		for _, fn := range ssaFunctions(pkg) {
			if !seen[fn] {
				seen[fn] = true
				functions = append(functions, fn)
			}
		}
	}
	sort.SliceStable(functions, func(i, j int) bool { return functionName(functions[i]) < functionName(functions[j]) })
	engine.index(functions)
	if len(engine.direct) == 0 {
		return
	}
	// Package initializers run first (they call the package's init functions
	// and set its variables), then every program's main. Each program starts
	// from what the initializers stored: two programs are two processes, so a
	// router one of them stores in a field is not in the other's.
	for _, fn := range functions {
		if fn.Synthetic == "package initializer" && engine.relevant[fn] {
			engine.interpret(fn, nil, nil, "")
		}
	}
	initialized := engine.fields
	for _, fn := range functions {
		if fn.Parent() == nil && fn.Signature.Recv() == nil && fn.Name() == "main" && fn.Pkg != nil && fn.Pkg.Pkg.Name() == "main" && engine.relevant[fn] {
			engine.fields = copyFields(initialized)
			engine.interpret(fn, nil, nil, "")
		}
	}
	// A registration no program reaches runs on its own, from the same start.
	for _, fn := range functions {
		if engine.direct[fn] && !engine.interpreted[fn] {
			engine.fields = copyFields(initialized)
			engine.interpretOpen(fn)
		}
	}
	engine.emit()
}

// index finds the functions with model calls, the functions that reach them
// through static calls or closures they create, and the concrete methods an
// interface call could enter.
func (engine *routeEngine) index(functions []*ssa.Function) {
	callers := make(map[*ssa.Function][]*ssa.Function)
	inProgram := make(map[*ssa.Function]bool, len(functions))
	for _, fn := range functions {
		inProgram[fn] = true
	}
	for _, fn := range functions {
		if fn.Signature.Recv() != nil && fn.Synthetic == "" {
			engine.implementers[fn.Name()] = append(engine.implementers[fn.Name()], fn)
		}
		// A wrapper SSA synthesizes (a promoted method, a bound method value) has no
		// source of its own: what it calls is the call written where it is used.
		if fn.Synthetic != "" && fn.Synthetic != "package initializer" {
			continue
		}
		for _, block := range fn.Blocks {
			for _, instr := range block.Instrs {
				if call, ok := instr.(ssa.CallInstruction); ok {
					common := call.Common()
					if model, _ := engine.models.match(common, common.StaticCallee()); model != nil {
						engine.direct[fn] = true
					}
					if callee := common.StaticCallee(); callee != nil && inProgram[callee] {
						callers[callee] = append(callers[callee], fn)
					}
				}
				for _, operand := range instr.Operands(nil) {
					if operand == nil {
						continue
					}
					if function, ok := (*operand).(*ssa.Function); ok && inProgram[function] {
						callers[function] = append(callers[function], fn)
					}
				}
			}
		}
	}
	var work []*ssa.Function
	for _, fn := range functions {
		if engine.direct[fn] {
			engine.relevant[fn] = true
			work = append(work, fn)
		}
	}
	for len(work) > 0 {
		fn := work[len(work)-1]
		work = work[:len(work)-1]
		for _, caller := range callers[fn] {
			if !engine.relevant[caller] {
				engine.relevant[caller] = true
				work = append(work, caller)
			}
		}
		// A closure is relevant through the function that creates it.
		if parent := fn.Parent(); parent != nil && !engine.relevant[parent] {
			engine.relevant[parent] = true
			work = append(work, parent)
		}
	}
}

func (engine *routeEngine) interpretOpen(fn *ssa.Function) {
	args := make([]absValue, len(fn.Params))
	for i, param := range fn.Params {
		if engine.isRouterType(param.Type()) {
			args[i] = absValue{routers: []int{engine.openObject("param:" + functionName(fn) + "#" + strconv.Itoa(i))}}
		}
	}
	free := make([]absValue, len(fn.FreeVars))
	for i, variable := range fn.FreeVars {
		if engine.isRouterType(variable.Type()) {
			free[i] = absValue{routers: []int{engine.openObject("free:" + functionName(fn) + "#" + strconv.Itoa(i))}}
		}
	}
	engine.interpret(fn, args, free, "open:"+functionName(fn))
}

func copyFields(fields map[string]absValue) map[string]absValue {
	out := make(map[string]absValue, len(fields))
	for key, value := range fields {
		out[key] = value
	}
	return out
}

func (engine *routeEngine) isRouterType(typ types.Type) bool {
	return engine.models.routerTypes[namedTypeName(typ)]
}

func (engine *routeEngine) openObject(key string) int {
	key = "open:" + key
	if id, ok := engine.objectByKey[key]; ok {
		return id
	}
	object := &routerObject{
		id:          len(engine.objects),
		key:         key,
		prefix:      routePath{complete: false},
		open:        true,
		parent:      -1,
		mountParent: -1,
	}
	engine.objects = append(engine.objects, object)
	engine.objectByKey[key] = object.id
	return object.id
}

func (engine *routeEngine) newObject(framework, key string, parent int, prefix routePath, middleware []routeFunc) int {
	if id, ok := engine.objectByKey[key]; ok {
		// The same creation site in the same calling context: the same router.
		return id
	}
	object := &routerObject{
		id:          len(engine.objects),
		key:         key,
		framework:   framework,
		prefix:      prefix,
		middleware:  append([]routeFunc{}, middleware...),
		parent:      parent,
		mountParent: -1,
	}
	if parent >= 0 {
		object.open = engine.objects[parent].open
		if object.framework == "" {
			object.framework = engine.objects[parent].framework
		}
	}
	engine.objects = append(engine.objects, object)
	engine.objectByKey[key] = object.id
	return object.id
}

// frame is one function being interpreted in one calling context.
type frame struct {
	engine *routeEngine
	fn     *ssa.Function
	ctx    string
	env    map[ssa.Value]absValue
	locals map[ssa.Value]absValue
	ret    absValue
}

func (engine *routeEngine) interpret(fn *ssa.Function, args, free []absValue, ctx string) absValue {
	if fn == nil || len(fn.Blocks) == 0 || engine.exhausted || engine.onStack[fn] || engine.depth >= routeDepthLimit {
		return absValue{}
	}
	engine.onStack[fn] = true
	engine.interpreted[fn] = true
	engine.depth++
	defer func() {
		engine.depth--
		delete(engine.onStack, fn)
	}()
	fr := &frame{
		engine: engine,
		fn:     fn,
		ctx:    ctx,
		env:    make(map[ssa.Value]absValue),
		locals: make(map[ssa.Value]absValue),
	}
	for i, param := range fn.Params {
		if i < len(args) {
			fr.env[param] = args[i]
		}
	}
	for i, variable := range fn.FreeVars {
		if i < len(free) {
			fr.env[variable] = free[i]
		}
	}
	for _, block := range fn.Blocks {
		for _, instr := range block.Instrs {
			engine.steps++
			if engine.steps > routeStepBudget {
				engine.exhausted = true
				return fr.ret
			}
			fr.step(instr)
		}
	}
	return fr.ret
}

func (fr *frame) value(v ssa.Value) absValue {
	if v == nil {
		return absValue{}
	}
	if known, ok := fr.env[v]; ok {
		return known
	}
	switch v := v.(type) {
	case *ssa.Const:
		if v.Value != nil && v.Value.Kind() == constant.String {
			text := constant.StringVal(v.Value)
			return absValue{text: &text}
		}
	case *ssa.Function:
		return absValue{funcs: []callable{fr.engine.functionCallable(v, nil)}}
	case *ssa.Global:
		return fr.engine.fields[globalKey(v)]
	}
	return absValue{}
}

func (fr *frame) step(instr ssa.Instruction) {
	switch in := instr.(type) {
	case *ssa.Call:
		fr.env[in] = fr.call(in, &in.Call)
	case *ssa.Go:
		fr.call(in, &in.Call)
	case *ssa.Defer:
		fr.call(in, &in.Call)
	case *ssa.Store:
		fr.store(in.Addr, fr.value(in.Val))
	case *ssa.UnOp:
		if in.Op == token.MUL {
			fr.env[in] = fr.load(in.X)
		}
	case *ssa.FieldAddr:
		fr.env[in] = fr.embeddedRouter(in.X, in.Field)
	case *ssa.Field:
		fr.env[in] = fr.embeddedRouter(in.X, in.Field).join(fr.fieldValue(in.X.Type(), in.Field))
	case *ssa.Phi:
		var merged absValue
		for _, edge := range in.Edges {
			merged = merged.join(fr.value(edge))
		}
		fr.env[in] = merged
	case *ssa.MakeInterface:
		fr.env[in] = fr.value(in.X)
	case *ssa.ChangeType:
		fr.env[in] = fr.value(in.X)
	case *ssa.ChangeInterface:
		fr.env[in] = fr.value(in.X)
	case *ssa.Convert:
		fr.env[in] = fr.value(in.X)
	case *ssa.TypeAssert:
		fr.env[in] = fr.value(in.X)
	case *ssa.Extract:
		fr.env[in] = fr.value(in.Tuple)
	case *ssa.Slice:
		fr.env[in] = fr.value(in.X)
	case *ssa.BinOp:
		if in.Op == token.ADD {
			left, right := fr.value(in.X), fr.value(in.Y)
			if left.text != nil && right.text != nil {
				text := *left.text + *right.text
				fr.env[in] = absValue{text: &text}
			}
		}
	case *ssa.MakeClosure:
		fn, _ := in.Fn.(*ssa.Function)
		bindings := make([]absValue, len(in.Bindings))
		for i, binding := range in.Bindings {
			bindings[i] = fr.value(binding)
		}
		fr.env[in] = absValue{funcs: []callable{fr.engine.functionCallable(fn, bindings)}}
	case *ssa.Return:
		for _, result := range in.Results {
			fr.ret = fr.ret.join(fr.value(result))
		}
	}
}

// stableHashString is a short deterministic digest of a calling context.
func stableHashString(text string) string {
	hash := fnv.New64a()
	hash.Write([]byte(text))
	return strconv.FormatUint(hash.Sum64(), 36)
}

func globalKey(global *ssa.Global) string {
	if global.Pkg != nil && global.Pkg.Pkg != nil {
		return "global:" + global.Pkg.Pkg.Path() + "." + global.Name()
	}
	return "global:" + global.Name()
}

// fieldKey names the one location the interpreter keeps for a field of a
// named struct type, or "" for an anonymous struct.
func fieldKey(structType types.Type, index int) (string, *types.Var) {
	if pointer, ok := types.Unalias(structType).(*types.Pointer); ok {
		structType = pointer.Elem()
	}
	structure, ok := structType.Underlying().(*types.Struct)
	if !ok || index >= structure.NumFields() {
		return "", nil
	}
	owner := namedTypeName(structType)
	if owner == "" {
		return "", structure.Field(index)
	}
	return "field:" + owner + "." + structure.Field(index).Name(), structure.Field(index)
}

func (fr *frame) fieldValue(structType types.Type, index int) absValue {
	key, field := fieldKey(structType, index)
	if key == "" {
		return absValue{}
	}
	stored := fr.engine.fields[key]
	if field == nil || !isFunctionLike(field.Type()) {
		return stored
	}
	label := strings.TrimPrefix(key, "field:")
	if len(stored.funcs) == 0 {
		return stored.join(absValue{funcs: []callable{{ref: routeFunc{Name: label, Kind: "field", Field: label}}}})
	}
	out := stored
	out.funcs = make([]callable, len(stored.funcs))
	for i, function := range stored.funcs {
		function.ref.Field = label
		out.funcs[i] = function
	}
	return out
}

// embeddedRouter is the router a field selection still is: selecting the
// embedded router of a router (gin's `Engine.RouterGroup`) does not leave it.
func (fr *frame) embeddedRouter(x ssa.Value, index int) absValue {
	_, field := fieldKey(x.Type(), index)
	if field == nil || !field.Embedded() || !fr.engine.isRouterType(field.Type()) {
		return absValue{}
	}
	return absValue{routers: fr.value(x).routers}
}

func (fr *frame) load(addr ssa.Value) absValue {
	switch address := addr.(type) {
	case *ssa.FieldAddr:
		return fr.value(address).join(fr.fieldValue(address.X.Type(), address.Field))
	case *ssa.Global:
		return fr.engine.fields[globalKey(address)]
	case *ssa.Alloc:
		return fr.locals[address]
	}
	return absValue{}
}

func (fr *frame) store(addr ssa.Value, value absValue) {
	switch address := addr.(type) {
	case *ssa.FieldAddr:
		if key, _ := fieldKey(address.X.Type(), address.Field); key != "" {
			fr.engine.fields[key] = fr.engine.fields[key].join(value)
		}
	case *ssa.Global:
		key := globalKey(address)
		fr.engine.fields[key] = fr.engine.fields[key].join(value)
	case *ssa.Alloc:
		fr.locals[address] = value
	}
}

func isFunctionLike(typ types.Type) bool {
	switch types.Unalias(typ).Underlying().(type) {
	case *types.Signature, *types.Interface:
		return true
	}
	return false
}

func (engine *routeEngine) functionCallable(fn *ssa.Function, bindings []absValue) callable {
	if fn == nil {
		return callable{ref: routeFunc{Kind: "unknown"}}
	}
	declared := declaredCallee(engine.prog, fn)
	kind := "function"
	if declared.Parent() != nil {
		kind = "literal"
	}
	return callable{ref: routeFunc{Name: functionName(declared), Kind: kind}, fn: fn, bindings: bindings}
}

// siteKey identifies a creation site in this calling context.
func (fr *frame) siteKey(instr ssa.Instruction) string {
	return fr.engine.e.positionKey(instr.Pos()) + "@" + fr.ctx
}

func (fr *frame) call(instr ssa.CallInstruction, common *ssa.CallCommon) absValue {
	callee := common.StaticCallee()
	var receiver ssa.Value
	params := common.Args
	switch {
	case common.IsInvoke():
		receiver = common.Value
	case callee != nil && callee.Signature.Recv() != nil && len(common.Args) > 0:
		receiver = common.Args[0]
		params = common.Args[1:]
	}
	if model, name := fr.engine.models.match(common, callee); model != nil {
		return fr.apply(model, name, instr, receiver, params)
	}
	childCtx := stableHashString(fr.ctx + "|" + fr.engine.e.positionKey(instr.Pos()))
	switch {
	case callee != nil:
		if len(callee.Blocks) > 0 && (fr.engine.relevant[callee] || fr.carriesRouter(common.Args)) {
			return fr.engine.interpret(callee, fr.values(common.Args), nil, childCtx)
		}
		return fr.engine.producedBy(callee.Signature, functionName(declaredCallee(fr.engine.prog, callee)))
	case common.IsInvoke():
		args := append([]ssa.Value{common.Value}, common.Args...)
		var out absValue
		if fr.carriesRouter(args) {
			for _, method := range fr.engine.implementations(common) {
				out = out.join(fr.engine.interpret(method, fr.values(args), nil, childCtx))
			}
		}
		if common.Method != nil {
			if signature, ok := common.Method.Type().(*types.Signature); ok {
				out = out.join(fr.engine.producedBy(signature, interfaceMethodName(instr)))
			}
		}
		return out
	default:
		var out absValue
		for _, function := range fr.value(common.Value).funcs {
			if function.fn != nil && len(function.fn.Blocks) > 0 && (fr.engine.relevant[function.fn] || fr.carriesRouter(common.Args)) {
				out = out.join(fr.engine.interpret(function.fn, fr.values(common.Args), function.bindings, childCtx))
			}
		}
		return out
	}
}

// producedBy is what a call the interpreter does not enter returns: when the
// result is a function or an interface value, the function that produced it.
func (engine *routeEngine) producedBy(signature *types.Signature, name string) absValue {
	if signature == nil || signature.Results().Len() == 0 || name == "" {
		return absValue{}
	}
	if !isFunctionLike(signature.Results().At(0).Type()) {
		return absValue{}
	}
	return absValue{funcs: []callable{{ref: routeFunc{Name: name, Kind: "factory"}}}}
}

func (fr *frame) values(values []ssa.Value) []absValue {
	out := make([]absValue, len(values))
	for i, value := range values {
		out[i] = fr.value(value)
	}
	return out
}

func (fr *frame) carriesRouter(values []ssa.Value) bool {
	for _, value := range values {
		if len(fr.value(value).routers) > 0 {
			return true
		}
	}
	return false
}

// implementations are the program's concrete methods an interface call could
// enter: the methods of that name whose receiver implements the interface.
func (engine *routeEngine) implementations(common *ssa.CallCommon) []*ssa.Function {
	if common.Method == nil {
		return nil
	}
	iface, ok := common.Value.Type().Underlying().(*types.Interface)
	if !ok {
		return nil
	}
	var out []*ssa.Function
	for _, method := range engine.implementers[common.Method.Name()] {
		if types.Implements(method.Signature.Recv().Type(), iface) {
			out = append(out, method)
		}
	}
	return out
}

func argument(params []ssa.Value, index *int) ssa.Value {
	if index == nil || *index < 0 || *index >= len(params) {
		return nil
	}
	return params[*index]
}

func (fr *frame) apply(model *RouteModel, name string, instr ssa.CallInstruction, receiver ssa.Value, params []ssa.Value) absValue {
	engine := fr.engine
	switch model.Role {
	case "router":
		var middleware []routeFunc
		for _, function := range model.InitialMiddleware {
			middleware = append(middleware, routeFunc{Name: function, Kind: "function"})
		}
		id := engine.newObject(model.Framework, fr.siteKey(instr), -1, knownPath(""), middleware)
		return absValue{routers: []int{id}}
	case "passthrough":
		return fr.value(argument(params, model.Argument))
	}
	routers := fr.routersOf(model, receiver, params)
	switch model.Role {
	case "route":
		method, path := fr.methodAndPath(model, name, params)
		handlers, inline := fr.handlerArguments(model, params)
		for _, id := range routers {
			engine.record(fr, instr, model, id, method, path, "", handlers, inline)
		}
		if model.ReturnsReceiver {
			return absValue{routers: routers}
		}
	case "group":
		path := knownPath("")
		if model.PathArgument != nil {
			path = fr.pathOf(argument(params, model.PathArgument))
		}
		middleware := fr.functionArguments(params, model.MiddlewareFrom)
		var children []int
		for _, parent := range routers {
			object := engine.objects[parent]
			child := engine.newObject(object.framework, fr.siteKey(instr)+"<"+object.key, parent, object.prefix.join(path), append(append([]routeFunc{}, object.middleware...), middleware...))
			children = append(children, child)
		}
		if model.CallbackArgument != nil {
			childCtx := stableHashString(fr.ctx + "|" + engine.e.positionKey(instr.Pos()))
			for _, function := range fr.value(argument(params, model.CallbackArgument)).funcs {
				if function.fn != nil {
					engine.interpret(function.fn, []absValue{{routers: children}}, function.bindings, childCtx)
				}
			}
		}
		return absValue{routers: children}
	case "use":
		middleware := fr.functionArguments(params, model.MiddlewareFrom)
		for _, id := range routers {
			engine.objects[id].middleware = append(engine.objects[id].middleware, middleware...)
		}
		if model.ReturnsReceiver {
			return absValue{routers: routers}
		}
	case "mount":
		path := fr.pathOf(argument(params, model.PathArgument))
		for _, child := range fr.value(argument(params, model.HandlerArgument)).routers {
			for _, parent := range routers {
				if child == parent {
					continue
				}
				object := engine.objects[child]
				object.mountParent = parent
				object.mountPrefix = engine.objects[parent].prefix.join(path)
				object.mountMiddleware = append([]routeFunc{}, engine.objects[parent].middleware...)
			}
		}
	case "subscriber":
		topic := fr.pathOf(argument(params, model.TopicArgument))
		handlerName := ""
		if text := fr.value(argument(params, model.NameArgument)).text; text != nil {
			handlerName = *text
		}
		handlers := fr.identities(argument(params, model.HandlerArgument))
		for _, id := range routers {
			engine.record(fr, instr, model, id, "", topic, handlerName, handlers, nil)
		}
	case "serve":
		for _, id := range routers {
			engine.serve(fr, instr, id)
		}
	}
	return absValue{}
}

// routersOf is the router a registration acts on: its receiver, its router
// argument, or the framework's implicit default router. A router the
// interpreter never saw created is an open router named by where it came
// from.
func (fr *frame) routersOf(model *RouteModel, receiver ssa.Value, params []ssa.Value) []int {
	source := receiver
	if model.RouterArgument != nil {
		source = argument(params, model.RouterArgument)
	}
	if source == nil {
		if model.DefaultRouter == "" {
			return nil
		}
		key := "default:" + model.DefaultRouter
		return []int{fr.engine.newObject(model.Framework, key, -1, knownPath(""), nil)}
	}
	if routers := fr.value(source).routers; len(routers) > 0 {
		return routers
	}
	return []int{fr.engine.openObject(fr.describe(source))}
}

// describe names where a value came from, for an open router's identity.
func (fr *frame) describe(value ssa.Value) string {
	switch v := value.(type) {
	case *ssa.Parameter:
		for i, param := range fr.fn.Params {
			if param == v {
				return "param:" + functionName(fr.fn) + "#" + strconv.Itoa(i)
			}
		}
	case *ssa.FreeVar:
		for i, variable := range fr.fn.FreeVars {
			if variable == v {
				return "free:" + functionName(fr.fn) + "#" + strconv.Itoa(i)
			}
		}
	case *ssa.UnOp:
		if field, ok := v.X.(*ssa.FieldAddr); ok {
			if key, _ := fieldKey(field.X.Type(), field.Field); key != "" {
				return key
			}
		}
		if global, ok := v.X.(*ssa.Global); ok {
			return globalKey(global)
		}
	case *ssa.Field:
		if key, _ := fieldKey(v.X.Type(), v.Field); key != "" {
			return key
		}
	case *ssa.FieldAddr:
		return fr.describe(v.X)
	}
	return "value:" + functionName(fr.fn) + ":" + fr.engine.e.positionKey(value.Pos())
}

func (fr *frame) pathOf(value ssa.Value) routePath {
	if value == nil {
		return knownPath("")
	}
	if text := fr.value(value).text; text != nil {
		return knownPath(*text)
	}
	return routePath{text: unknownPathPart, complete: false}
}

func (fr *frame) methodAndPath(model *RouteModel, name string, params []ssa.Value) (string, routePath) {
	path := fr.pathOf(argument(params, model.PathArgument))
	method := "*"
	switch {
	case model.HTTPMethod != "":
		method = model.HTTPMethod
	case model.MethodFromName:
		method = strings.ToUpper(name)
	case model.MethodArgument != nil:
		method = "?"
		if text := fr.value(argument(params, model.MethodArgument)).text; text != nil {
			method = strings.ToUpper(*text)
		}
	case model.MethodInPattern && path.complete:
		if verb, rest, ok := strings.Cut(path.text, " "); ok && verb != "" && !strings.Contains(verb, "/") {
			method = strings.ToUpper(verb)
			path = knownPath(strings.TrimSpace(rest))
		}
	}
	return method, path
}

// handlerArguments reads a registration's handler and inline middleware: the
// handler argument, or the variadic tail whose last element is the handler.
func (fr *frame) handlerArguments(model *RouteModel, params []ssa.Value) ([]routeFunc, []routeFunc) {
	if model.HandlerArgument != nil {
		return fr.identities(argument(params, model.HandlerArgument)), nil
	}
	if model.HandlersFrom == nil {
		return nil, nil
	}
	elements, known := fr.variadic(params, *model.HandlersFrom)
	if !known {
		return []routeFunc{{Kind: "unknown"}}, []routeFunc{{Kind: "unknown"}}
	}
	if len(elements) == 0 {
		return nil, nil
	}
	var inline []routeFunc
	for _, element := range elements[:len(elements)-1] {
		inline = append(inline, fr.identities(element)...)
	}
	return fr.identities(elements[len(elements)-1]), inline
}

func (fr *frame) functionArguments(params []ssa.Value, from *int) []routeFunc {
	if from == nil {
		return nil
	}
	elements, known := fr.variadic(params, *from)
	if !known {
		return []routeFunc{{Kind: "unknown"}}
	}
	var out []routeFunc
	for _, element := range elements {
		out = append(out, fr.identities(element)...)
	}
	return out
}

// variadic returns the arguments from position `from` on, unpacking the
// slice SSA builds for a variadic call. A slice the caller built elsewhere
// (`handlers...`) is not known.
func (fr *frame) variadic(params []ssa.Value, from int) ([]ssa.Value, bool) {
	if from < 0 || from >= len(params) {
		return nil, true
	}
	if from < len(params)-1 {
		return params[from:], true
	}
	last := params[from]
	if _, ok := last.Type().Underlying().(*types.Slice); !ok {
		return params[from:], true
	}
	if constant, ok := last.(*ssa.Const); ok && constant.IsNil() {
		return nil, true
	}
	slice, ok := last.(*ssa.Slice)
	if !ok {
		return nil, false
	}
	alloc, ok := slice.X.(*ssa.Alloc)
	if !ok {
		return nil, false
	}
	array, ok := types.Unalias(alloc.Type()).(*types.Pointer).Elem().Underlying().(*types.Array)
	if !ok {
		return nil, false
	}
	elements := make([]ssa.Value, array.Len())
	for _, referrer := range *alloc.Referrers() {
		address, ok := referrer.(*ssa.IndexAddr)
		if !ok || address.X != alloc {
			continue
		}
		index, ok := address.Index.(*ssa.Const)
		if !ok {
			return nil, false
		}
		position, exact := constant.Int64Val(index.Value)
		if !exact || position < 0 || position >= int64(len(elements)) {
			return nil, false
		}
		for _, use := range *address.Referrers() {
			if store, ok := use.(*ssa.Store); ok && store.Addr == address {
				elements[position] = store.Val
			}
		}
	}
	for _, element := range elements {
		if element == nil {
			return nil, false
		}
	}
	return elements, true
}

// identities names what a handler or middleware argument is.
func (fr *frame) identities(value ssa.Value) []routeFunc {
	if value == nil {
		return nil
	}
	known := fr.value(value).funcs
	if len(known) == 0 {
		return []routeFunc{{Kind: "unknown"}}
	}
	out := make([]routeFunc, 0, len(known))
	for _, function := range known {
		out = append(out, function.ref)
	}
	return out
}

func (engine *routeEngine) site(fr *frame, instr ssa.CallInstruction) (string, *Span, string) {
	index := engine.callIndexes[fr.fn]
	if index == nil {
		index = newCallIndex(fr.fn.Syntax())
		engine.callIndexes[fr.fn] = index
	}
	if syntax := index.syntaxFor(instr); syntax != nil {
		if span := engine.e.positionSpan(syntax.Pos(), syntax.End()); span != nil {
			return posFile(engine.e.fset, syntax.Pos(), engine.e.root), span, engine.e.positionKey(syntax.Pos())
		}
	}
	if span := engine.e.positionSpan(instr.Pos(), instr.Pos()); span != nil {
		return posFile(engine.e.fset, instr.Pos(), engine.e.root), span, engine.e.positionKey(instr.Pos())
	}
	return "", nil, engine.e.positionKey(instr.Pos())
}

func (engine *routeEngine) record(fr *frame, instr ssa.CallInstruction, model *RouteModel, router int, method string, path routePath, name string, handlers, inline []routeFunc) {
	object := engine.objects[router]
	transport := "http"
	full := object.prefix.join(path)
	if model.Role == "subscriber" {
		transport = "message"
		full = path
	}
	middleware := append(append([]routeFunc{}, object.middleware...), inline...)
	complete := !object.open && !containsUnknown(middleware)
	file, span, position := engine.site(fr, instr)
	engine.routes = append(engine.routes, routeRecord{
		framework:  model.Framework,
		transport:  transport,
		method:     method,
		path:       full,
		registered: path,
		name:       name,
		handlers:   handlers,
		middleware: middleware,
		complete:   complete,
		router:     router,
		function:   functionName(fr.fn),
		file:       file,
		span:       span,
		position:   position,
	})
}

func containsUnknown(functions []routeFunc) bool {
	for _, function := range functions {
		if function.Kind == "unknown" {
			return true
		}
	}
	return false
}

func (engine *routeEngine) serve(fr *frame, instr ssa.CallInstruction, router int) {
	file, span, position := engine.site(fr, instr)
	engine.serves = append(engine.serves, serveRecord{
		router:   router,
		function: functionName(fr.fn),
		file:     file,
		span:     span,
		position: position,
	})
}

// root is the router a router was derived from, following groups and
// mounts, so a serve call on it dispatches to every route below it.
func (engine *routeEngine) root(id int) int {
	for steps := 0; steps < len(engine.objects); steps++ {
		object := engine.objects[id]
		switch {
		case object.mountParent >= 0:
			id = object.mountParent
		case object.parent >= 0:
			id = object.parent
		default:
			return id
		}
	}
	return id
}

// mounted is the prefix and middleware a router inherits from the routers it
// was mounted under.
func (engine *routeEngine) mounted(id int) (routePath, []routeFunc) {
	prefix := knownPath("")
	var middleware []routeFunc
	for steps := 0; steps < len(engine.objects); steps++ {
		object := engine.objects[id]
		if object.mountParent >= 0 {
			prefix = object.mountPrefix.join(prefix)
			middleware = append(append([]routeFunc{}, object.mountMiddleware...), middleware...)
			id = object.mountParent
			continue
		}
		if object.parent >= 0 {
			id = object.parent
			continue
		}
		break
	}
	return prefix, middleware
}

func sortedKeys(set map[string]bool) []string {
	keys := make([]string, 0, len(set))
	for key := range set {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	return keys
}

func routeFuncNames(functions []routeFunc) string {
	parts := make([]string, len(functions))
	for i, function := range functions {
		parts[i] = function.Kind + ":" + function.Name + ":" + function.Field
	}
	return strings.Join(parts, ",")
}

// emit writes one `route` row per registration call and what it registers.
//
// The same registration runs once per calling context (a constructor two
// programs call builds two routers), and with field values kept per field a
// registration can also act on routers another context built. Records that
// differ only in the router are one row listing every router and the routers
// they derive from, so a serve call on any of them reaches the route; records
// that differ in path or middleware stay separate rows.
func (engine *routeEngine) emit() {
	type keyed struct {
		key string
		row Row
	}
	type merged struct {
		route       routeRecord
		path        routePath
		middleware  []routeFunc
		pathOpen    bool
		routers     map[string]bool
		routerRoots map[string]bool
	}
	var order []string
	byKey := make(map[string]*merged)
	for _, route := range engine.routes {
		object := engine.objects[route.router]
		path, middleware := route.path, route.middleware
		if route.transport == "http" {
			prefix, inherited := engine.mounted(route.router)
			path = prefix.join(path)
			middleware = append(inherited, middleware...)
		}
		pathComplete := path.complete && !object.open
		key := stableKey("route", route.position, route.method, path.text, route.registered.text, strconv.FormatBool(pathComplete), route.name, routeFuncNames(route.handlers), routeFuncNames(middleware), strconv.FormatBool(route.complete))
		entry := byKey[key]
		if entry == nil {
			entry = &merged{route: route, path: path, middleware: middleware, pathOpen: !pathComplete, routers: make(map[string]bool), routerRoots: make(map[string]bool)}
			byKey[key] = entry
			order = append(order, key)
		}
		entry.routers[object.key] = true
		entry.routerRoots[engine.objects[engine.root(route.router)].key] = true
	}
	var rows []keyed
	for _, key := range order {
		entry := byKey[key]
		route := entry.route
		handlers, middleware := route.handlers, entry.middleware
		if handlers == nil {
			handlers = []routeFunc{}
		}
		if middleware == nil {
			middleware = []routeFunc{}
		}
		row := Row{
			"kind":                "route",
			"framework":           route.framework,
			"transport":           route.transport,
			"method":              route.method,
			"path":                entry.path.text,
			"registered_path":     route.registered.text,
			"path_complete":       !entry.pathOpen,
			"handlers":            handlers,
			"middleware":          middleware,
			"middleware_complete": route.complete,
			"routers":             sortedKeys(entry.routers),
			"router_roots":        sortedKeys(entry.routerRoots),
			"function":            route.function,
			"stable_key":          key,
		}
		if route.name != "" {
			row["name"] = route.name
		}
		if route.file != "" {
			row["file"] = route.file
			row["span"] = route.span
		}
		rows = append(rows, keyed{key: route.file + "\x00" + key, row: row})
	}
	serveRoots := make(map[string]map[string]bool)
	var serveOrder []string
	serveByKey := make(map[string]serveRecord)
	for _, call := range engine.serves {
		key := stableKey("route_serve", call.position, call.function)
		if _, seen := serveByKey[key]; !seen {
			serveByKey[key] = call
			serveRoots[key] = make(map[string]bool)
			serveOrder = append(serveOrder, key)
		}
		serveRoots[key][engine.objects[engine.root(call.router)].key] = true
	}
	for _, key := range serveOrder {
		call := serveByKey[key]
		row := Row{
			"kind":         "route_serve",
			"function":     call.function,
			"router_roots": sortedKeys(serveRoots[key]),
			"stable_key":   key,
		}
		if call.file != "" {
			row["file"] = call.file
			row["span"] = call.span
		}
		rows = append(rows, keyed{key: call.file + "\x00" + key, row: row})
	}
	sort.SliceStable(rows, func(i, j int) bool { return rows[i].key < rows[j].key })
	for _, row := range rows {
		engine.e.add(row.row)
	}
	if engine.exhausted {
		engine.e.add(Row{
			"kind":       "route_budget",
			"steps":      engine.steps,
			"stable_key": stableKey("route_budget"),
		})
	}
}
