//go:build !race

package opencrs

import (
	"context"
	"errors"
	"testing"
	"time"
)

// These build a 50 MB document and need the full speed of the module, which
// the race detector takes away, so they are left out of `go test -race`
// (CI's run). Plain `go test` runs them.

// A deadline stops a call on a 50 MB document, and the next call is good,
// including the one that reuses the stopped call's pool.
func TestDeadlineStopsLargeCalls(t *testing.T) {
	if testing.Short() {
		t.Skip("builds a 50 MB document")
	}
	doc := bigScreens(true)
	small := harbourline(t, "retail.screens.json")

	// Warm an instance so the stopped call reuses a pooled one.
	warm, stop := context.WithCancel(context.Background())
	defer stop()
	if _, err := LintContext(warm, small); err != nil {
		t.Fatal(err)
	}

	calls := map[string]func(context.Context) error{
		"Lint": func(ctx context.Context) error { _, err := LintContext(ctx, doc); return err },
		"Check": func(ctx context.Context) error {
			_, err := CheckContext(ctx, [][]byte{doc}, []byte(bigManifest), nil, fixedNow)
			return err
		},
		"Resolve": func(ctx context.Context) error {
			_, err := ResolveContext(ctx, doc, "big.example/net/fmt-0")
			return err
		},
		"ResolveStrictest": func(ctx context.Context) error {
			_, err := ResolveStrictestContext(ctx, doc, "big.example/net/fmt-0")
			return err
		},
	}
	// Deadlines in the middle of the work. The stop comes within a short time
	// of the deadline; how short is logged.
	for name, f := range calls {
		for _, deadline := range []time.Duration{20 * time.Millisecond, 150 * time.Millisecond, 300 * time.Millisecond} {
			ctx, cancel := context.WithTimeout(context.Background(), deadline)
			start := time.Now()
			err := f(ctx)
			took := time.Since(start)
			cancel()
			if err == nil {
				// The call finished before the deadline.
				t.Logf("%s: finished in %v, before a %v deadline", name, took, deadline)
				continue
			}
			if !errors.Is(err, context.DeadlineExceeded) {
				t.Errorf("%s: err = %v after %v, want DeadlineExceeded", name, err, took)
			}
			t.Logf("%s: deadline %v, stopped after %v (%v late)", name, deadline, took, took-deadline)
			if took-deadline > time.Second {
				t.Errorf("%s: stopped %v after a %v deadline", name, took-deadline, deadline)
			}
			// The next call is unaffected.
			res, err := Lint(small)
			if err != nil || !res.OK() {
				t.Errorf("%s: lint afterwards: %v", name, err)
			}
			res, err = LintContext(warm, small)
			if err != nil || !res.OK() {
				t.Errorf("%s: lint under the context afterwards: %v", name, err)
			}
		}
	}
}

// A context that can end but doesn't gives the same answers on the large
// document as a plain call, in about the same time (the module asks the host
// whether to stop only every few thousand units of work). The times are
// logged, best of three; they are not asserted, as machines differ.
func TestLiveContextOnLargeDocument(t *testing.T) {
	if testing.Short() {
		t.Skip("builds a 50 MB document")
	}
	doc := bigScreens(true)
	ref := "big.example/net/fmt-0"
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()
	best := func(f func() error) time.Duration {
		min := time.Duration(1<<63 - 1)
		for range 3 {
			start := time.Now()
			if err := f(); err != nil {
				t.Fatal(err)
			}
			if d := time.Since(start); d < min {
				min = d
			}
		}
		return min
	}
	run := map[string]func(context.Context) error{
		"Lint": func(c context.Context) error {
			res, err := LintContext(c, doc)
			if err == nil && (len(res.DocumentChecks) != 1 || res.DocumentChecks[0].Format != "fmt-0") {
				t.Errorf("lint found %+v", res.DocumentChecks)
			}
			return err
		},
		"Check": func(c context.Context) error {
			res, err := CheckContext(c, [][]byte{doc}, []byte(bigManifest), nil, fixedNow)
			if err == nil && len(res.Reports) != 2 {
				t.Errorf("check made %d reports", len(res.Reports))
			}
			return err
		},
		"ResolveStrictest": func(c context.Context) error {
			res, err := ResolveStrictestContext(c, doc, ref)
			if err == nil && res.Screens != 190000 {
				t.Errorf("strictest covers %d screens", res.Screens)
			}
			return err
		},
	}
	for _, name := range []string{"Lint", "Check", "ResolveStrictest"} {
		f := run[name]
		plain := best(func() error { return f(context.Background()) })
		live := best(func() error { return f(ctx) })
		t.Logf("%s: plain %v, with a live deadline %v (%+.1f%%)", name, plain, live,
			100*(float64(live)/float64(plain)-1))
	}
}

// A publisher with three documents of 50 MB fits in the module's memory
// (the pages go in as raw bytes, once), and each is trusted and linted.
func TestPublisherThreeLargeDocuments(t *testing.T) {
	if testing.Short() {
		t.Skip("builds a 50 MB document")
	}
	doc := string(bigScreens(true))
	urls := []string{"https://cdn.example/a.json", "https://cdn.example/b.json", "https://cdn.example/c.json"}
	pages := map[string]Page{"https://big.example/.well-known/opencrs.json": indexPage(t, "big.example", "Big", urls, nil)}
	for _, u := range urls {
		pages[u] = okPage([]byte(doc))
	}
	start := time.Now()
	r, err := Publisher("big.example", pages, fixedNow)
	if err != nil {
		t.Fatal(err)
	}
	t.Logf("publisher check of 3 documents of %d bytes: %v", len(doc), time.Since(start))
	if len(r.Wanted) != 0 || r.Error != "" || len(r.Discovery.Documents) != 3 {
		t.Fatalf("result: wanted %v, error %q, %d documents", r.Wanted, r.Error, len(r.Discovery.Documents))
	}
	for _, d := range r.Discovery.Documents {
		if d.Status != "trusted" || d.Screens != 190000 {
			t.Errorf("%s: %s (%s), %d screens", d.URL, d.Status, d.Reason, d.Screens)
		}
		if l := r.Lints[d.URL]; l == nil || len(l.DocumentChecks) != 1 || l.DocumentChecks[0].Format != "fmt-0" {
			t.Errorf("%s: lint %+v", d.URL, l)
		}
	}
	if r.Errors != 0 {
		t.Errorf("errors = %d: %+v", r.Errors, r.Advice)
	}
}
