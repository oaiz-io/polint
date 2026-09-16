package symbols

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

	"golang.org/x/mod/modfile"
	"golang.org/x/tools/go/packages"
	"golang.org/x/tools/go/types/objectpath"
)

const SchemaVersion = "polint-go-symbols-semantic-1"
const topologyManifestMaxBytes int64 = 1_048_576

type Config struct {
	Root         string
	ModuleRoots  []string
	Patterns     []string
	IncludeTests bool
	BuildTags    []string
	// RootedPatterns says Patterns are already relative to Root. The caller
	// derives one directory per in-scope file, and a directory belongs to one
	// module root, so the module-root cross product in rootedPackagePatterns
	// would turn each entry into paths that do not exist.
	RootedPatterns bool
}

type Output struct {
	Schema          string              `json:"schema"`
	GoVersion       string              `json:"go_version"`
	ModulePath      string              `json:"module_path,omitempty"`
	Packages        []PackageRow        `json:"packages"`
	Symbols         []SymbolRow         `json:"symbols"`
	Definitions     []DefinitionRow     `json:"definitions"`
	References      []ReferenceRow      `json:"references"`
	Scopes          []ScopeRow          `json:"scopes"`
	Imports         []ImportRow         `json:"imports"`
	Exports         []ExportRow         `json:"exports"`
	ResolutionSteps []ResolutionStepRow `json:"resolution_steps"`
	Errors          []PackageError      `json:"errors,omitempty"`
}

type PackageRow struct {
	ID          string   `json:"id"`
	Path        string   `json:"path"`
	Name        string   `json:"name"`
	ModulePath  string   `json:"module_path,omitempty"`
	TestVariant string   `json:"test_variant"`
	Files       []string `json:"files"`
}

type SymbolRow struct {
	Key           string   `json:"key"`
	PackageID     string   `json:"package_id"`
	PackagePath   string   `json:"package_path"`
	TestVariant   string   `json:"test_variant"`
	File          string   `json:"file,omitempty"`
	OwnerKey      string   `json:"owner_key,omitempty"`
	OwnerChain    []string `json:"owner_chain,omitempty"`
	Name          string   `json:"name"`
	QualifiedName string   `json:"qualified_name"`
	Namespace     string   `json:"namespace"`
	Kind          string   `json:"kind"`
	ObjectPath    string   `json:"objectpath,omitempty"`
	Span          Span     `json:"span"`
	Exported      bool     `json:"exported"`
}

type DefinitionRow struct {
	SymbolKey string `json:"symbol_key"`
	PackageID string `json:"package_id"`
	File      string `json:"file,omitempty"`
	Name      string `json:"name"`
	Kind      string `json:"kind"`
	Span      Span   `json:"span"`
	Implicit  bool   `json:"implicit"`
	Primary   bool   `json:"primary"`
}

type ReferenceRow struct {
	PackageID string `json:"package_id"`
	File      string `json:"file,omitempty"`
	Name      string `json:"name"`
	TargetKey string `json:"target_key,omitempty"`
	Kind      string `json:"kind"`
	Span      Span   `json:"span"`
	Precision string `json:"precision"`
}

type ScopeRow struct {
	Key         string `json:"key"`
	ParentKey   string `json:"parent_key"`
	Kind        string `json:"kind"`
	PackagePath string `json:"package_path"`
	File        string `json:"file,omitempty"`
	Span        Span   `json:"span"`
}

type ImportRow struct {
	Path      string `json:"path"`
	LocalName string `json:"local_name,omitempty"`
	AliasKind string `json:"alias_kind"`
	File      string `json:"file,omitempty"`
	Span      Span   `json:"span"`
}

type ExportRow struct {
	SymbolKey   string `json:"symbol_key"`
	ExportName  string `json:"export_name"`
	Namespace   string `json:"namespace"`
	ObjectPath  string `json:"object_path"`
	PackagePath string `json:"package_path"`
	Generated   bool   `json:"generated"`
}

type ResolutionStepRow struct {
	ReferenceKey  string   `json:"reference_key"`
	Step          string   `json:"step"`
	Status        string   `json:"status"`
	TargetKey     string   `json:"target_key,omitempty"`
	CandidateKeys []string `json:"candidate_keys"`
}

type PackageError struct {
	PackageID   string `json:"package_id"`
	PackagePath string `json:"package_path"`
	Message     string `json:"message"`
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
	root        string
	fset        *token.FileSet
	out         Output
	symbols     map[types.Object]string
	symbolRows  map[string]SymbolRow
	defRows     map[string]DefinitionRow
	refRows     map[string]ReferenceRow
	scopeRows   map[string]ScopeRow
	importRows  map[string]ImportRow
	exportRows  map[string]ExportRow
	stepRows    map[string]ResolutionStepRow
	kindByPos   map[token.Pos]string
	parents     map[ast.Node]ast.Node
	ownerRanges []ownerRange
}

type ownerRange struct {
	start token.Pos
	end   token.Pos
	name  string
}

func Emit(config Config) (Output, error) {
	root, err := filepath.Abs(config.Root)
	if err != nil {
		return Output{}, fmt.Errorf("resolve root: %w", err)
	}
	moduleRoots, err := normalizeModuleRoots(config.ModuleRoots)
	if err != nil {
		return Output{}, err
	}
	if err := validateModuleRootFiles(root, moduleRoots); err != nil {
		return Output{}, err
	}
	patterns := config.Patterns
	patterns, err = validatePackagePatterns(patterns)
	if err != nil {
		return Output{}, err
	}
	if !config.RootedPatterns {
		patterns = rootedPackagePatterns(moduleRoots, patterns)
	}
	env, cleanup, err := goPackageEnv(root, moduleRoots)
	if err != nil {
		return Output{}, err
	}
	defer cleanup()

	fset := token.NewFileSet()
	loadConfig := &packages.Config{
		Mode: packages.NeedName |
			packages.NeedFiles |
			packages.NeedCompiledGoFiles |
			packages.NeedImports |
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

	pkgs, err := packages.Load(loadConfig, patterns...)
	if err != nil {
		return Output{}, err
	}
	sort.Slice(pkgs, func(i, j int) bool {
		return pkgs[i].ID < pkgs[j].ID
	})

	e := &emitter{
		root: root,
		fset: fset,
		out: Output{
			Schema:          SchemaVersion,
			GoVersion:       runtime.Version(),
			Packages:        []PackageRow{},
			Symbols:         []SymbolRow{},
			Definitions:     []DefinitionRow{},
			References:      []ReferenceRow{},
			Scopes:          []ScopeRow{},
			Imports:         []ImportRow{},
			Exports:         []ExportRow{},
			ResolutionSteps: []ResolutionStepRow{},
			Errors:          []PackageError{},
		},
		symbols:    make(map[types.Object]string),
		symbolRows: make(map[string]SymbolRow),
		defRows:    make(map[string]DefinitionRow),
		refRows:    make(map[string]ReferenceRow),
		scopeRows:  make(map[string]ScopeRow),
		importRows: make(map[string]ImportRow),
		exportRows: make(map[string]ExportRow),
		stepRows:   make(map[string]ResolutionStepRow),
		kindByPos:  make(map[token.Pos]string),
		parents:    make(map[ast.Node]ast.Node),
	}

	for _, pkg := range pkgs {
		if e.out.ModulePath == "" && pkg.Module != nil {
			e.out.ModulePath = pkg.Module.Path
		}
		e.indexSyntax(pkg)
		e.emitPackage(pkg)
		e.emitPackageErrors(pkg)
		e.emitScopesAndImports(pkg)
		e.emitDefinitions(pkg)
		e.emitImplicits(pkg)
		e.emitUses(pkg)
		e.emitSelections(pkg)
	}
	e.finish()
	return e.out, nil
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
			pattern = strings.TrimSpace(pattern)
			if pattern == "" {
				continue
			}
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
	if path := filepath.Join(root, "go.work"); repoRegularFileExists(root, "go.work") && goWorkCoversModuleRoots(root, moduleRoots) {
		env = append(env, "GOWORK="+path)
		return env, func() {}, nil
	}
	if needsSyntheticWorkspace(root, moduleRoots) {
		gowork, cleanup, err := writeSyntheticGoWork(root, moduleRoots)
		if err != nil {
			return nil, nil, err
		}
		env = append(env, "GOWORK="+gowork)
		return env, cleanup, nil
	}
	env = append(env, "GOWORK=off")
	return env, func() {}, nil
}

func needsSyntheticWorkspace(root string, moduleRoots []string) bool {
	if len(moduleRoots) != 1 || moduleRoots[0] != "." {
		return true
	}
	return !repoRegularFileExists(root, "go.mod")
}

func goWorkCoversModuleRoots(root string, moduleRoots []string) bool {
	contents, err := readRepoFile(root, "go.work", topologyManifestMaxBytes)
	if err != nil {
		return false
	}
	workPath := filepath.Join(root, "go.work")
	workFile, err := modfile.ParseWork(workPath, contents, nil)
	if err != nil {
		return false
	}
	usedRoots := make(map[string]bool)
	for _, use := range workFile.Use {
		usedRoot, ok := goWorkUseRoot(root, use.Path)
		if !ok {
			return false
		}
		if !repoRegularFileExists(root, moduleRootManifest(usedRoot)) {
			return false
		}
		usedRoots[usedRoot] = true
	}
	for _, moduleRoot := range moduleRoots {
		if !usedRoots[moduleRoot] {
			return false
		}
	}
	return true
}

func goWorkUseRoot(root string, usePath string) (string, bool) {
	path := filepath.FromSlash(usePath)
	if !filepath.IsAbs(path) {
		path = filepath.Join(root, path)
	}
	relative, err := filepath.Rel(root, path)
	if err != nil {
		return "", false
	}
	clean := filepath.ToSlash(filepath.Clean(relative))
	if clean == "." {
		return ".", true
	}
	if clean == ".." || strings.HasPrefix(clean, "../") {
		return "", false
	}
	return clean, true
}

func writeSyntheticGoWork(root string, moduleRoots []string) (string, func(), error) {
	file, err := os.CreateTemp(filepath.Dir(root), "polint-go-symbols-*.work")
	if err != nil {
		return "", nil, fmt.Errorf("create synthetic go.work: %w", err)
	}
	path := file.Name()
	workDir := filepath.Dir(path)
	cleanup := func() {
		_ = os.Remove(path)
	}
	var builder strings.Builder
	builder.WriteString("go ")
	builder.WriteString(workspaceGoVersion())
	builder.WriteString("\n\nuse (\n")
	for _, moduleRoot := range moduleRoots {
		path := root
		if moduleRoot != "." {
			path = filepath.Join(root, filepath.FromSlash(moduleRoot))
		}
		builder.WriteString("\t")
		builder.WriteString(strconv.Quote(goWorkUsePath(workDir, path)))
		builder.WriteString("\n")
	}
	builder.WriteString(")\n")
	if _, err := file.WriteString(builder.String()); err != nil {
		_ = file.Close()
		cleanup()
		return "", nil, fmt.Errorf("write synthetic go.work: %w", err)
	}
	if err := file.Close(); err != nil {
		cleanup()
		return "", nil, fmt.Errorf("close synthetic go.work: %w", err)
	}
	return path, cleanup, nil
}

func goWorkUsePath(workDir string, modulePath string) string {
	relative, err := filepath.Rel(workDir, modulePath)
	if err == nil {
		return filepath.ToSlash(relative)
	}
	return filepath.ToSlash(modulePath)
}

func workspaceGoVersion() string {
	version := strings.TrimPrefix(runtime.Version(), "go")
	version = strings.TrimFunc(version, func(r rune) bool {
		return !(r == '.' || r >= '0' && r <= '9')
	})
	if version == "" {
		return "1.24"
	}
	return version
}

func repoRegularFileExists(root string, relative string) bool {
	_, _, err := repoRegularFile(root, relative)
	return err == nil
}

func readRepoFile(root string, relative string, maxBytes int64) ([]byte, error) {
	path, info, err := repoRegularFile(root, relative)
	if err != nil {
		return nil, err
	}
	if info.Size() > maxBytes {
		return nil, fmt.Errorf("%s exceeds max size", relative)
	}
	file, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer file.Close()
	bytes, err := io.ReadAll(io.LimitReader(file, maxBytes+1))
	if err != nil {
		return nil, err
	}
	if int64(len(bytes)) > maxBytes {
		return nil, fmt.Errorf("%s exceeds max size", relative)
	}
	return bytes, nil
}

func repoRegularFile(root string, relative string) (string, os.FileInfo, error) {
	clean, err := cleanRepoRelative(relative)
	if err != nil {
		return "", nil, err
	}
	path := root
	parts := strings.Split(clean, string(filepath.Separator))
	for index, part := range parts {
		path = filepath.Join(path, part)
		info, err := os.Lstat(path)
		if err != nil {
			return "", nil, err
		}
		if info.Mode()&os.ModeSymlink != 0 {
			return "", nil, fmt.Errorf("%s escapes repository through a symlink", relative)
		}
		if index+1 < len(parts) {
			if !info.IsDir() {
				return "", nil, fmt.Errorf("%s has a non-directory ancestor", relative)
			}
			continue
		}
		if !info.Mode().IsRegular() {
			return "", nil, fmt.Errorf("%s is not a regular repository file", relative)
		}
		return path, info, nil
	}
	return "", nil, fmt.Errorf("%s is not repository relative", relative)
}

func cleanRepoRelative(relative string) (string, error) {
	path := filepath.Clean(filepath.FromSlash(relative))
	if path == "." || filepath.IsAbs(path) {
		return "", fmt.Errorf("%s is not repository relative", relative)
	}
	if path == ".." || strings.HasPrefix(path, ".."+string(filepath.Separator)) {
		return "", fmt.Errorf("%s escapes repository", relative)
	}
	return path, nil
}

func (e *emitter) indexSyntax(pkg *packages.Package) {
	for _, file := range pkg.Syntax {
		e.indexParents(file)
		e.indexKinds(file)
		e.indexOwners(file)
	}
}

func (e *emitter) indexParents(file *ast.File) {
	var stack []ast.Node
	ast.Inspect(file, func(node ast.Node) bool {
		if node == nil {
			if len(stack) > 0 {
				stack = stack[:len(stack)-1]
			}
			return false
		}
		if len(stack) > 0 {
			e.parents[node] = stack[len(stack)-1]
		}
		stack = append(stack, node)
		return true
	})
}

func (e *emitter) indexKinds(file *ast.File) {
	ast.Inspect(file, func(node ast.Node) bool {
		switch n := node.(type) {
		case *ast.FuncType:
			markFieldListKinds(e.kindByPos, n.Params, "parameter")
			markFieldListKinds(e.kindByPos, n.Results, "parameter")
		case *ast.StructType:
			markFieldListKinds(e.kindByPos, n.Fields, "field")
		}
		return true
	})
}

func (e *emitter) indexOwners(file *ast.File) {
	ast.Inspect(file, func(node ast.Node) bool {
		switch n := node.(type) {
		case *ast.FuncDecl:
			name := "func " + n.Name.Name
			if recv := receiverName(n.Recv); recv != "" {
				name = "method " + recv + "." + n.Name.Name
			}
			e.ownerRanges = append(e.ownerRanges, ownerRange{start: n.Pos(), end: n.End(), name: name})
		case *ast.FuncLit:
			span := e.span(n.Type.Pos(), n.Type.Pos()+token.Pos(len("func")))
			e.ownerRanges = append(e.ownerRanges, ownerRange{
				start: n.Pos(),
				end:   n.End(),
				name:  fmt.Sprintf("func@line:%d:column:%d", span.StartLine, span.StartColumn),
			})
		case *ast.TypeSpec:
			e.ownerRanges = append(e.ownerRanges, ownerRange{start: n.Pos(), end: n.End(), name: "type " + n.Name.Name})
		}
		return true
	})
}

func markFieldListKinds(kinds map[token.Pos]string, fields *ast.FieldList, kind string) {
	if fields == nil {
		return
	}
	for _, field := range fields.List {
		for _, name := range field.Names {
			kinds[name.Pos()] = kind
		}
	}
}

func (e *emitter) emitPackage(pkg *packages.Package) {
	files := make([]string, 0, len(pkg.CompiledGoFiles))
	for _, file := range pkg.CompiledGoFiles {
		if rel := e.relativePath(file); rel != "" {
			files = append(files, rel)
		}
	}
	sort.Strings(files)
	modulePath := ""
	if pkg.Module != nil {
		modulePath = pkg.Module.Path
	}
	e.out.Packages = append(e.out.Packages, PackageRow{
		ID:          pkg.ID,
		Path:        pkg.PkgPath,
		Name:        pkg.Name,
		ModulePath:  modulePath,
		TestVariant: testVariant(pkg),
		Files:       files,
	})
}

func (e *emitter) emitPackageErrors(pkg *packages.Package) {
	for _, err := range pkg.Errors {
		e.out.Errors = append(e.out.Errors, PackageError{
			PackageID:   pkg.ID,
			PackagePath: pkg.PkgPath,
			Message:     err.Msg,
		})
	}
}

func (e *emitter) emitScopesAndImports(pkg *packages.Package) {
	packageKey := scopeKey(pkg.PkgPath, "", "package", 0, pkg.PkgPath)
	e.addScope(packageKey, "", "package", pkg.PkgPath, "", Span{})

	for _, file := range pkg.Syntax {
		filePath := e.fileForPos(file.Pos())
		fileKey := scopeKey(pkg.PkgPath, filePath, "file", 0, filePath)
		e.addScope(fileKey, packageKey, "file", pkg.PkgPath, filePath, e.span(file.Pos(), file.End()))
		for _, spec := range file.Imports {
			e.addImport(spec)
		}
		ast.Inspect(file, func(node ast.Node) bool {
			switch n := node.(type) {
			case *ast.FuncDecl:
				kind := "function"
				name := n.Name.Name
				if recv := receiverName(n.Recv); recv != "" {
					kind = "method"
					name = recv + "." + n.Name.Name
				}
				key := scopeKey(pkg.PkgPath, filePath, kind, e.fileRelativeOffset(n.Pos()), name)
				e.addScope(key, fileKey, kind, pkg.PkgPath, filePath, e.span(n.Pos(), n.End()))
			case *ast.FuncLit:
				key := scopeKey(pkg.PkgPath, filePath, "function", e.fileRelativeOffset(n.Pos()), "literal")
				e.addScope(key, fileKey, "function", pkg.PkgPath, filePath, e.span(n.Pos(), n.End()))
			case *ast.TypeSpec:
				key := scopeKey(pkg.PkgPath, filePath, "type", e.fileRelativeOffset(n.Pos()), n.Name.Name)
				e.addScope(key, fileKey, "type", pkg.PkgPath, filePath, e.span(n.Pos(), n.End()))
			case *ast.BlockStmt:
				offset := e.fileRelativeOffset(n.Pos())
				key := scopeKey(pkg.PkgPath, filePath, "block", offset, fmt.Sprintf("%d", offset))
				parent := e.enclosingScopeKey(pkg, filePath, fileKey, n.Pos())
				e.addScope(key, parent, "block", pkg.PkgPath, filePath, e.span(n.Pos(), n.End()))
			}
			return true
		})
	}
}

func (e *emitter) addScope(key string, parentKey string, kind string, packagePath string, file string, span Span) {
	if key == "" {
		return
	}
	e.scopeRows[key] = ScopeRow{
		Key:         key,
		ParentKey:   parentKey,
		Kind:        kind,
		PackagePath: packagePath,
		File:        file,
		Span:        span,
	}
}

func (e *emitter) addImport(spec *ast.ImportSpec) {
	path, err := strconv.Unquote(spec.Path.Value)
	if err != nil {
		path = strings.Trim(spec.Path.Value, `"`)
	}
	localName := ""
	aliasKind := "implicit"
	if spec.Name != nil {
		localName = spec.Name.Name
		switch spec.Name.Name {
		case ".":
			aliasKind = "dot"
		case "_":
			aliasKind = "blank"
		default:
			aliasKind = "named"
		}
	}
	file := e.fileForPos(spec.Pos())
	span := e.span(spec.Pos(), spec.End())
	key := strings.Join([]string{file, path, localName, aliasKind, fmt.Sprintf("%d", span.StartByte)}, "|")
	e.importRows[key] = ImportRow{
		Path:      path,
		LocalName: localName,
		AliasKind: aliasKind,
		File:      file,
		Span:      span,
	}
}

func (e *emitter) emitDefinitions(pkg *packages.Package) {
	type item struct {
		ident *ast.Ident
		obj   types.Object
	}
	items := make([]item, 0, len(pkg.TypesInfo.Defs))
	for ident, obj := range pkg.TypesInfo.Defs {
		if ident != nil && obj != nil {
			items = append(items, item{ident: ident, obj: obj})
		}
	}
	sort.Slice(items, func(i, j int) bool {
		return posLess(items[i].ident.Pos(), items[j].ident.Pos(), items[i].ident.Name, items[j].ident.Name)
	})
	for _, item := range items {
		key := e.ensureSymbol(pkg, item.ident, item.obj)
		if key == "" {
			continue
		}
		span := e.identSpan(item.ident)
		e.defRows[definitionKey(key, span, false)] = DefinitionRow{
			SymbolKey: key,
			PackageID: pkg.ID,
			File:      e.fileForPos(item.ident.Pos()),
			Name:      item.ident.Name,
			Kind:      definitionKind(item.obj),
			Span:      span,
			Implicit:  false,
			Primary:   true,
		}
	}
}

func (e *emitter) emitImplicits(pkg *packages.Package) {
	type item struct {
		node ast.Node
		obj  types.Object
	}
	items := make([]item, 0, len(pkg.TypesInfo.Implicits))
	for node, obj := range pkg.TypesInfo.Implicits {
		if node != nil && obj != nil {
			items = append(items, item{node: node, obj: obj})
		}
	}
	sort.Slice(items, func(i, j int) bool {
		return posLess(items[i].node.Pos(), items[j].node.Pos(), items[i].obj.Name(), items[j].obj.Name())
	})
	for _, item := range items {
		ident := ast.NewIdent(item.obj.Name())
		ident.NamePos = item.node.Pos()
		key := e.ensureSymbol(pkg, ident, item.obj)
		if key == "" {
			continue
		}
		span := e.span(item.node.Pos(), item.node.End())
		e.defRows[definitionKey(key, span, true)] = DefinitionRow{
			SymbolKey: key,
			PackageID: pkg.ID,
			File:      e.fileForPos(item.node.Pos()),
			Name:      item.obj.Name(),
			Kind:      "implicit",
			Span:      span,
			Implicit:  true,
			Primary:   false,
		}
	}
}

func (e *emitter) emitUses(pkg *packages.Package) {
	type item struct {
		ident *ast.Ident
		obj   types.Object
	}
	items := make([]item, 0, len(pkg.TypesInfo.Uses))
	for ident, obj := range pkg.TypesInfo.Uses {
		if ident != nil && obj != nil {
			items = append(items, item{ident: ident, obj: obj})
		}
	}
	sort.Slice(items, func(i, j int) bool {
		return posLess(items[i].ident.Pos(), items[j].ident.Pos(), items[i].ident.Name, items[j].ident.Name)
	})
	for _, item := range items {
		if selector := e.selectorParent(item.ident); selector != nil {
			if _, ok := pkg.TypesInfo.Selections[selector]; ok && selector.Sel == item.ident {
				continue
			}
		}
		targetKey := e.ensureSymbol(pkg, item.ident, item.obj)
		kind := e.referenceKind(item.ident, item.obj)
		e.addReference(pkg.ID, e.fileForPos(item.ident.Pos()), item.ident.Name, targetKey, kind, e.identSpan(item.ident))
	}
}

func (e *emitter) emitSelections(pkg *packages.Package) {
	type item struct {
		selector  *ast.SelectorExpr
		selection *types.Selection
	}
	items := make([]item, 0, len(pkg.TypesInfo.Selections))
	for selector, selection := range pkg.TypesInfo.Selections {
		if selector != nil && selection != nil && selection.Obj() != nil {
			items = append(items, item{selector: selector, selection: selection})
		}
	}
	sort.Slice(items, func(i, j int) bool {
		return posLess(items[i].selector.Sel.Pos(), items[j].selector.Sel.Pos(), items[i].selector.Sel.Name, items[j].selector.Sel.Name)
	})
	for _, item := range items {
		targetKey := e.ensureSymbol(pkg, item.selector.Sel, item.selection.Obj())
		kind := e.selectionReferenceKind(item.selector, item.selection)
		e.addReference(pkg.ID, e.fileForPos(item.selector.Sel.Pos()), item.selector.Sel.Name, targetKey, kind, e.identSpan(item.selector.Sel))
	}
}

func (e *emitter) ensureSymbol(pkg *packages.Package, ident *ast.Ident, obj types.Object) string {
	if obj == nil {
		return ""
	}
	if obj.Name() == "" {
		return ""
	}
	if ident == nil {
		ident = ast.NewIdent(obj.Name())
		ident.NamePos = obj.Pos()
	}
	if key, ok := e.symbols[obj]; ok {
		e.upgradeSymbolLocation(pkg, ident, obj, key)
		return key
	}
	span := e.identSpan(ident)
	kind := e.symbolKind(obj, ident)
	namespace := symbolNamespace(obj)
	objectPath, hasObjectPath := safeObjectPath(obj)
	file := e.fileForPos(ident.Pos())
	if symbolFileShouldBeEmpty(pkg, obj) {
		file = ""
	}
	ownerChain := e.ownerChainAt(pkg, ident.Pos())
	if file == "" || isPackageLevel(obj, pkg) {
		ownerChain = nil
	}
	ownerKey := ""
	key := e.symbolKey(pkg, obj, file, kind, namespace, objectPath, hasObjectPath, ownerChain, span)
	if key == "" {
		return ""
	}

	row := SymbolRow{
		Key:           key,
		PackageID:     pkg.ID,
		PackagePath:   symbolPackagePath(pkg, obj),
		TestVariant:   testVariant(pkg),
		File:          file,
		OwnerKey:      ownerKey,
		OwnerChain:    ownerChain,
		Name:          obj.Name(),
		QualifiedName: qualifiedName(obj),
		Namespace:     namespace,
		Kind:          kind,
		ObjectPath:    objectPath,
		Span:          span,
		Exported:      obj.Exported(),
	}
	e.symbols[obj] = key
	e.symbolRows[key] = row
	e.addExport(row)
	return key
}

func (e *emitter) upgradeSymbolLocation(pkg *packages.Package, ident *ast.Ident, obj types.Object, key string) {
	if symbolFileShouldBeEmpty(pkg, obj) {
		return
	}
	row, ok := e.symbolRows[key]
	if !ok || row.File != "" {
		return
	}
	row.PackageID = pkg.ID
	row.PackagePath = symbolPackagePath(pkg, obj)
	row.TestVariant = testVariant(pkg)
	row.File = e.fileForPos(ident.Pos())
	row.Span = e.identSpan(ident)
	row.OwnerChain = e.ownerChainAt(pkg, ident.Pos())
	if isPackageLevel(obj, pkg) {
		row.OwnerChain = nil
	}
	e.symbolRows[key] = row
	e.addExport(row)
}

func (e *emitter) addExport(row SymbolRow) {
	if !row.Exported || row.ObjectPath == "" || row.PackagePath == "" {
		return
	}
	key := strings.Join([]string{row.PackagePath, row.Namespace, row.Name, row.ObjectPath, row.Key}, "|")
	e.exportRows[key] = ExportRow{
		SymbolKey:   row.Key,
		ExportName:  row.Name,
		Namespace:   row.Namespace,
		ObjectPath:  row.ObjectPath,
		PackagePath: row.PackagePath,
		Generated:   false,
	}
}

func (e *emitter) symbolKind(obj types.Object, ident *ast.Ident) string {
	switch typed := obj.(type) {
	case *types.Func:
		if signature, ok := typed.Type().(*types.Signature); ok && signature.Recv() != nil {
			return "method"
		}
		return "function"
	case *types.Var:
		if typed.IsField() {
			return "field"
		}
		if e.kindByPos[ident.Pos()] == "parameter" {
			return "parameter"
		}
		return "variable"
	case *types.Const:
		return "constant"
	case *types.TypeName:
		return "type"
	case *types.PkgName:
		return "package"
	case *types.Label:
		return "label"
	default:
		return "unknown"
	}
}

func symbolNamespace(obj types.Object) string {
	switch obj.(type) {
	case *types.TypeName:
		return "type"
	case *types.PkgName:
		return "package"
	case *types.Label:
		return "label"
	default:
		return "value"
	}
}

func (e *emitter) symbolKey(pkg *packages.Package, obj types.Object, file string, kind string, namespace string, objectPath string, hasObjectPath bool, ownerChain []string, span Span) string {
	var pkgPath string
	if obj.Pkg() != nil {
		pkgPath = obj.Pkg().Path()
	}
	if pkgPath == "" {
		pkgPath = pkg.PkgPath
	}
	if hasObjectPath {
		return strings.Join([]string{
			"go:package",
			"module:" + modulePath(pkg),
			"package:" + pkgPath,
			"variant:" + testVariant(pkg),
			"namespace:" + namespace,
			"kind:" + kind,
			"name:" + obj.Name(),
			"objectpath:" + objectPath,
		}, "|")
	}
	if obj.Pkg() == nil && file == "" {
		return strings.Join([]string{
			"go:builtin",
			"namespace:" + namespace,
			"kind:" + kind,
			"name:" + obj.Name(),
		}, "|")
	}
	if isPackageLevel(obj, pkg) {
		return strings.Join([]string{
			"go:package",
			"module:" + modulePath(pkg),
			"package:" + pkgPath,
			"variant:" + testVariant(pkg),
			"namespace:" + namespace,
			"kind:" + kind,
			"name:" + obj.Name(),
			fmt.Sprintf("line:%d", span.StartLine),
			fmt.Sprintf("column:%d", span.StartColumn),
		}, "|")
	}
	parts := []string{
		"go:local",
		"package_id:" + pkg.ID,
		"package:" + pkgPath,
		"variant:" + testVariant(pkg),
		"file:" + file,
		"owner:" + strings.Join(ownerChain, "/"),
		"namespace:" + namespace,
		"kind:" + kind,
		"name:" + obj.Name(),
		fmt.Sprintf("line:%d", span.StartLine),
		fmt.Sprintf("column:%d", span.StartColumn),
	}
	return strings.Join(parts, "|")
}

func symbolFileShouldBeEmpty(pkg *packages.Package, obj types.Object) bool {
	if obj.Pkg() == nil {
		return true
	}
	if pkg == nil {
		return true
	}
	return obj.Pkg().Path() != pkg.PkgPath
}

func symbolPackagePath(pkg *packages.Package, obj types.Object) string {
	if obj != nil && obj.Pkg() != nil && obj.Pkg().Path() != "" {
		return obj.Pkg().Path()
	}
	if pkg == nil {
		return ""
	}
	return pkg.PkgPath
}

func isPackageLevel(obj types.Object, pkg *packages.Package) bool {
	return pkg != nil && obj.Parent() != nil && pkg.Types != nil && obj.Parent() == pkg.Types.Scope()
}

func (e *emitter) referenceKind(ident *ast.Ident, obj types.Object) string {
	if _, ok := obj.(*types.PkgName); ok {
		return "package"
	}
	if _, ok := obj.(*types.TypeName); ok {
		return "type"
	}
	if e.isCallIdentifier(ident) {
		return "call"
	}
	if kind := e.assignmentKind(ident); kind != "" {
		return kind
	}
	return "read"
}

func (e *emitter) selectionReferenceKind(selector *ast.SelectorExpr, selection *types.Selection) string {
	if e.isCallSelector(selector) {
		return "call"
	}
	if kind := e.assignmentKind(selector); kind != "" {
		return kind
	}
	switch selection.Kind() {
	case types.FieldVal:
		return "field"
	case types.MethodVal, types.MethodExpr:
		return "method"
	default:
		return "member"
	}
}

func (e *emitter) isCallIdentifier(ident *ast.Ident) bool {
	parent := e.parents[ident]
	if call, ok := parent.(*ast.CallExpr); ok {
		return call.Fun == ident
	}
	if selector, ok := parent.(*ast.SelectorExpr); ok && selector.Sel == ident {
		return e.isCallSelector(selector)
	}
	return false
}

func (e *emitter) isCallSelector(selector *ast.SelectorExpr) bool {
	parent := e.parents[selector]
	call, ok := parent.(*ast.CallExpr)
	return ok && call.Fun == selector
}

func (e *emitter) assignmentKind(expr ast.Expr) string {
	parent := e.parents[expr]
	switch p := parent.(type) {
	case *ast.AssignStmt:
		if !exprListContains(p.Lhs, expr) {
			return ""
		}
		if p.Tok == token.ASSIGN || p.Tok == token.DEFINE {
			return "write"
		}
		return "read_write"
	case *ast.IncDecStmt:
		if p.X == expr {
			return "read_write"
		}
	}
	return ""
}

func exprListContains(expressions []ast.Expr, target ast.Expr) bool {
	for _, expression := range expressions {
		if expression == target {
			return true
		}
	}
	return false
}

func (e *emitter) selectorParent(ident *ast.Ident) *ast.SelectorExpr {
	selector, ok := e.parents[ident].(*ast.SelectorExpr)
	if !ok {
		return nil
	}
	return selector
}

func (e *emitter) addReference(packageID string, file string, name string, targetKey string, kind string, span Span) {
	key := strings.Join([]string{
		packageID,
		file,
		name,
		targetKey,
		kind,
		fmt.Sprintf("%d", span.StartByte),
		fmt.Sprintf("%d", span.EndByte),
	}, "|")
	e.refRows[key] = ReferenceRow{
		PackageID: packageID,
		File:      file,
		Name:      name,
		TargetKey: targetKey,
		Kind:      kind,
		Span:      span,
		Precision: "exact_semantic",
	}
	candidates := []string{}
	status := "unresolved"
	if targetKey != "" {
		candidates = []string{targetKey}
		status = "resolved"
	}
	e.stepRows[key] = ResolutionStepRow{
		ReferenceKey:  key,
		Step:          "LexicalLookup",
		Status:        status,
		TargetKey:     targetKey,
		CandidateKeys: candidates,
	}
}

func (e *emitter) ownerChainAt(pkg *packages.Package, pos token.Pos) []string {
	var ranges []ownerRange
	for _, owner := range e.ownerRanges {
		if owner.start <= pos && pos <= owner.end {
			ranges = append(ranges, owner)
		}
	}
	sort.Slice(ranges, func(i, j int) bool {
		if ranges[i].start == ranges[j].start {
			return ranges[i].end < ranges[j].end
		}
		return ranges[i].start < ranges[j].start
	})
	chain := make([]string, 0, len(ranges)+1)
	for _, owner := range ranges {
		chain = append(chain, owner.name)
	}
	if depth := scopeDepthAt(pkg, pos); depth > 0 {
		chain = append(chain, fmt.Sprintf("scope-depth:%d", depth))
	}
	return chain
}

func (e *emitter) finish() {
	for _, row := range e.symbolRows {
		e.out.Symbols = append(e.out.Symbols, row)
	}
	for _, row := range e.defRows {
		e.out.Definitions = append(e.out.Definitions, row)
	}
	for _, row := range e.refRows {
		e.out.References = append(e.out.References, row)
	}
	for _, row := range e.scopeRows {
		e.out.Scopes = append(e.out.Scopes, row)
	}
	for _, row := range e.importRows {
		e.out.Imports = append(e.out.Imports, row)
	}
	for _, row := range e.exportRows {
		e.out.Exports = append(e.out.Exports, row)
	}
	for _, row := range e.stepRows {
		e.out.ResolutionSteps = append(e.out.ResolutionSteps, row)
	}
	sort.Slice(e.out.Packages, func(i, j int) bool { return e.out.Packages[i].ID < e.out.Packages[j].ID })
	sort.Slice(e.out.Symbols, func(i, j int) bool { return e.out.Symbols[i].Key < e.out.Symbols[j].Key })
	sort.Slice(e.out.Definitions, func(i, j int) bool {
		return definitionOrderKey(e.out.Definitions[i]) < definitionOrderKey(e.out.Definitions[j])
	})
	sort.Slice(e.out.References, func(i, j int) bool {
		return referenceOrderKey(e.out.References[i]) < referenceOrderKey(e.out.References[j])
	})
	sort.Slice(e.out.Scopes, func(i, j int) bool { return scopeOrderKey(e.out.Scopes[i]) < scopeOrderKey(e.out.Scopes[j]) })
	sort.Slice(e.out.Imports, func(i, j int) bool { return importOrderKey(e.out.Imports[i]) < importOrderKey(e.out.Imports[j]) })
	sort.Slice(e.out.Exports, func(i, j int) bool { return exportOrderKey(e.out.Exports[i]) < exportOrderKey(e.out.Exports[j]) })
	sort.Slice(e.out.ResolutionSteps, func(i, j int) bool {
		return resolutionStepOrderKey(e.out.ResolutionSteps[i]) < resolutionStepOrderKey(e.out.ResolutionSteps[j])
	})
	sort.Slice(e.out.Errors, func(i, j int) bool {
		left := e.out.Errors[i]
		right := e.out.Errors[j]
		return strings.Join([]string{left.PackageID, left.PackagePath, left.Message}, "\x00") <
			strings.Join([]string{right.PackageID, right.PackagePath, right.Message}, "\x00")
	})
}

func (e *emitter) enclosingScopeKey(pkg *packages.Package, filePath string, fileKey string, pos token.Pos) string {
	var best ownerRange
	found := false
	for _, owner := range e.ownerRanges {
		if owner.start <= pos && pos <= owner.end {
			if !found || owner.start > best.start {
				best = owner
				found = true
			}
		}
	}
	if !found {
		return fileKey
	}
	kind := "function"
	name := best.name
	switch {
	case strings.HasPrefix(name, "type "):
		kind = "type"
		name = strings.TrimPrefix(name, "type ")
	case strings.HasPrefix(name, "method "):
		kind = "method"
		name = strings.TrimPrefix(name, "method ")
	case strings.HasPrefix(name, "func "):
		name = strings.TrimPrefix(name, "func ")
	case strings.HasPrefix(name, "func@"):
		name = "literal"
	}
	return scopeKey(pkg.PkgPath, filePath, kind, e.fileRelativeOffset(best.start), name)
}

func (e *emitter) identSpan(ident *ast.Ident) Span {
	if ident == nil {
		return Span{}
	}
	return e.span(ident.Pos(), ident.End())
}

func (e *emitter) span(start token.Pos, end token.Pos) Span {
	startPos := e.fset.PositionFor(start, false)
	endPos := e.fset.PositionFor(end, false)
	return Span{
		StartByte:   startPos.Offset,
		EndByte:     endPos.Offset,
		StartLine:   startPos.Line,
		StartColumn: startPos.Column,
		EndLine:     endPos.Line,
		EndColumn:   endPos.Column,
	}
}

func (e *emitter) fileRelativeOffset(pos token.Pos) int {
	if pos == token.NoPos {
		return 0
	}
	offset := e.fset.PositionFor(pos, false).Offset
	if offset < 0 {
		return 0
	}
	return offset
}

func (e *emitter) fileForPos(pos token.Pos) string {
	filename := e.fset.PositionFor(pos, false).Filename
	return e.relativePath(filename)
}

func (e *emitter) relativePath(path string) string {
	if path == "" {
		return ""
	}
	abs, err := filepath.Abs(path)
	if err != nil {
		return filepath.ToSlash(path)
	}
	rel, err := filepath.Rel(e.root, abs)
	if err != nil || strings.HasPrefix(rel, "..") || filepath.IsAbs(rel) {
		return filepath.ToSlash(path)
	}
	return filepath.ToSlash(rel)
}

func safeObjectPath(obj types.Object) (path string, ok bool) {
	defer func() {
		if recover() != nil {
			path = ""
			ok = false
		}
	}()
	parts, err := objectpath.For(obj)
	if err != nil {
		return "", false
	}
	return string(parts), true
}

func scopeDepthAt(pkg *packages.Package, pos token.Pos) int {
	if pkg == nil {
		return 0
	}
	depth := 0
	for _, scope := range pkg.TypesInfo.Scopes {
		if scope == nil || scope.Pos() > pos || pos > scope.End() {
			continue
		}
		currentDepth := 0
		for current := scope; current != nil; current = current.Parent() {
			currentDepth++
		}
		if currentDepth > depth {
			depth = currentDepth
		}
	}
	return depth
}

func qualifiedName(obj types.Object) string {
	if obj == nil {
		return ""
	}
	if pkg := obj.Pkg(); pkg != nil && pkg.Path() != "" {
		return pkg.Path() + "." + obj.Name()
	}
	return obj.Name()
}

func modulePath(pkg *packages.Package) string {
	if pkg.Module == nil {
		return ""
	}
	return pkg.Module.Path
}

func testVariant(pkg *packages.Package) string {
	if strings.Contains(pkg.ID, ".test") || strings.Contains(pkg.ID, " [") || strings.HasSuffix(pkg.Name, "_test") {
		return "test"
	}
	return "regular"
}

func definitionKind(obj types.Object) string {
	switch obj.(type) {
	case *types.TypeName:
		return "type"
	case *types.PkgName:
		return "import"
	default:
		return "declaration"
	}
}

func receiverName(recv *ast.FieldList) string {
	if recv == nil || len(recv.List) == 0 {
		return ""
	}
	return exprName(recv.List[0].Type)
}

func exprName(expr ast.Expr) string {
	switch typed := expr.(type) {
	case *ast.Ident:
		return typed.Name
	case *ast.StarExpr:
		return exprName(typed.X)
	case *ast.SelectorExpr:
		return exprName(typed.X) + "." + typed.Sel.Name
	default:
		return ""
	}
}

func posLess(left token.Pos, right token.Pos, leftName string, rightName string) bool {
	if left == right {
		return leftName < rightName
	}
	return left < right
}

func definitionKey(symbolKey string, span Span, implicit bool) string {
	return strings.Join([]string{
		symbolKey,
		fmt.Sprintf("%t", implicit),
		fmt.Sprintf("%d", span.StartByte),
		fmt.Sprintf("%d", span.EndByte),
	}, "|")
}

func definitionOrderKey(row DefinitionRow) string {
	return strings.Join([]string{
		row.SymbolKey,
		row.File,
		fmt.Sprintf("%d", row.Span.StartByte),
		row.Name,
		row.Kind,
	}, "\x00")
}

func referenceOrderKey(row ReferenceRow) string {
	return strings.Join([]string{
		row.PackageID,
		row.File,
		fmt.Sprintf("%d", row.Span.StartByte),
		row.Name,
		row.Kind,
		row.TargetKey,
	}, "\x00")
}

func scopeKey(packagePath string, file string, kind string, offset int, name string) string {
	return strings.Join([]string{
		"go:scope",
		"package:" + packagePath,
		"file:" + file,
		"kind:" + kind,
		"name:" + name,
		fmt.Sprintf("offset:%d", offset),
	}, "|")
}

func scopeOrderKey(row ScopeRow) string {
	return strings.Join([]string{
		row.Key,
		row.ParentKey,
		row.Kind,
		row.PackagePath,
		row.File,
	}, "\x00")
}

func importOrderKey(row ImportRow) string {
	return strings.Join([]string{
		row.File,
		fmt.Sprintf("%d", row.Span.StartByte),
		row.Path,
		row.LocalName,
		row.AliasKind,
	}, "\x00")
}

func exportOrderKey(row ExportRow) string {
	return strings.Join([]string{
		row.PackagePath,
		row.Namespace,
		row.ExportName,
		row.ObjectPath,
		row.SymbolKey,
	}, "\x00")
}

func resolutionStepOrderKey(row ResolutionStepRow) string {
	return strings.Join([]string{
		row.ReferenceKey,
		row.Step,
		row.Status,
		row.TargetKey,
		strings.Join(row.CandidateKeys, "\x01"),
	}, "\x00")
}
