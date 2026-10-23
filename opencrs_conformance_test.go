package opencrs

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"sort"
	"strings"
	"testing"
	"time"
)

// A small port of the conformance suite's runner (conformance/conformance.go
// and the core's compare rules), kept here so this package's tests stand
// alone when the Go conformance package is removed. See conformance/README.md
// for the rules.

type ccase struct {
	ID       string    `json:"id"`
	Now      string    `json:"now"`
	Specs    []string  `json:"specs"`
	Creative string    `json:"creative"`
	Expect   []creport `json:"expect"`
	dir      string
}

type creport struct {
	File   string   `json:"file"`
	Target string   `json:"target"`
	Result string   `json:"result"`
	Checks []ccheck `json:"checks"`
}

type ccheck struct {
	Code     string   `json:"code"`
	Severity string   `json:"severity"`
	Element  string   `json:"element"`
	Screens  []string `json:"screens"`
	Message  string   `json:"message"`
}

func loadCases(t *testing.T, dir string) []*ccase {
	t.Helper()
	paths, err := filepath.Glob(filepath.Join(dir, "cases", "*", "case.json"))
	if err != nil || len(paths) == 0 {
		t.Fatalf("no cases under %s (%v)", dir, err)
	}
	var out []*ccase
	for _, p := range paths {
		raw, err := os.ReadFile(p)
		if err != nil {
			t.Fatal(err)
		}
		var c ccase
		if err := json.Unmarshal(raw, &c); err != nil {
			t.Fatalf("%s: %v", p, err)
		}
		c.dir = filepath.Dir(p)
		out = append(out, &c)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].ID < out[j].ID })
	return out
}

func describeCheck(c ccheck) string {
	s := c.Code + " " + c.Severity
	if c.Element != "" {
		s += " on " + c.Element
	}
	if len(c.Screens) > 0 {
		s += " [" + strings.Join(c.Screens, ", ") + "]"
	}
	return s
}

func sameSet(a, b []string) bool {
	x, y := slices.Clone(a), slices.Clone(b)
	slices.Sort(x)
	slices.Sort(y)
	return slices.Equal(x, y)
}

// compareReports returns what is wrong with output against the case's
// expectations: one report per expected (file, target); matching results;
// every expected finding present (element and screens where given; no
// screens means the finding covers every screen); no other error, warning or
// not_checked finding; extra info findings allowed.
func compareReports(c *ccase, output []byte) []string {
	var actual []creport
	if err := json.Unmarshal(output, &actual); err != nil {
		return []string{"output isn't a JSON array of reports: " + err.Error()}
	}
	var problems []string
	seen := map[string]bool{}
	for _, want := range c.Expect {
		key := want.File + " -> " + want.Target
		seen[key] = true
		i := slices.IndexFunc(actual, func(a creport) bool { return a.File == want.File && a.Target == want.Target })
		if i < 0 {
			problems = append(problems, key+": no report")
			continue
		}
		got := actual[i]
		if got.Result != want.Result {
			problems = append(problems, fmt.Sprintf("%s: result %q, want %q", key, got.Result, want.Result))
		}
		used := make([]bool, len(got.Checks))
		for _, w := range want.Checks {
			found := false
			for j, g := range got.Checks {
				if used[j] || g.Code != w.Code || g.Severity != w.Severity {
					continue
				}
				if w.Element != "" && g.Element != w.Element {
					continue
				}
				if !sameSet(g.Screens, w.Screens) {
					continue
				}
				used[j], found = true, true
				break
			}
			if !found {
				problems = append(problems, key+": missing "+describeCheck(w))
			}
		}
		for j, g := range got.Checks {
			if !used[j] && g.Severity != "info" {
				problems = append(problems, fmt.Sprintf("%s: unexpected %s (%q)", key, describeCheck(g), g.Message))
			}
		}
	}
	for _, a := range actual {
		if key := a.File + " -> " + a.Target; !seen[key] {
			problems = append(problems, key+": unexpected report")
		}
	}
	return problems
}

// TestConformance runs every case of conformance/0.1 through CheckDir.
func TestConformance(t *testing.T) {
	cases := loadCases(t, "conformance/0.1")
	for _, c := range cases {
		now, err := time.Parse(time.RFC3339, c.Now)
		if err != nil {
			t.Fatalf("%s: %v", c.ID, err)
		}
		res, err := CheckDir(os.DirFS(c.dir), c.Specs, c.Creative, now)
		if err != nil {
			t.Errorf("%s: %v", c.ID, err)
			continue
		}
		out, _ := json.Marshal(res.Reports)
		for _, p := range compareReports(c, out) {
			t.Errorf("%s: %s", c.ID, p)
		}
	}
	t.Logf("%d cases", len(cases))
}
