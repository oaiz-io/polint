package semantic

import (
	"fmt"
	"sort"
	"strings"
	"testing"
)

const ginStub = `package gin

import "net/http"

type Context struct{}

type HandlerFunc func(*Context)

type HandlersChain []HandlerFunc

type IRoutes interface {
	Use(...HandlerFunc) IRoutes
	Handle(string, string, ...HandlerFunc) IRoutes
	GET(string, ...HandlerFunc) IRoutes
	POST(string, ...HandlerFunc) IRoutes
}

type IRouter interface {
	IRoutes
	Group(string, ...HandlerFunc) *RouterGroup
}

type RouterGroup struct {
	Handlers HandlersChain
	basePath string
}

func (group *RouterGroup) Use(middleware ...HandlerFunc) IRoutes {
	group.Handlers = append(group.Handlers, middleware...)
	return group
}

func (group *RouterGroup) Group(path string, handlers ...HandlerFunc) *RouterGroup {
	return &RouterGroup{Handlers: append(append(HandlersChain{}, group.Handlers...), handlers...), basePath: group.basePath + path}
}

func (group *RouterGroup) Handle(method, path string, handlers ...HandlerFunc) IRoutes { return group }

func (group *RouterGroup) GET(path string, handlers ...HandlerFunc) IRoutes { return group }

func (group *RouterGroup) POST(path string, handlers ...HandlerFunc) IRoutes { return group }

type Engine struct{ RouterGroup }

func New() *Engine { return &Engine{} }

func Default() *Engine {
	engine := New()
	engine.Use(Logger(), Recovery())
	return engine
}

func (engine *Engine) Use(middleware ...HandlerFunc) IRoutes {
	engine.RouterGroup.Use(middleware...)
	return engine
}

func (engine *Engine) ServeHTTP(w http.ResponseWriter, r *http.Request) {}

func Logger() HandlerFunc { return func(*Context) {} }

func Recovery() HandlerFunc { return func(*Context) {} }
`

const watermillStub = `package message

type Message struct{}

type NoPublishHandlerFunc func(msg *Message) error

type Subscriber interface{}

type RouterConfig struct{}

type Handler struct{}

type Router struct{}

func NewRouter(config RouterConfig, logger interface{}) (*Router, error) { return &Router{}, nil }

func (r *Router) AddConsumerHandler(handlerName string, subscribeTopic string, subscriber Subscriber, handlerFunc NoPublishHandlerFunc) *Handler {
	return &Handler{}
}
`

var routeFixture = map[string]string{
	"go.mod": `module example.test/routes

go 1.24

require (
	github.com/ThreeDotsLabs/watermill v1.0.0
	github.com/gin-gonic/gin v1.0.0
)

replace github.com/gin-gonic/gin => ./stubs/gin

replace github.com/ThreeDotsLabs/watermill => ./stubs/watermill
`,
	"stubs/gin/go.mod":                  "module github.com/gin-gonic/gin\n\ngo 1.24\n",
	"stubs/gin/gin.go":                  ginStub,
	"stubs/watermill/go.mod":            "module github.com/ThreeDotsLabs/watermill\n\ngo 1.24\n",
	"stubs/watermill/message/router.go": watermillStub,
	"server/server.go": `package server

import "github.com/gin-gonic/gin"

type Base struct {
	Router *gin.Engine
	Auth   gin.HandlerFunc
}

func Authenticate() gin.HandlerFunc { return func(*gin.Context) {} }

func Budget() gin.HandlerFunc { return func(*gin.Context) {} }

func NewBase() *Base {
	router := gin.New()
	router.Use(Budget())
	return &Base{Router: router, Auth: Authenticate()}
}
`,
	"catalog/http.go": `package catalog

import (
	"github.com/gin-gonic/gin"

	"example.test/routes/server"
)

type HTTP struct {
	server.Base
}

func New(base string, srv server.Base) *HTTP {
	h := &HTTP{Base: srv}
	h.setup(base)
	return h
}

func (h HTTP) setup(base string) {
	h.Router.GET("/"+base, func(c *gin.Context) {})
	api := h.Router.Group("/" + base)
	api.Use(h.Auth)
	api.POST("/items", h.create)
	register(api.Group("/admin", Admin()))
	h.Router.Use(Late())
	h.Router.GET("/late", h.list)
	api.GET("/after", wrap(h.list))
}

func register(group *gin.RouterGroup) { group.GET("/stats", stats) }

func wrap(handler gin.HandlerFunc) gin.HandlerFunc { return handler }

func (h HTTP) create(c *gin.Context) {}

func (h HTTP) list(c *gin.Context) {}

func stats(c *gin.Context) {}

func Admin() gin.HandlerFunc { return func(*gin.Context) {} }

func Late() gin.HandlerFunc { return func(*gin.Context) {} }
`,
	"orphan/orphan.go": `package orphan

import "github.com/gin-gonic/gin"

func Register(router gin.IRouter) { router.GET("/orphan", handle) }

func handle(c *gin.Context) {}
`,
	"events/events.go": `package events

import "github.com/ThreeDotsLabs/watermill/message"

type Subscriber struct{ router *message.Router }

func NewSubscriber() *Subscriber {
	router, _ := message.NewRouter(message.RouterConfig{}, nil)
	s := &Subscriber{router: router}
	s.router.AddConsumerHandler("item-created", "items.created", nil, s.onItemCreated)
	return s
}

func (s *Subscriber) onItemCreated(msg *message.Message) error { return nil }
`,
	"stdlib/stdlib.go": `package stdlib

import "net/http"

func Serve() {
	http.HandleFunc("GET /health", health)
	mux := http.NewServeMux()
	mux.HandleFunc("/items/{id}", item)
}

func health(w http.ResponseWriter, r *http.Request) {}

func item(w http.ResponseWriter, r *http.Request) {}
`,
	"probe/probe.go": `package probe

import "github.com/gin-gonic/gin"

func Exercise() {
	engine := gin.Default()
	engine.GET("/ping", ping)
	engine.ServeHTTP(nil, nil)
}

func ping(c *gin.Context) {}
`,
	"cmd/app/main.go": `package main

import (
	"example.test/routes/catalog"
	"example.test/routes/events"
	"example.test/routes/server"
	"example.test/routes/stdlib"
)

func main() {
	base := server.NewBase()
	catalog.New("catalog", *base)
	events.NewSubscriber()
	stdlib.Serve()
}
`,
}

func emitRouteFixture(t *testing.T, repository *RouteModels) []Row {
	t.Helper()
	models, err := WithBuiltinRouteModels(repository)
	if err != nil {
		t.Fatalf("route models: %v", err)
	}
	root := writeFixture(t, routeFixture)
	rows, err := Emit(Config{Root: root, ModuleRoots: []string{"."}, Patterns: []string{"./..."}, RouteModels: models})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}
	return rows
}

func intPointer(value int) *int { return &value }

func routeFuncsText(value interface{}) string {
	functions, _ := value.([]routeFunc)
	parts := make([]string, len(functions))
	for i, function := range functions {
		parts[i] = function.Kind + " " + function.Name
		if function.Field != "" {
			parts[i] += " via " + function.Field
		}
	}
	return strings.Join(parts, " | ")
}

// routeTable renders each route as one line: method, path, completeness,
// handlers and middleware.
func routeTable(rows []Row) []string {
	var lines []string
	for _, row := range rowsOfKind(rows, "route") {
		lines = append(lines, fmt.Sprintf("%s %s %s path_complete=%v middleware_complete=%v handlers=[%s] middleware=[%s]",
			row["transport"], row["method"], row["path"], row["path_complete"], row["middleware_complete"],
			routeFuncsText(row["handlers"]), routeFuncsText(row["middleware"])))
	}
	sort.Strings(lines)
	return lines
}

func TestRoutesFollowGroupsUseOrderFieldsHelpersAndContexts(t *testing.T) {
	wrap := RouteModels{Models: []RouteModel{{
		Framework: "local", Role: "passthrough", Function: "example.test/routes/catalog.wrap", Argument: intPointer(0),
	}}}
	got := routeTable(emitRouteFixture(t, &wrap))
	budget := "factory example.test/routes/server.Budget"
	auth := "factory example.test/routes/server.Authenticate via example.test/routes/server.Base.Auth"
	want := []string{
		"http * /items/{id} path_complete=true middleware_complete=true handlers=[function example.test/routes/stdlib.item] middleware=[]",
		"http GET /catalog path_complete=true middleware_complete=true handlers=[literal (example.test/routes/catalog.HTTP).setup$1] middleware=[" + budget + "]",
		"http GET /catalog/admin/stats path_complete=true middleware_complete=true handlers=[function example.test/routes/catalog.stats] middleware=[" + budget + " | " + auth + " | factory example.test/routes/catalog.Admin]",
		// A use call after a group was created does not reach the group.
		"http GET /catalog/after path_complete=true middleware_complete=true handlers=[function (example.test/routes/catalog.HTTP).list] middleware=[" + budget + " | " + auth + "]",
		"http GET /health path_complete=true middleware_complete=true handlers=[function example.test/routes/stdlib.health] middleware=[]",
		"http GET /late path_complete=true middleware_complete=true handlers=[function (example.test/routes/catalog.HTTP).list] middleware=[" + budget + " | factory example.test/routes/catalog.Late]",
		// A registration on a router nothing in the program passes is listed with
		// what is unknown about it marked.
		"http GET /orphan path_complete=false middleware_complete=false handlers=[function example.test/routes/orphan.handle] middleware=[]",
		"http GET /ping path_complete=true middleware_complete=true handlers=[function example.test/routes/probe.ping] middleware=[function github.com/gin-gonic/gin.Logger | function github.com/gin-gonic/gin.Recovery]",
		"http POST /catalog/items path_complete=true middleware_complete=true handlers=[function (example.test/routes/catalog.HTTP).create] middleware=[" + budget + " | " + auth + "]",
		"message  items.created path_complete=true middleware_complete=true handlers=[function (*example.test/routes/events.Subscriber).onItemCreated] middleware=[]",
	}
	sort.Strings(want)
	if strings.Join(got, "\n") != strings.Join(want, "\n") {
		t.Fatalf("routes:\n%s\n\nwant:\n%s", strings.Join(got, "\n"), strings.Join(want, "\n"))
	}
}

func TestRoutesWithoutAPassthroughModelNameTheWrapper(t *testing.T) {
	for _, line := range routeTable(emitRouteFixture(t, nil)) {
		if strings.Contains(line, " /catalog/after ") && !strings.Contains(line, "handlers=[factory example.test/routes/catalog.wrap]") {
			t.Fatalf("without a model the handler is what produced it: %s", line)
		}
	}
}

func TestServeCallsReachTheRoutesBelowTheirRouter(t *testing.T) {
	rows := emitRouteFixture(t, nil)
	var pingRoots []string
	for _, row := range rowsOfKind(rows, "route") {
		if row["path"] == "/ping" {
			pingRoots, _ = row["router_roots"].([]string)
		}
	}
	serves := rowsOfKind(rows, "route_serve")
	if len(serves) != 1 {
		t.Fatalf("want one serve row, got %#v", serves)
	}
	roots, _ := serves[0]["router_roots"].([]string)
	if serves[0]["function"] != "example.test/routes/probe.Exercise" || len(pingRoots) != 1 || fmt.Sprint(roots) != fmt.Sprint(pingRoots) {
		t.Fatalf("serve %#v, ping roots %q", serves[0], pingRoots)
	}
}

func TestRouteRowsAreDeterministicAndKeyed(t *testing.T) {
	first := emitRouteFixture(t, nil)
	second := emitRouteFixture(t, nil)
	keys := func(rows []Row) []string {
		var out []string
		for _, row := range rowsOfKind(rows, "route") {
			out = append(out, row["stable_key"].(string)+" "+fmt.Sprint(row["routers"]))
		}
		return out
	}
	if strings.Join(keys(first), "\n") != strings.Join(keys(second), "\n") {
		t.Fatalf("route rows differ between runs")
	}
	seen := make(map[string]bool)
	for _, row := range rowsOfKind(first, "route") {
		key := row["stable_key"].(string)
		if seen[key] {
			t.Fatalf("duplicate route key %s", key)
		}
		seen[key] = true
	}
}

// One registration reached from two calling contexts is one route listing
// both routers, not two rows that differ only in the router.
func TestARegistrationReachedTwiceIsOneRouteOnBothRouters(t *testing.T) {
	fixture := map[string]string{}
	for path, contents := range routeFixture {
		fixture[path] = contents
	}
	fixture["cmd/second/main.go"] = `package main

import (
	"example.test/routes/catalog"
	"example.test/routes/server"
)

func main() {
	base := server.NewBase()
	catalog.New("catalog", *base)
}
`
	models, err := WithBuiltinRouteModels(nil)
	if err != nil {
		t.Fatalf("route models: %v", err)
	}
	rows, err := Emit(Config{Root: writeFixture(t, fixture), ModuleRoots: []string{"."}, Patterns: []string{"./..."}, RouteModels: models})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}
	var items []Row
	for _, row := range rowsOfKind(rows, "route") {
		if row["path"] == "/catalog/items" {
			items = append(items, row)
		}
	}
	if len(items) != 1 {
		t.Fatalf("want one /catalog/items route, got %d: %#v", len(items), items)
	}
	if routers, _ := items[0]["routers"].([]string); len(routers) != 2 {
		t.Fatalf("want both programs' routers, got %#v", items[0]["routers"])
	}
}

func TestRouteModelsRejectUnknownRolesAndUnnamedCalls(t *testing.T) {
	if _, err := parseRouteModels([]byte(`{"models":[{"framework":"x","role":"teleport","function":"a.B"}]}`)); err == nil {
		t.Fatalf("an unknown role must be rejected")
	}
	if _, err := parseRouteModels([]byte(`{"models":[{"framework":"x","role":"route","methods":["GET"]}]}`)); err == nil {
		t.Fatalf("a model without a function or receivers must be rejected")
	}
	if _, err := BuiltinRouteModels(); err != nil {
		t.Fatalf("built-in models: %v", err)
	}
}

// A program that reaches its route setup only through a function value read
// from a table (a run mode) or through an interface call still registers
// complete routes: the setup is interpreted from `main`, not left open.
func TestRoutesReachedThroughFunctionValuesAndInterfaceCalls(t *testing.T) {
	fixture := map[string]string{}
	for path, contents := range routeFixture {
		fixture[path] = contents
	}
	fixture["cmd/app/main.go"] = `package main

import (
	"context"

	"example.test/routes/catalog"
	"example.test/routes/server"
)

type mode struct {
	name string
	run  func(context.Context) error
}

var modes = []mode{
	{name: "serve", run: serve},
	{name: "noop", run: func(context.Context) error { return nil }},
}

func lookup(name string) mode {
	for _, m := range modes {
		if m.name == name {
			return m
		}
	}
	return mode{}
}

func serve(ctx context.Context) error {
	base := server.NewBase()
	catalog.New("catalog", *base)
	return nil
}

func main() {
	_ = lookup("serve").run(context.Background())
}
`
	fixture["cmd/plugins/main.go"] = `package main

import (
	"example.test/routes/catalog"
	"example.test/routes/server"
)

type app interface{ Mount(base server.Base) }

type shop struct{}

func (shop) Mount(base server.Base) { catalog.New("shop", base) }

func apps() []app { return []app{shop{}} }

func main() {
	base := server.NewBase()
	for _, application := range apps() {
		application.Mount(*base)
	}
}
`
	models, err := WithBuiltinRouteModels(nil)
	if err != nil {
		t.Fatalf("route models: %v", err)
	}
	rows, err := Emit(Config{Root: writeFixture(t, fixture), ModuleRoots: []string{"."}, Patterns: []string{"./..."}, RouteModels: models})
	if err != nil {
		t.Fatalf("Emit failed: %v", err)
	}
	table := strings.Join(routeTable(rows), "\n")
	budget := "factory example.test/routes/server.Budget"
	auth := "factory example.test/routes/server.Authenticate via example.test/routes/server.Base.Auth"
	for _, want := range []string{
		"http POST /catalog/items path_complete=true middleware_complete=true handlers=[function (example.test/routes/catalog.HTTP).create] middleware=[" + budget + " | " + auth + "]",
		"http POST /shop/items path_complete=true middleware_complete=true handlers=[function (example.test/routes/catalog.HTTP).create] middleware=[" + budget + " | " + auth + "]",
	} {
		if !strings.Contains(table, want) {
			t.Fatalf("missing %q in routes:\n%s", want, table)
		}
	}
	if strings.Contains(table, "/{?}/items") || strings.Contains(table, "http POST /items ") {
		t.Fatalf("the setup was left open:\n%s", table)
	}
}
