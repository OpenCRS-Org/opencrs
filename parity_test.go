package opencrs

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"reflect"
	"strings"
	"testing"
	"time"
)

func ptr[T any](v T) *T { return &v }

// The calculators give the spec's worked examples, in full.
func TestCalculators(t *testing.T) {
	l, err := Legibility(LegibilityRequest{DistanceM: [2]float64{2, 6}, Screen: &ScreenWidth{WidthMM: 940, WidthPX: 1080}})
	if err != nil {
		t.Fatal(err)
	}
	want := &LegibilityResult{
		Ratio: 150,
		Key:   TextSize{MM: 40, PX: ptr(int64(46)), PXExact: ptr(45.96), Basis: "at 6 m, ratio 150"},
		Legal: TextSize{MM: 13.3, PX: ptr(int64(16)), PXExact: ptr(15.32), Basis: "at 2 m, ratio 150"},
	}
	if !reflect.DeepEqual(l, want) {
		t.Errorf("legibility = %+v, want %+v", l, want)
	}

	// With a dwell time and no screen: a word limit, and no pixel sizes.
	l, err = Legibility(LegibilityRequest{DistanceM: [2]float64{2, 6}, DwellS: ptr(4.0)})
	if err != nil {
		t.Fatal(err)
	}
	if l.Key.PX != nil || l.Words == nil || *l.Words != (WordLimit{Value: 8, Basis: "4 s dwell at 2 words/s"}) {
		t.Errorf("legibility with a dwell = %+v", l)
	}

	d, err := Dwell(DwellRequest{SpeedKMH: 80, DistanceM: [2]float64{20, 150}})
	if err != nil {
		t.Fatal(err)
	}
	wantGlance := GlanceView{SpeedMS: 22.22, CrossingS: 5.85, Seconds: 2, Words: 4, ReadFromM: 64.44}
	if d.SpeedKMH != 80 || d.Ratio != 150 || d.Rate != 2 || d.Glance != wantGlance || d.Key.MM != 429.6 || len(d.Packs) != 0 {
		t.Errorf("dwell = %+v", d)
	}
}

func TestRulePacksAndPackLimits(t *testing.T) {
	packs, err := RulePacks()
	if err != nil {
		t.Fatal(err)
	}
	var refs []string
	for _, p := range packs {
		refs = append(refs, p.Ref)
		if p.Title == "" || p.Status == "" || !strings.HasPrefix(p.SourceURL, "https://") {
			t.Errorf("incomplete pack %+v", p)
		}
	}
	wantRefs := []string{
		"au-nsw-transport-corridor@2017-11",
		"au-qld-tmr-roadside-advertising@2022-12",
		"au-sa-dit-advertising-signs@2014-08",
		"au-tas-spp-signs-code@2024-06",
		"au-vic-dtp-illuminated-signage@2023-10",
		"au-wa-mrwa-digital-signs@2025-05",
	}
	if !reflect.DeepEqual(refs, wantRefs) {
		t.Errorf("rule packs = %v, want %v", refs, wantRefs)
	}

	d, err := Dwell(DwellRequest{SpeedKMH: 60, DistanceM: [2]float64{20, 150}, RulePacks: []string{wantRefs[0]}})
	if err != nil {
		t.Fatal(err)
	}
	if d.Glance.ReadFromM != 53.33 || d.Key.MM != 355.6 || len(d.Packs) != 1 {
		t.Fatalf("dwell = %+v", d)
	}
	p := d.Packs[0]
	if p.Ref != wantRefs[0] || p.Status != "draft" || p.MinDwellS == nil || *p.MinDwellS != 10 ||
		p.DwellBasis != "below 80 km/h" || p.MaxTransitionS == nil || *p.MaxTransitionS != 0.1 ||
		p.MotionAllowed == nil || *p.MotionAllowed || !p.NoFlashing || !p.MaxWordsAtGlance ||
		!p.NoSequencing || p.FailureState != "black" || p.SchoolZone != "fixed_display_during_hours" {
		t.Errorf("pack limits = %+v", p)
	}
}

func TestCalculatorErrors(t *testing.T) {
	_, err := Dwell(DwellRequest{SpeedKMH: 60, DistanceM: [2]float64{20, 150}, RulePacks: []string{"nope"}})
	if err == nil || err.Error() != "unknown rule pack nope" {
		t.Errorf("unknown pack: %v", err)
	}
	_, err = Legibility(LegibilityRequest{DistanceM: [2]float64{6, 2}})
	if err == nil || err.Error() != "the near distance (6 m) is past the far one (2 m)" {
		t.Errorf("near past far: %v", err)
	}
}

func okPage(body []byte) Page {
	return Page{Status: 200, Body: string(body), ETag: `"1"`, CacheControl: "max-age=3600",
		ContentType: "application/json", AllowOrigin: "*"}
}

const (
	pubDomain   = "harbourline.example"
	pubIndexURL = "https://harbourline.example/.well-known/opencrs.json"
	pubDocURL   = "https://harbourline.example/opencrs/retail.screens.json"
)

// Publisher runs discovery over pages the caller fetched, and says which
// URLs it still wants, never one it has been given.
func TestPublisherPasses(t *testing.T) {
	index, err := os.ReadFile("examples/harbourline/.well-known/opencrs.json")
	if err != nil {
		t.Fatal(err)
	}
	now := time.Date(2026, 10, 25, 12, 0, 0, 0, time.UTC)
	pages := map[string]Page{}

	// Pass 1: nothing fetched, the index is wanted. That is a result, not an
	// error.
	r, err := Publisher(pubDomain, pages, now)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(r.Wanted, []string{pubIndexURL}) {
		t.Fatalf("pass 1 wanted %v, want the index", r.Wanted)
	}
	if r.Domain != pubDomain || r.IndexURL != pubIndexURL || r.Errors != 1 || r.Discovery != nil ||
		r.Error != "fetching the index: "+pubIndexURL+": not fetched" || r.Advice == nil || len(r.Advice) != 0 {
		t.Errorf("pass 1 = %+v", r)
	}

	pages[pubIndexURL] = okPage(index)
	r, err = Publisher(pubDomain, pages, now)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(r.Wanted, []string{pubDocURL}) {
		t.Fatalf("pass 2 wanted %v, want the document", r.Wanted)
	}
	if r.Discovery == nil || len(r.Discovery.Documents) != 1 || r.Discovery.Documents[0].URL != pubDocURL ||
		r.Discovery.Documents[0].Status != "failed" || !strings.HasSuffix(r.Discovery.Documents[0].Reason, "not fetched") {
		t.Errorf("pass 2 discovery = %+v", r.Discovery)
	}

	pages[pubDocURL] = okPage(harbourline(t, "retail.screens.json"))
	r, err = Publisher(pubDomain, pages, now)
	if err != nil {
		t.Fatal(err)
	}
	if len(r.Wanted) != 0 || r.Wanted == nil {
		t.Errorf("pass 3 wanted %#v, want an empty list", r.Wanted)
	}
	if r.Error != "" || r.Errors != 0 || r.Warnings != 0 || len(r.Advice) != 0 || len(r.Lints) != 0 {
		t.Errorf("pass 3 not clean: %+v", r)
	}
	if r.Discovery == nil || len(r.Discovery.Documents) != 1 {
		t.Fatalf("discovery = %+v", r.Discovery)
	}
	doc := r.Discovery.Documents[0]
	if doc.URL != pubDocURL || doc.Status != "trusted" || doc.Publisher != pubDomain || doc.ListedBy != pubDomain || doc.Screens == 0 {
		t.Errorf("document = %+v", doc)
	}

	// Past the document's expiry the core refuses to trust it.
	r, err = Publisher(pubDomain, pages, time.Date(2030, 1, 1, 0, 0, 0, 0, time.UTC))
	if err != nil {
		t.Fatal(err)
	}
	if r.Discovery == nil || r.Discovery.Documents[0].Status != "expired" || r.Errors == 0 || len(r.Wanted) != 0 {
		t.Errorf("after expiry: %+v", r)
	}
}

// A page that was fetched and failed is still a page: it isn't asked for
// again, so the loop ends.
func TestPublisherNeverAsksAgainForAGivenURL(t *testing.T) {
	now := time.Date(2026, 10, 25, 12, 0, 0, 0, time.UTC)
	for name, page := range map[string]Page{
		"transport failure": {Error: "connection refused"},
		"404":               {Status: 404},
		"empty 200":         {Status: 200},
	} {
		pages := map[string]Page{pubIndexURL: page}
		r, err := Publisher(pubDomain, pages, now)
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		if len(r.Wanted) != 0 {
			t.Errorf("%s: wanted %v although the index is in pages", name, r.Wanted)
		}
		if r.Error == "" || r.Errors != 1 {
			t.Errorf("%s: error %q, %d errors", name, r.Error, r.Errors)
		}
	}
	// The same for a document the index lists.
	index, err := os.ReadFile("examples/harbourline/.well-known/opencrs.json")
	if err != nil {
		t.Fatal(err)
	}
	pages := map[string]Page{pubIndexURL: okPage(index), pubDocURL: {Error: "timed out"}}
	r, err := Publisher(pubDomain, pages, now)
	if err != nil {
		t.Fatal(err)
	}
	if len(r.Wanted) != 0 || r.Discovery == nil || r.Discovery.Documents[0].Status != "failed" ||
		!strings.Contains(r.Discovery.Documents[0].Reason, "timed out") {
		t.Errorf("failed document: wanted %v, %+v", r.Wanted, r.Discovery)
	}
}

// When the index arrived but isn't valid, the result keeps what is wrong
// with it.
func TestPublisherKeepsIndexLint(t *testing.T) {
	pages := map[string]Page{pubIndexURL: okPage([]byte(`{"opencrs":"0.1"}`))}
	r, err := Publisher(pubDomain, pages, time.Date(2026, 10, 25, 12, 0, 0, 0, time.UTC))
	if err != nil {
		t.Fatalf("an unusable index is a result, not an error: %v", err)
	}
	if !strings.Contains(r.Error, "isn't a valid index") || r.Errors != 1 || len(r.Wanted) != 0 {
		t.Errorf("result = %+v", r)
	}
	if r.IndexLint == nil || r.IndexLint.OK() || len(r.IndexLint.SchemaErrors) == 0 {
		t.Fatalf("index lint was dropped: %+v", r.IndexLint)
	}
	var missing []string
	for _, e := range r.IndexLint.SchemaErrors {
		missing = append(missing, e.Message)
	}
	if !strings.Contains(strings.Join(missing, "\n"), "publisher") {
		t.Errorf("schema errors don't name the missing publisher: %v", missing)
	}
}

// Every Context call refuses a context that has ended, without taking an
// instance from the pool or making one.
func TestContextAlreadyDone(t *testing.T) {
	// Fill the pool so that taking one would show.
	if _, err := VersionContext(context.Background()); err != nil {
		t.Fatal(err)
	}
	pooled := poolSize()
	if pooled == 0 {
		t.Fatal("pool not filled")
	}

	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	doc := harbourline(t, "retail.screens.json")
	calls := map[string]func() error{
		"Lint": func() error { _, err := LintContext(ctx, doc); return err },
		"Check": func() error {
			_, err := CheckContext(ctx, [][]byte{doc}, harbourline(t, "creative.json"), nil, fixedNow)
			return err
		},
		"CheckDir": func() error {
			_, err := CheckDirContext(ctx, harbourlineFS(t), []string{"retail.screens.json"}, "creative.json", fixedNow)
			return err
		},
		"Resolve": func() error {
			_, err := ResolveContext(ctx, doc, "harbourline.example/retail/portrait-totem-75")
			return err
		},
		"ResolveStrictest": func() error {
			_, err := ResolveStrictestContext(ctx, doc, "harbourline.example/retail/portrait-totem-75")
			return err
		},
		"Probe":     func() error { _, err := ProbeContext(ctx, bytes.NewReader(doc), int64(len(doc))); return err },
		"Publisher": func() error { _, err := PublisherContext(ctx, pubDomain, nil, fixedNow); return err },
		"Dwell":     func() error { _, err := DwellContext(ctx, DwellRequest{DistanceM: [2]float64{1, 2}}); return err },
		"Legibility": func() error {
			_, err := LegibilityContext(ctx, LegibilityRequest{DistanceM: [2]float64{1, 2}})
			return err
		},
		"RulePacks": func() error { _, err := RulePacksContext(ctx); return err },
		"Version":   func() error { _, err := VersionContext(ctx); return err },
	}
	for name, f := range calls {
		if err := f(); !errors.Is(err, context.Canceled) {
			t.Errorf("%s: err = %v, want context.Canceled", name, err)
		}
	}
	if n := poolSize(); n != pooled {
		t.Errorf("pool went from %d to %d: a done context took an instance", pooled, n)
	}
}

// An instance reused from the pool runs under the new caller's context, and
// a context that ended after a call leaves nothing behind for the next.
func TestPooledInstanceUsesTheCallersContext(t *testing.T) {
	doc := harbourline(t, "retail.screens.json")
	first, cancel1 := context.WithCancel(context.Background())
	if _, err := LintContext(first, doc); err != nil {
		t.Fatal(err)
	}
	if poolSize() == 0 {
		t.Fatal("the instance was not kept")
	}
	cancel1() // the first call's context ends after it, while its instance idles

	// The next call gets that instance. It must not be bound to the dead
	// context, and a context of its own must be able to stop it.
	second, cancel2 := context.WithCancel(context.Background())
	res, err := LintContext(second, doc)
	if err != nil || !res.OK() || res.Kind != "screens" {
		t.Fatalf("lint after the first context ended: %v, %+v", err, res)
	}
	cancel2()
	if _, err := LintContext(second, doc); !errors.Is(err, context.Canceled) {
		t.Errorf("an ended context: %v", err)
	}
	if res, err := Lint(doc); err != nil || !res.OK() {
		t.Errorf("plain lint afterwards: %v", err)
	}
}

// The Context calls give the same answers as the plain ones.
func TestContextCallsMatchPlainCalls(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	doc := harbourline(t, "retail.screens.json")
	ref := "harbourline.example/retail/portrait-totem-75"

	a, err1 := Resolve(doc, ref)
	b, err2 := ResolveContext(ctx, doc, ref)
	if err1 != nil || err2 != nil || len(a) != 2 || !reflect.DeepEqual(a, b) {
		t.Errorf("Resolve: %v %v, %d vs %d", err1, err2, len(a), len(b))
	}
	s1, err1 := ResolveStrictest(doc, ref)
	s2, err2 := ResolveStrictestContext(ctx, doc, ref)
	if err1 != nil || err2 != nil || s1.Screens != 2 || !reflect.DeepEqual(s1, s2) {
		t.Errorf("ResolveStrictest: %v %v", err1, err2)
	}
	l1, _ := Lint(doc)
	l2, err := LintContext(ctx, doc)
	if err != nil || !reflect.DeepEqual(l1, l2) {
		t.Errorf("Lint: %v", err)
	}
	if v, err := VersionContext(ctx); err != nil || v != "0.1" {
		t.Errorf("Version: %q, %v", v, err)
	}
	c1, err1 := Check([][]byte{doc}, harbourline(t, "creative.json"), nil, fixedNow)
	c2, err2 := CheckContext(ctx, [][]byte{doc}, harbourline(t, "creative.json"), nil, fixedNow)
	if err1 != nil || err2 != nil || len(c1.Reports) == 0 || !reflect.DeepEqual(c1, c2) {
		t.Errorf("Check: %v %v", err1, err2)
	}
}

// complexScreens is a small document that asks for far more work than its
// size: 2,000 required elements, rechecked for 6,000 screens that all differ.
func complexScreens() []byte {
	var b strings.Builder
	b.WriteString(`{"opencrs":"0.1","publisher":"x.example","updated_at":"2026-10-20T09:00:00+11:00","networks":[{"id":"n","name":"N","rules":{"required_elements":[`)
	for i := range 2000 {
		if i > 0 {
			b.WriteByte(',')
		}
		fmt.Fprintf(&b, `{"role":"r%d","min_cap_height_mm":1}`, i)
	}
	b.WriteString(`]},"formats":[{"id":"f","pixels":[1080,1920],"media":{"static":["image/png"]}}],"screens":[`)
	for i := range 6000 {
		if i > 0 {
			b.WriteByte(',')
		}
		fmt.Fprintf(&b, `{"id":"s%d","format":"f","viewing":{"distance_m":[1,%d],"dwell_s":%d},"placement":{"zone":"z%d"}}`, i, 2+i%900, 1+i%40, i)
	}
	b.WriteString(`]}]}`)
	return []byte(b.String())
}

// A document that is too complex gives an error wrapping ErrTooComplex, from
// every resolve call, with the module's own text.
func TestErrTooComplex(t *testing.T) {
	doc := complexScreens()
	ref := "x.example/n/f"
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	calls := map[string]func() error{
		"Resolve":                 func() error { _, err := Resolve(doc, ref); return err },
		"ResolveContext":          func() error { _, err := ResolveContext(ctx, doc, ref); return err },
		"ResolveStrictest":        func() error { _, err := ResolveStrictest(doc, ref); return err },
		"ResolveStrictestContext": func() error { _, err := ResolveStrictestContext(ctx, doc, ref); return err },
	}
	for name, f := range calls {
		err := f()
		if !errors.Is(err, ErrTooComplex) {
			t.Errorf("%s: err = %v, want ErrTooComplex", name, err)
			continue
		}
		if !strings.HasPrefix(err.Error(), CodeTooComplex+": ") || !strings.Contains(err.Error(), "2,000 required elements") {
			t.Errorf("%s: the text lost the reason: %q", name, err)
		}
	}
	// Other errors are not ErrTooComplex.
	if _, err := Resolve(harbourline(t, "retail.screens.json"), "harbourline.example/retail/nope"); err == nil || errors.Is(err, ErrTooComplex) {
		t.Errorf("an unknown reference: %v", err)
	}
}
