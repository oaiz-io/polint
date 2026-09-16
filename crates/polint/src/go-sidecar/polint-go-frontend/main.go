package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"strconv"
	"strings"

	"github.com/oaiz-io/polint/tools/polint-go-frontend/internal/semantic"
)

func main() {
	if len(os.Args) < 2 || os.Args[1] != "semantic" {
		fmt.Fprintln(os.Stderr, "usage: polint-go-frontend semantic --root <path> --module-roots <comma-list> --patterns <comma-list> --tests <bool> --build-tags <comma-list> [--rta-edges] [--scope-files <path>] --ndjson")
		os.Exit(2)
	}

	flags := flag.NewFlagSet("semantic", flag.ExitOnError)
	root := flags.String("root", ".", "repository root")
	moduleRoots := flags.String("module-roots", ".", "comma-separated module roots relative to --root")
	patterns := flags.String("patterns", "./...", "comma-separated package patterns")
	tests := flags.String("tests", "true", "include test package variants")
	buildTags := flags.String("build-tags", "", "comma-separated Go build tags")
	rtaEdges := flags.Bool("rta-edges", false, "emit rta_edge rows (only the polint-eval callgraph comparison reads them)")
	scopeFiles := flags.String("scope-files", "", "path to a newline-delimited list of repository-relative Go files this scan discovered")
	ndjson := flags.Bool("ndjson", false, "emit newline-delimited JSON")
	if err := flags.Parse(os.Args[2:]); err != nil {
		fmt.Fprintf(os.Stderr, "parse flags: %v\n", err)
		os.Exit(2)
	}
	if !*ndjson {
		fmt.Fprintln(os.Stderr, "--ndjson is required")
		os.Exit(2)
	}
	includeTests, err := strconv.ParseBool(*tests)
	if err != nil {
		fmt.Fprintf(os.Stderr, "invalid --tests value %q: %v\n", *tests, err)
		os.Exit(2)
	}

	scope, err := readScopeFiles(*scopeFiles)
	if err != nil {
		fmt.Fprintf(os.Stderr, "read --scope-files %q: %v\n", *scopeFiles, err)
		os.Exit(1)
	}

	rows, err := semantic.Emit(semantic.Config{
		Root:         *root,
		ModuleRoots:  splitComma(*moduleRoots),
		Patterns:     splitComma(*patterns),
		IncludeTests: includeTests,
		BuildTags:    splitComma(*buildTags),
		EmitRTAEdges: *rtaEdges,
		ScopeFiles:   scope,
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "emit Go semantics: %v\n", err)
		os.Exit(1)
	}

	encoder := json.NewEncoder(os.Stdout)
	encoder.SetEscapeHTML(false)
	for _, row := range rows {
		if err := encoder.Encode(row); err != nil {
			fmt.Fprintf(os.Stderr, "encode NDJSON: %v\n", err)
			os.Exit(1)
		}
	}
}

// readScopeFiles loads the discovered-file list. An empty path means the caller did
// not narrow the scan, which is not the same as narrowing it to nothing: it returns a
// nil map, and a nil map disables the filter.
func readScopeFiles(path string) (map[string]bool, error) {
	if path == "" {
		return nil, nil
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	scope := make(map[string]bool)
	for _, line := range strings.Split(string(raw), "\n") {
		line = strings.TrimSpace(line)
		if line != "" {
			scope[line] = true
		}
	}
	return scope, nil
}

func splitComma(value string) []string {
	var values []string
	for _, part := range strings.Split(value, ",") {
		part = strings.TrimSpace(part)
		if part != "" {
			values = append(values, part)
		}
	}
	return values
}
