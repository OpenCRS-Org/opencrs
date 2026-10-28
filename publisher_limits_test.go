package opencrs

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/url"
	"strings"
	"testing"
	"time"
)

func indexPage(t *testing.T, publisher, name string, docs []string, delegates []string) Page {
	t.Helper()
	ds := []map[string]string{}
	var dl []map[string]string
	for _, u := range docs {
		ds = append(ds, map[string]string{"url": u})
	}
	for _, d := range delegates {
		dl = append(dl, map[string]string{"domain": d})
	}
	idx := map[string]any{
		"opencrs":   "0.1",
		"publisher": map[string]string{"id": publisher, "name": name},
		"documents": ds,
	}
	if dl != nil {
		idx["delegates"] = dl
	}
	b, err := json.Marshal(idx)
	if err != nil {
		t.Fatal(err)
	}
	return okPage(b)
}

// An index can name any URL. Addresses, local-only names and URLs with a
// password are rejected by the core with a reason and never reach Wanted, and
// neither does an address as a delegate.
func TestPublisherDoesNotWantInternalURLs(t *testing.T) {
	bad := []string{
		"https://127.0.0.1:6379/x",
		"https://169.254.169.254/latest/meta-data/iam/security-credentials/",
		"https://user:pw@10.0.0.1:8443/admin",
		"https://[::1]/",
		"https://metadata.google.internal/computeMetadata/v1/",
		"https://printer.local/x",
		"https://2130706433/x",
	}
	good := "https://cdn.example/ok.screens.json"
	pages := map[string]Page{pubIndexURL: indexPage(t, pubDomain, "H", append(bad[:len(bad):len(bad)], good),
		[]string{"10.0.0.5", "127.0.0.1", "169.254.169.254", "metadata.google.internal", "d.example"})}
	r, err := Publisher(pubDomain, pages, fixedNow)
	if err != nil {
		t.Fatal(err)
	}
	wantURLs := []string{good, "https://d.example/.well-known/opencrs.json"}
	if fmt.Sprint(r.Wanted) != fmt.Sprint(wantURLs) {
		t.Fatalf("wanted %v, want %v", r.Wanted, wantURLs)
	}
	if len(r.WantedDetail) != 2 || r.WantedDetail[0] != (WantedURL{URL: good, Kind: "document"}) ||
		r.WantedDetail[1] != (WantedURL{URL: wantURLs[1], Kind: "index", SameHost: true}) {
		t.Errorf("detail = %+v", r.WantedDetail)
	}
	rejected := map[string]string{}
	for _, d := range r.Discovery.Documents {
		if d.Status == "rejected" {
			rejected[d.URL] = d.Reason
		}
	}
	for _, u := range bad {
		if rejected[u] == "" {
			t.Errorf("%s was not rejected with a reason: %+v", u, r.Discovery.Documents)
		}
	}
	// What the reason says doesn't depend on what answered: no status code.
	for u, why := range rejected {
		if strings.Contains(why, "HTTP") {
			t.Errorf("%s: reason %q", u, why)
		}
	}
	refused := 0
	for _, dl := range r.Discovery.Delegates {
		if strings.Contains(dl.Error, "isn't a publisher domain") {
			refused++
		}
	}
	if refused != 4 {
		t.Errorf("%d delegates refused, want 4: %+v", refused, r.Discovery.Delegates)
	}
}

// 100 delegates listing 1,000 documents each ask for a bounded number of
// URLs, say what was left out, and finish promptly, deadline or not.
func TestPublisherBoundsWhatItWants(t *testing.T) {
	const delegates, perDelegate = 100, 1000
	var names []string
	pages := map[string]Page{}
	for d := range delegates {
		name := fmt.Sprintf("d%d.example", d)
		names = append(names, name)
		var docs []string
		for i := range perDelegate {
			docs = append(docs, fmt.Sprintf("https://victim.example/%d/%d.json", d, i))
		}
		pages["https://"+name+"/.well-known/opencrs.json"] = indexPage(t, name, name, docs, nil)
	}
	pages[pubIndexURL] = indexPage(t, pubDomain, "H", nil, names)

	start := time.Now()
	r, err := Publisher(pubDomain, pages, fixedNow)
	took := time.Since(start)
	if err != nil {
		t.Fatal(err)
	}
	if took > 5*time.Second {
		t.Errorf("took %v", took)
	}
	if len(r.Wanted) == 0 || len(r.Wanted) > 2000 {
		t.Fatalf("wanted %d URLs, want some and at most 2,000", len(r.Wanted))
	}
	perHost := map[string]int{}
	seen := map[string]bool{}
	for _, w := range r.Wanted {
		if seen[w] {
			t.Errorf("%s wanted twice", w)
		}
		seen[w] = true
		u, _ := url.Parse(w)
		perHost[u.Host]++
	}
	if perHost["victim.example"] >= 2000 || perHost["victim.example"] < 1000 {
		t.Errorf("%d URLs at the victim", perHost["victim.example"])
	}
	// The delegates' documents are capped at 250 each: after a delegate's
	// index is read, its first 250 are asked for.
	var limits []string
	for _, a := range r.Advice {
		if a.Severity == "warning" && strings.Contains(a.Message, "limit of this validator") {
			limits = append(limits, a.Message)
		}
	}
	if len(limits) == 0 || !strings.Contains(limits[0], "only the first 250") {
		t.Errorf("no limit finding in %+v", r.Advice)
	}
	if r.Warnings < len(limits) {
		t.Errorf("warnings %d, limits %d", r.Warnings, len(limits))
	}

	// An ended context stops it, wherever it is.
	ctx, cancel := context.WithTimeout(context.Background(), time.Millisecond)
	defer cancel()
	time.Sleep(2 * time.Millisecond)
	start = time.Now()
	if _, err := PublisherContext(ctx, pubDomain, pages, fixedNow); !errors.Is(err, context.DeadlineExceeded) || time.Since(start) > time.Second {
		t.Errorf("deadline: %v after %v", err, time.Since(start))
	}
	// And one that ends during the call, between the module's polls.
	ctx, cancel2 := context.WithTimeout(context.Background(), 5*time.Millisecond)
	defer cancel2()
	start = time.Now()
	_, err = PublisherContext(ctx, pubDomain, pages, fixedNow)
	if err != nil && !errors.Is(err, context.DeadlineExceeded) {
		t.Errorf("deadline during the call: %v", err)
	}
	if time.Since(start) > 2*time.Second {
		t.Errorf("deadline during the call: took %v", time.Since(start))
	}
}

// A page body is bytes, not JSON text: markup is not escaped on the way in,
// and a body that isn't UTF-8 is read as what it is.
func TestPublisherPageBodiesAreBytes(t *testing.T) {
	p := indexPage(t, pubDomain, "Harbourline <Retail> & Co", nil, nil)
	// Raw bytes in a string value of the index, as a publisher might serve.
	r, err := Publisher(pubDomain, map[string]Page{pubIndexURL: p}, fixedNow)
	if err != nil {
		t.Fatal(err)
	}
	if r.Discovery == nil || r.Discovery.Index.Publisher.Name != "Harbourline <Retail> & Co" {
		t.Fatalf("discovery = %+v", r.Discovery)
	}
	bad := Page{Status: 200, Body: "{\"opencrs\":\"0.1\",\"x\":\"\xff\xfe\"}"}
	r, err = Publisher(pubDomain, map[string]Page{pubIndexURL: bad}, fixedNow)
	if err != nil {
		t.Fatal(err)
	}
	if r.Error == "" || r.Errors != 1 {
		t.Errorf("a body that isn't UTF-8: %+v", r)
	}
}
