package opencrs

import (
	"encoding/json"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"slices"
	"strings"
	"testing"
)

// parity/operations.json lists every operation of the core and the name each
// caller gives it. This test holds the Go package to it: every name listed
// must be an exported function here, and every exported function must be
// listed (as an operation or a helper), so an operation added to the file
// fails here until Go has it, and a call added here fails until it is listed.

type operation struct {
	Summary string `json:"summary"`
	// A binding's names, or null (nil here) when it has none.
	CLI    []string `json:"cli"`
	Go     []string `json:"go"`
	JS     []string `json:"js"`
	NPM    []string `json:"npm"`
	Python []string `json:"python"`
}

type manifest struct {
	Operations map[string]json.RawMessage `json:"operations"`
	Exceptions map[string]string          `json:"exceptions"`
	Helpers    map[string][]string        `json:"helpers"`
}

func loadManifest(t *testing.T) manifest {
	t.Helper()
	raw, err := os.ReadFile("parity/operations.json")
	if err != nil {
		t.Fatal(err)
	}
	var m manifest
	if err := json.Unmarshal(raw, &m); err != nil {
		t.Fatal(err)
	}
	return m
}

// exportedFuncs is the package's exported top-level functions, from its
// non-test source.
func exportedFuncs(t *testing.T) []string {
	t.Helper()
	fset := token.NewFileSet()
	pkgs, err := parser.ParseDir(fset, ".", func(fi os.FileInfo) bool {
		return !strings.HasSuffix(fi.Name(), "_test.go")
	}, 0)
	if err != nil {
		t.Fatal(err)
	}
	var names []string
	for _, pkg := range pkgs {
		for _, f := range pkg.Files {
			for _, d := range f.Decls {
				if fn, ok := d.(*ast.FuncDecl); ok && fn.Recv == nil && fn.Name.IsExported() {
					names = append(names, fn.Name.Name)
				}
			}
		}
	}
	slices.Sort(names)
	return names
}

func TestEveryOperationInTheManifestIsExportedAndNothingElseIs(t *testing.T) {
	m := loadManifest(t)
	have := exportedFuncs(t)
	listed := map[string]bool{}
	for _, h := range m.Helpers["go"] {
		listed[h] = true
	}
	for id, rawOp := range m.Operations {
		// Every binding must be named, with a list or an explicit null.
		var keys map[string]json.RawMessage
		if err := json.Unmarshal(rawOp, &keys); err != nil {
			t.Fatalf("%s: %v", id, err)
		}
		for _, b := range []string{"cli", "go", "js", "npm", "python"} {
			if _, ok := keys[b]; !ok {
				t.Errorf("%s: the manifest says nothing for %s", id, b)
			}
		}
		var op operation
		if err := json.Unmarshal(rawOp, &op); err != nil {
			t.Fatalf("%s: %v", id, err)
		}
		_, excepted := m.Exceptions["go."+id]
		switch {
		case op.Go == nil && !excepted:
			t.Errorf("%s: Go has no name and no exception", id)
		case op.Go != nil && excepted:
			t.Errorf("%s: Go has names and an exception", id)
		}
		for _, name := range op.Go {
			if !slices.Contains(have, name) {
				t.Errorf("%s: the package has no exported function %s", id, name)
			}
			listed[name] = true
		}
	}
	for _, name := range have {
		if !listed[name] {
			t.Errorf("%s is exported but not in parity/operations.json (as an operation, or under helpers)", name)
		}
	}
}
