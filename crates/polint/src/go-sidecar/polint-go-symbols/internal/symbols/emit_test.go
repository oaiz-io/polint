package symbols

import (
	"os"
	"path/filepath"
	"reflect"
	"strconv"
	"strings"
	"testing"
)

func TestGoBuildFlagsAlwaysUsesReadonlyModules(t *testing.T) {
	got := goBuildFlags(nil)
	want := []string{"-mod=readonly"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("goBuildFlags(nil) = %#v, want %#v", got, want)
	}
}

func TestGoBuildFlagsKeepsBuildTags(t *testing.T) {
	got := goBuildFlags([]string{"integration", "linux"})
	want := []string{"-mod=readonly", "-tags=integration,linux"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("goBuildFlags(tags) = %#v, want %#v", got, want)
	}
}

func TestValidatePackagePatternsRejectsFlags(t *testing.T) {
	_, err := validatePackagePatterns([]string{"./...", "-json"})
	if err == nil {
		t.Fatalf("validatePackagePatterns accepted a go list flag")
	}
}

func TestValidateModuleRootFilesRejectsSymlinkAncestor(t *testing.T) {
	root := t.TempDir()
	outside := t.TempDir()
	if err := os.MkdirAll(filepath.Join(outside, "app"), 0o755); err != nil {
		t.Fatalf("mkdir outside app: %v", err)
	}
	if err := os.WriteFile(filepath.Join(outside, "app", "go.mod"), []byte("module example.com/app\n\ngo 1.24\n"), 0o644); err != nil {
		t.Fatalf("write outside go.mod: %v", err)
	}
	if err := os.Symlink(outside, filepath.Join(root, "link")); err != nil {
		t.Skipf("symlink unavailable: %v", err)
	}

	if err := validateModuleRootFiles(root, []string{"link/app"}); err == nil {
		t.Fatalf("validateModuleRootFiles accepted symlinked module root")
	}
}

func TestGoWorkCoversModuleRootsRejectsOutsideUse(t *testing.T) {
	root := t.TempDir()
	outside := t.TempDir()
	if err := os.MkdirAll(filepath.Join(root, "app"), 0o755); err != nil {
		t.Fatalf("mkdir app: %v", err)
	}
	if err := os.WriteFile(filepath.Join(root, "app", "go.mod"), []byte("module example.com/app\n\ngo 1.24\n"), 0o644); err != nil {
		t.Fatalf("write app go.mod: %v", err)
	}
	workPath := filepath.Join(root, "go.work")
	contents := "go 1.24.0\n\nuse (\n\t./app\n\t" + strconv.Quote(filepath.ToSlash(outside)) + "\n)\n"
	if err := os.WriteFile(workPath, []byte(contents), 0o644); err != nil {
		t.Fatalf("write go.work: %v", err)
	}

	if goWorkCoversModuleRoots(root, []string{"app"}) {
		t.Fatalf("goWorkCoversModuleRoots accepted go.work with outside use entry")
	}
}

func TestGoWorkCoversModuleRootsRejectsSymlinkUse(t *testing.T) {
	root := t.TempDir()
	outside := t.TempDir()
	if err := os.MkdirAll(filepath.Join(root, "app"), 0o755); err != nil {
		t.Fatalf("mkdir app: %v", err)
	}
	if err := os.WriteFile(filepath.Join(root, "app", "go.mod"), []byte("module example.com/app\n\ngo 1.24\n"), 0o644); err != nil {
		t.Fatalf("write app go.mod: %v", err)
	}
	if err := os.MkdirAll(filepath.Join(outside, "link"), 0o755); err != nil {
		t.Fatalf("mkdir outside link: %v", err)
	}
	if err := os.WriteFile(filepath.Join(outside, "link", "go.mod"), []byte("module example.com/link\n\ngo 1.24\n"), 0o644); err != nil {
		t.Fatalf("write outside go.mod: %v", err)
	}
	if err := os.Symlink(filepath.Join(outside, "link"), filepath.Join(root, "link")); err != nil {
		t.Skipf("symlink unavailable: %v", err)
	}
	workPath := filepath.Join(root, "go.work")
	contents := "go 1.24.0\n\nuse (\n\t./app\n\t./link\n)\n"
	if err := os.WriteFile(workPath, []byte(contents), 0o644); err != nil {
		t.Fatalf("write go.work: %v", err)
	}

	if goWorkCoversModuleRoots(root, []string{"app"}) {
		t.Fatalf("goWorkCoversModuleRoots accepted symlinked go.work use entry")
	}
}

func TestGoPackageEnvRejectsGoWorkSymlink(t *testing.T) {
	root := t.TempDir()
	outside := t.TempDir()
	outsideWork := filepath.Join(outside, "go.work")
	if err := os.WriteFile(outsideWork, []byte("go 1.24.0\n\nuse ./app\n"), 0o644); err != nil {
		t.Fatalf("write outside go.work: %v", err)
	}
	if err := os.Symlink(outsideWork, filepath.Join(root, "go.work")); err != nil {
		t.Skipf("symlink unavailable: %v", err)
	}

	env, cleanup, err := goPackageEnv(root, []string{"app"})
	if err != nil {
		t.Fatalf("goPackageEnv: %v", err)
	}
	defer cleanup()

	for _, value := range env {
		if value == "GOWORK="+filepath.Join(root, "go.work") {
			t.Fatalf("goPackageEnv reused symlinked go.work")
		}
	}
}

func TestSyntheticGoWorkVersionUsesHighestModuleGoLine(t *testing.T) {
	root := t.TempDir()
	for dir, goLine := range map[string]string{"api": "1.24", "core": "1.27.1", "tools": "1.26"} {
		if err := os.MkdirAll(filepath.Join(root, dir), 0o755); err != nil {
			t.Fatalf("mkdir %s: %v", dir, err)
		}
		contents := "module example.com/" + dir + "\n\ngo " + goLine + "\n"
		if err := os.WriteFile(filepath.Join(root, dir, "go.mod"), []byte(contents), 0o644); err != nil {
			t.Fatalf("write %s go.mod: %v", dir, err)
		}
	}

	got := syntheticGoWorkVersion(root, []string{"api", "core", "tools"})
	if got != "1.27.1" {
		t.Fatalf("syntheticGoWorkVersion = %q, want the highest module go line 1.27.1", got)
	}
}

func TestSyntheticGoWorkVersionNeverDropsBelowTheMinimum(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "go.mod"), []byte("module example.com/app\n\ngo 1.21\n"), 0o644); err != nil {
		t.Fatalf("write go.mod: %v", err)
	}

	got := syntheticGoWorkVersion(root, []string{"."})
	if got != minimumWorkspaceGoVersion {
		t.Fatalf("syntheticGoWorkVersion = %q, want %q", got, minimumWorkspaceGoVersion)
	}
}

func TestWriteSyntheticGoWorkKeepsItsFilesOutOfTheRepositoryParent(t *testing.T) {
	parent := t.TempDir()
	root := filepath.Join(parent, "repo")
	if err := os.MkdirAll(filepath.Join(root, "core"), 0o755); err != nil {
		t.Fatalf("mkdir core: %v", err)
	}
	if err := os.WriteFile(filepath.Join(root, "core", "go.mod"), []byte("module example.com/core\n\ngo 1.27.1\n"), 0o644); err != nil {
		t.Fatalf("write go.mod: %v", err)
	}

	path, cleanup, err := writeSyntheticGoWork(root, []string{"core"})
	if err != nil {
		t.Fatalf("writeSyntheticGoWork: %v", err)
	}
	contents, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read synthetic go.work: %v", err)
	}
	if !strings.HasPrefix(string(contents), "go 1.27.1\n") {
		t.Fatalf("synthetic go.work starts %q, want the module's go line", string(contents))
	}
	// The go command writes a checksum file beside the workspace it loads.
	sum := path + ".sum"
	if err := os.WriteFile(sum, []byte(""), 0o600); err != nil {
		t.Fatalf("write go.work.sum: %v", err)
	}
	cleanup()

	if _, err := os.Stat(filepath.Dir(path)); !os.IsNotExist(err) {
		t.Fatalf("cleanup left the workspace directory behind: %v", err)
	}
	entries, err := os.ReadDir(parent)
	if err != nil {
		t.Fatalf("read repository parent: %v", err)
	}
	if len(entries) != 1 || entries[0].Name() != "repo" {
		t.Fatalf("repository parent holds %d entries, want only the repository", len(entries))
	}
}
