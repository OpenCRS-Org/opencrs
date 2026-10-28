package opencrs

import (
	"fmt"
	"strings"
	"testing"
	"time"
)

// bigScreens builds a screens.json of about 50 MB: 190,000 screens, the
// most the spec's document size limit allows at this many bytes a screen.
// With oneFormat, every screen uses the first format, so a target on it
// covers them all.
func bigScreens(oneFormat bool) []byte {
	const screens = 190000
	var b strings.Builder
	b.Grow(52 << 20)
	b.WriteString(`{"opencrs":"0.1","publisher":"big.example","updated_at":"2026-10-20T09:00:00+11:00","networks":[{"id":"net","name":"Big","venues":[`)
	for i := range 500 {
		if i > 0 {
			b.WriteByte(',')
		}
		fmt.Fprintf(&b, `{"id":"venue-%d","name":"Venue %d"}`, i, i)
	}
	b.WriteString(`],"formats":[`)
	for i := range 20 {
		if i > 0 {
			b.WriteByte(',')
		}
		fmt.Fprintf(&b, `{"id":"fmt-%d","pixels":[1080,1920],"physical_mm":[940,1670],"media":{"static":["image/png"],"motion":["video/mp4"],"slot_s":10,"fps":30}`, i)
		if i == 0 {
			// Every screen on this format carries the same finding.
			b.WriteString(`,"rules":{"rule_packs":["nope@1"]}`)
		}
		b.WriteByte('}')
	}
	b.WriteString(`],"screens":[`)
	for i := range screens {
		if i > 0 {
			b.WriteByte(',')
		}
		f := i % 20
		if oneFormat {
			f = 0
		}
		fmt.Fprintf(&b, `{"id":"S-%07d","format":"fmt-%d","venue":"venue-%d","placement":{"environment":"indoor","level":"L1","zone":"food court area number %d"},"viewing":{"distance_m":[2,6],"dwell_s":4,"approach":"head-on","light":"indoor"},"platform_ids":{"cms-a":"frame-%d"}}`,
			i, f, i%500, i%97, i)
	}
	b.WriteString(`]}]}`)
	return []byte(b.String())
}

const bigManifest = `{"opencrs":"0.1","campaign":{"id":"c","advertiser":"A","category":"IAB8"},"files":[{"file":"a.png","media_type":"image/png","pixels":[1080,1920],"targets":["big.example/net/fmt-0","big.example/net/screen/S-0000000"],"elements":[{"role":"headline","text":"Hi","rect_px":[90,210,700,260],"cap_height_px":92}]}]}`

// A document at the spec's size limit lints and checks within the memory
// cap, and in seconds.
func TestLintAndCheckLargeDocument(t *testing.T) {
	if testing.Short() {
		t.Skip("builds and checks a 50 MB document")
	}
	doc := bigScreens(true)
	t.Logf("document: %d bytes", len(doc))

	start := time.Now()
	lint, err := Lint(doc)
	if err != nil {
		t.Fatal(err)
	}
	t.Logf("lint: %v", time.Since(start))
	// The unknown rule pack is in the format's rules: one finding, for the
	// format, not one listing every screen that inherits it.
	if len(lint.DocumentChecks) != 1 || lint.DocumentChecks[0].Format != "fmt-0" || len(lint.DocumentChecks[0].Screens) != 0 {
		t.Fatalf("lint found %+v", lint.DocumentChecks)
	}

	start = time.Now()
	res, err := Check([][]byte{doc}, []byte(bigManifest), nil, time.Date(2026, 10, 28, 0, 0, 0, 0, time.UTC))
	if err != nil {
		t.Fatal(err)
	}
	t.Logf("check: %v", time.Since(start))
	if len(res.Reports) != 2 {
		t.Fatalf("got %d reports, want 2", len(res.Reports))
	}
	groups := res.Requirements["big.example/net/fmt-0"]
	total := 0
	for _, g := range groups {
		total += len(g.Screens)
	}
	// Screens differ only by venue, so there is an entry for each.
	if total != 190000 || len(groups) != 500 {
		t.Fatalf("format target has %d entries covering %d screens, want 500 covering 190000", len(groups), total)
	}
	if took := time.Since(start); took > 2*time.Minute {
		t.Errorf("check took %v", took)
	}
}

// A document built to make the checks do far more work than its size
// warrants stops with CRS-TOO-COMPLEX and a reason, not a crash or a hang.
func TestComplexDocumentStopsWithAReason(t *testing.T) {
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
	manifest := `{"opencrs":"0.1","files":[{"file":"a.png","media_type":"image/png","pixels":[1080,1920],"targets":["x.example/n/f"]}]}`
	res, err := Check([][]byte{[]byte(b.String())}, []byte(manifest), nil, time.Date(2026, 10, 28, 0, 0, 0, 0, time.UTC))
	if err != nil {
		t.Fatal(err)
	}
	var found *Finding
	for i, c := range res.Reports[0].Checks {
		if c.Code == CodeTooComplex {
			found = &res.Reports[0].Checks[i]
		}
	}
	if found == nil || found.Severity != "not_checked" || res.Reports[0].Result != "incomplete" {
		t.Fatalf("no CRS-TOO-COMPLEX finding: %+v", res.Reports[0])
	}
	if !strings.Contains(found.Message, "2,000 required elements") || !strings.Contains(found.Message, "/networks/0/rules/required_elements") {
		t.Errorf("the reason doesn't say where the work came from: %s", found.Message)
	}
}
