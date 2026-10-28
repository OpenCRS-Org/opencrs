package opencrs

import (
	"context"
	"encoding/json"
	"slices"
	"time"
	"unsafe"
)

// Publisher checks a publisher's published specs from pages the caller has
// already fetched. It fetches nothing itself.
//
// # The loop
//
// Start with no pages. Each call runs discovery from scratch over the pages
// it is given, and the result's Wanted lists the URLs discovery needed that
// are not in pages, in the order it asked, each once (WantedDetail says what
// each is for). Fetch them, add them to pages, call again, until Wanted is
// empty:
//
//	pages := map[string]opencrs.Page{}
//	for round := 0; round < 5; round++ { // the core needs at most three
//		res, err := opencrs.Publisher(domain, pages, now)
//		if err != nil {
//			return err
//		}
//		if len(res.Wanted) == 0 {
//			return report(res) // final
//		}
//		for _, w := range res.WantedDetail {
//			pages[w.URL] = opencrs.FetchPage(ctx, client, w, 0)
//		}
//	}
//
// Or let FetchPublisher run that loop. The rules of the loop:
//
//   - Key pages by the URL exactly as Wanted gives it, not by where a redirect
//     ended. Wanted matches page keys by exact string, so a page kept under
//     another key is asked for again, and a caller that keeps doing so never
//     finishes.
//   - Record a fetch that failed too: a Page with Error set (Status 0), or
//     the Status of an error response. A URL that is in pages is never in
//     Wanted, and a failed one is reported against its document.
//   - The passes are the index, then its documents and delegate indexes,
//     then the delegates' documents: at most three. The core asks for at
//     most 2,000 URLs in a pass, 250 documents from any one delegate, and
//     says in the result (a warning in Advice) when it left some out.
//
// # Every Wanted URL is the publisher's choice
//
// The core decides trust, but it does not fetch, and the URLs it wants come
// out of files the publisher (and its delegates) wrote. It refuses ones at an
// IP address, at a name reserved for the local machine or network, or with a
// user name in them, but a name that looks public can resolve to an address
// inside your network, and a public URL can redirect to one. Fetched with a
// default http.Client, they would let a publisher aim your server at its
// neighbours, and the pages' status codes, which the result reports, would
// tell it what answered. Fetch them with SafeClient and FetchPage (or have
// FetchPublisher do it): HTTPS only, no connection to a non-public address
// checked after DNS and on every redirect, no redirect to another host for a
// SameHost URL, a cap on each page and on the total, no proxy.
//
// The module holds every page at once, in its 256 MiB of memory. An index
// and three documents of 50 MB fit (four do too, just; five do not): the
// limit is about 200 MB of pages in all, and a request past the module's
// memory fails the call. FetchPublisher stops at 180 MiB.
//
// A zero now means the current time.
//
// An error means the request could not be run (the module failed, ctx ended
// in PublisherContext, or now was not a time). Problems with the publisher's
// files are in the result: when the index itself is unusable, the result's
// Error says why and IndexLint, if the index arrived but is invalid, lists
// what is wrong with it.
func Publisher(domain string, pages map[string]Page, now time.Time) (*PublisherResult, error) {
	return PublisherContext(context.Background(), domain, pages, now)
}

// PublisherContext is Publisher with a context; see LintContext.
func PublisherContext(ctx context.Context, domain string, pages map[string]Page, now time.Time) (*PublisherResult, error) {
	if now.IsZero() {
		now = time.Now()
	}
	// The page bodies go to the module as raw bytes after a JSON header of
	// everything else, in the order of the header's page list, so nothing is
	// escaped and nothing copied but into the module's memory.
	urls := make([]string, 0, len(pages))
	for u := range pages {
		urls = append(urls, u)
	}
	slices.Sort(urls)
	type pageHead struct {
		URL string `json:"url"`
		Page
	}
	heads := make([]pageHead, len(urls))
	bodies := make([][]byte, len(urls))
	for i, u := range urls {
		heads[i] = pageHead{URL: u, Page: pages[u]}
		// A view of the string's bytes: it is only read.
		bodies[i] = unsafe.Slice(unsafe.StringData(heads[i].Body), len(heads[i].Body))
	}
	req := map[string]any{
		"domain": domain,
		"pages":  heads,
		"now":    now.Format(time.RFC3339),
	}
	// The result has an "error" field of its own (why the index couldn't be
	// used), so it can't go through decode, which takes that for a failure
	// of the call and would drop the rest.
	resp, err := runRaw(ctx, func(in *instance) ([]byte, error) {
		return in.callDocs("opencrs_publisher", req, bodies...)
	})
	if err != nil {
		return nil, err
	}
	var head struct {
		Domain *string `json:"domain"`
		Error  *string `json:"error"`
	}
	if json.Unmarshal(resp, &head) == nil && head.Domain == nil && head.Error != nil {
		return nil, &coreError{*head.Error}
	}
	var res PublisherResult
	if err := json.Unmarshal(resp, &res); err != nil {
		return nil, err
	}
	if res.Advice == nil {
		res.Advice = []Advice{}
	}
	if res.Wanted == nil {
		res.Wanted = []string{}
	}
	if res.WantedDetail == nil {
		res.WantedDetail = []WantedURL{}
	}
	return &res, nil
}

// Legibility works out the minimum text sizes for a screen seen from
// req.DistanceM, and the word limit for a dwell time, as the legibility check
// does. Inputs that make no sense are an error.
func Legibility(req LegibilityRequest) (*LegibilityResult, error) {
	return LegibilityContext(context.Background(), req)
}

// LegibilityContext is Legibility with a context; see LintContext.
func LegibilityContext(ctx context.Context, req LegibilityRequest) (*LegibilityResult, error) {
	var res LegibilityResult
	if err := run(ctx, "opencrs_legibility", req, &res); err != nil {
		return nil, err
	}
	return &res, nil
}

// Dwell works out what a driver at req.SpeedKMH can read of a sign seen from
// req.DistanceM, and the limits each named built-in rule pack sets at that
// speed. An unknown pack, or inputs that make no sense, are an error.
func Dwell(req DwellRequest) (*DwellResult, error) {
	return DwellContext(context.Background(), req)
}

// DwellContext is Dwell with a context; see LintContext.
func DwellContext(ctx context.Context, req DwellRequest) (*DwellResult, error) {
	var res DwellResult
	if err := run(ctx, "opencrs_dwell", req, &res); err != nil {
		return nil, err
	}
	if res.Packs == nil {
		res.Packs = []PackLimits{}
	}
	return &res, nil
}

// RulePacks lists the rule packs built into the module, in ref order.
func RulePacks() ([]RulePack, error) {
	return RulePacksContext(context.Background())
}

// RulePacksContext is RulePacks with a context; see LintContext.
func RulePacksContext(ctx context.Context) ([]RulePack, error) {
	var res struct {
		RulePacks []RulePack `json:"rule_packs"`
	}
	if err := run(ctx, "opencrs_rule_packs", struct{}{}, &res); err != nil {
		return nil, err
	}
	return res.RulePacks, nil
}
