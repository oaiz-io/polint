package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"strconv"
	"strings"

	"github.com/oaiz-io/polint/tools/polint-go-symbols/internal/symbols"
)

func main() {
	if len(os.Args) < 2 || os.Args[1] != "symbols" {
		fmt.Fprintln(os.Stderr, "usage: polint-go-symbols symbols --root <path> --module-roots <comma-list> --patterns <comma-list> [--rooted-patterns] --tests <bool> --build-tags <comma-list> --json")
		os.Exit(2)
	}

	flags := flag.NewFlagSet("symbols", flag.ExitOnError)
	root := flags.String("root", ".", "repository root")
	moduleRoots := flags.String("module-roots", ".", "comma-separated module roots relative to --root")
	patterns := flags.String("patterns", "./...", "comma-separated package patterns")
	tests := flags.String("tests", "true", "include test package variants")
	buildTags := flags.String("build-tags", "", "comma-separated Go build tags")
	rootedPatterns := flags.Bool("rooted-patterns", false, "treat --patterns as already rooted at --root instead of relative to each module root")
	jsonOutput := flags.Bool("json", false, "emit JSON")
	if err := flags.Parse(os.Args[2:]); err != nil {
		fmt.Fprintf(os.Stderr, "parse flags: %v\n", err)
		os.Exit(2)
	}
	if !*jsonOutput {
		fmt.Fprintln(os.Stderr, "--json is required")
		os.Exit(2)
	}
	includeTests, err := strconv.ParseBool(*tests)
	if err != nil {
		fmt.Fprintf(os.Stderr, "invalid --tests value %q: %v\n", *tests, err)
		os.Exit(2)
	}

	out, err := symbols.Emit(symbols.Config{
		Root:         *root,
		ModuleRoots:  splitComma(*moduleRoots),
		Patterns:     splitComma(*patterns),
		IncludeTests:   includeTests,
		BuildTags:      splitComma(*buildTags),
		RootedPatterns: *rootedPatterns,
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "emit Go symbols: %v\n", err)
		os.Exit(1)
	}

	encoder := json.NewEncoder(os.Stdout)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(out); err != nil {
		fmt.Fprintf(os.Stderr, "encode JSON: %v\n", err)
		os.Exit(1)
	}
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
