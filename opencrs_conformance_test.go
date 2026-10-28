package opencrs

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
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
	// Strictest, for a case about strictest requirements, is the references
	// to resolve in Specs[0] and what each must give.
	Strictest []cstrictest `json:"strictest"`
	dir       string
}

type cstrictest struct {
	Ref    string          `json:"ref"`
	Expect json.RawMessage `json:"expect"`
	Notes  *int            `json:"notes"`
	Absent []string        `json:"absent"`
	Error  string          `json:"error"`
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

// within adds to problems wherever got doesn't hold what want says: objects
// are matched by the members want gives, lists exactly, numbers by value.
func within(want, got any, at, ref string, problems *[]string) {
	here := at
	if here == "" {
		here = "the result"
	}
	if want == nil {
		return
	}
	switch w := want.(type) {
	case map[string]any:
		g, ok := got.(map[string]any)
		if !ok {
			*problems = append(*problems, fmt.Sprintf("%s: %s is %v, want %v", ref, here, got, want))
			return
		}
		for k, wv := range w {
			path := k
			if at != "" {
				path = at + "." + k
			}
			gv, ok := g[k]
			if !ok {
				*problems = append(*problems, fmt.Sprintf("%s: %s is missing, want %v", ref, path, wv))
				continue
			}
			within(wv, gv, path, ref, problems)
		}
	case []any:
		g, ok := got.([]any)
		if !ok || len(g) != len(w) {
			*problems = append(*problems, fmt.Sprintf("%s: %s is %v, want %v", ref, here, got, want))
			return
		}
		for i := range w {
			within(w[i], g[i], fmt.Sprintf("%s[%d]", at, i), ref, problems)
		}
	default:
		if !reflect.DeepEqual(want, got) {
			*problems = append(*problems, fmt.Sprintf("%s: %s is %v, want %v", ref, here, got, want))
		}
	}
}

// compareStrictest checks the strictest results of a case, one per entry.
func compareStrictest(c *ccase, spec []byte) []string {
	var problems []string
	for _, s := range c.Strictest {
		// The result is read as the module wrote it, not through the typed
		// result, so a list that is empty on purpose stays an empty list.
		var raw map[string]any
		err := runWith(context.Background(), &raw, func(in *instance) ([]byte, error) {
			return in.callDocs("opencrs_resolve_strictest", map[string]any{"ref": s.Ref}, spec)
		})
		if s.Error != "" {
			if err == nil || !strings.Contains(err.Error(), s.Error) {
				problems = append(problems, fmt.Sprintf("%s: error %v, want one containing %q", s.Ref, err, s.Error))
			}
			continue
		}
		if err != nil {
			problems = append(problems, fmt.Sprintf("%s: error %v", s.Ref, err))
			continue
		}
		var want any
		if len(s.Expect) > 0 {
			if err := json.Unmarshal(s.Expect, &want); err != nil {
				problems = append(problems, fmt.Sprintf("%s: %v", s.Ref, err))
				continue
			}
		}
		if want == nil && s.Notes == nil {
			problems = append(problems, s.Ref+": the case expects nothing of the result")
		}
		within(want, raw, "", s.Ref, &problems)
		for _, path := range s.Absent {
			var at any = raw
			for _, k := range strings.Split(path, ".") {
				m, _ := at.(map[string]any)
				if at = m[k]; at == nil {
					break
				}
			}
			if at != nil {
				problems = append(problems, fmt.Sprintf("%s: %s is there, want it left out", s.Ref, path))
			}
		}
		if notes, _ := raw["notes"].([]any); s.Notes != nil && len(notes) != *s.Notes {
			problems = append(problems, fmt.Sprintf("%s: %d notes, want %d", s.Ref, len(notes), *s.Notes))
		}
		// The typed result reads the same answer.
		if _, err := ResolveStrictest(spec, s.Ref); err != nil {
			problems = append(problems, fmt.Sprintf("%s: ResolveStrictest: %v", s.Ref, err))
		}
	}
	return problems
}

// TestConformance runs every case of conformance/0.1 through CheckDir.
func TestConformance(t *testing.T) {
	cases := loadCases(t, "conformance/0.1")
	for _, c := range cases {
		if len(c.Strictest) > 0 {
			spec, err := os.ReadFile(filepath.Join(c.dir, c.Specs[0]))
			if err != nil {
				t.Fatal(err)
			}
			for _, p := range compareStrictest(c, spec) {
				t.Errorf("%s: %s", c.ID, p)
			}
			continue
		}
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
