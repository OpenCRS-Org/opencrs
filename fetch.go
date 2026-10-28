package opencrs

import (
	"compress/gzip"
	"context"
	"crypto/tls"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/netip"
	"net/url"
	"strings"
	"syscall"
	"time"
)

// The publisher check fetches nothing itself: the URLs it wants come from
// the publisher's own files, so they can be anything, including addresses
// inside your network. FetchPublisher and FetchPage fetch them the way the
// command line's fetch does, and SafeClient is the client they use.

const (
	// MaxDocumentBytes is the spec's limit on one document.
	MaxDocumentBytes = 50 << 20

	// DefaultMaxTotalBytes is how much FetchPublisher reads in all. The
	// pages are held in the module's memory together, which is 256 MiB: four
	// documents of 50 MB fit, five do not, and this leaves room for the
	// work on them.
	DefaultMaxTotalBytes = 180 << 20

	// DefaultMaxRounds is how many times FetchPublisher asks the core. The
	// core needs at most three: the index, then its documents and delegate
	// indexes, then the delegates' documents.
	DefaultMaxRounds = 5

	// DefaultMaxPages is how many URLs FetchPublisher fetches in all.
	DefaultMaxPages = 4000

	maxRedirects = 5 // requests in a chain, as the command line allows
)

// FetchOptions controls FetchPublisher. The zero value is a good default.
type FetchOptions struct {
	// Client fetches each URL. Nil means SafeClient(). Anything else must
	// be as careful as SafeClient: HTTPS only, no connection to a
	// non-public address (checked after DNS, on every redirect), no proxy.
	Client *http.Client
	// MaxDocumentBytes is the most one page may be; zero means
	// MaxDocumentBytes (50 MiB).
	MaxDocumentBytes int64
	// MaxTotalBytes is the most all pages may be together; zero means
	// DefaultMaxTotalBytes. A page past it is recorded as not fetched.
	MaxTotalBytes int64
	// MaxRounds is how many times the core is asked; zero means
	// DefaultMaxRounds.
	MaxRounds int
	// MaxPages is how many URLs are fetched in all; zero means
	// DefaultMaxPages. A URL past it is recorded as not fetched.
	MaxPages int
}

// ErrTooManyRounds is wrapped by the error FetchPublisher returns when the
// core still wanted URLs after MaxRounds rounds. The result is returned with
// it.
var ErrTooManyRounds = errors.New("opencrs: the publisher check still wanted URLs after its last round")

// FetchPublisher runs the publisher check end to end: it asks the core which
// URLs it wants, fetches them with FetchPage, and asks again, until nothing
// more is wanted or opts.MaxRounds is reached. Every URL was chosen by the
// publisher, so the fetching is done as SafeClient and FetchPage do it. A
// fetch that fails is recorded as a failed page, which the result reports
// against its document; only a failure of the check itself, or ctx ending, is
// an error. A zero now means the current time.
func FetchPublisher(ctx context.Context, domain string, now time.Time, opts *FetchOptions) (*PublisherResult, error) {
	var o FetchOptions
	if opts != nil {
		o = *opts
	}
	if o.Client == nil {
		o.Client = SafeClient()
	}
	if o.MaxDocumentBytes <= 0 {
		o.MaxDocumentBytes = MaxDocumentBytes
	}
	if o.MaxTotalBytes <= 0 {
		o.MaxTotalBytes = DefaultMaxTotalBytes
	}
	if o.MaxRounds <= 0 {
		o.MaxRounds = DefaultMaxRounds
	}
	if o.MaxPages <= 0 {
		o.MaxPages = DefaultMaxPages
	}
	pages := map[string]Page{}
	var total int64
	var res *PublisherResult
	for range o.MaxRounds {
		var err error
		res, err = PublisherContext(ctx, domain, pages, now)
		if err != nil {
			return nil, err
		}
		if len(res.Wanted) == 0 {
			return res, nil
		}
		for _, w := range res.WantedDetail {
			if _, have := pages[w.URL]; have {
				continue // never fetched twice, whatever the core says
			}
			switch {
			case len(pages) >= o.MaxPages:
				pages[w.URL] = Page{Error: fmt.Sprintf("not fetched: a check fetches at most %d URLs", o.MaxPages)}
			case total >= o.MaxTotalBytes:
				pages[w.URL] = Page{Error: fmt.Sprintf("not fetched: a check reads at most %d bytes in all", o.MaxTotalBytes)}
			default:
				limit := min(o.MaxDocumentBytes, o.MaxTotalBytes-total)
				p, read := fetchPage(ctx, o.Client, w, limit)
				total += read
				pages[w.URL] = p
			}
			if err := ctx.Err(); err != nil {
				return nil, fmt.Errorf("opencrs: %w", err)
			}
		}
	}
	return res, fmt.Errorf("%w (%d rounds)", ErrTooManyRounds, o.MaxRounds)
}

type sameHostKey struct{}

// FetchPage fetches one wanted URL with c, which should be SafeClient() or
// as careful, and returns it as a Page to add to the pages under w.URL
// exactly. It never fails: a fetch that did not work is a Page with Status 0
// and Error set, and a response that is not 200 is a Page with its Status. A
// page of more than maxBytes (zero means MaxDocumentBytes) is an error page.
// Only HTTPS is fetched, with no user name or password; w.SameHost makes a
// redirect to another host an error. The body is read as served, with gzip
// undone, and the headers the check looks at are kept.
func FetchPage(ctx context.Context, c *http.Client, w WantedURL, maxBytes int64) Page {
	p, _ := fetchPage(ctx, c, w, maxBytes)
	return p
}

// countingReader adds what it reads to *n.
type countingReader struct {
	r io.Reader
	n *int64
}

func (c countingReader) Read(p []byte) (int, error) {
	n, err := c.r.Read(p)
	*c.n += int64(n)
	return n, err
}

// fetchPage is FetchPage that also says how many bytes it read, from the
// network and, for a gzipped body, again decompressed, however it ended: a
// page that was too big, or never decoded, still cost what it cost, and
// FetchPublisher counts that against its total.
func fetchPage(ctx context.Context, c *http.Client, w WantedURL, maxBytes int64) (Page, int64) {
	if maxBytes <= 0 {
		maxBytes = MaxDocumentBytes
	}
	fail := func(err error) (Page, int64) { return Page{Error: err.Error()}, 0 }
	u, err := url.Parse(w.URL)
	if err != nil {
		return fail(err)
	}
	if u.Scheme != "https" || u.User != nil || u.Host == "" {
		return Page{Error: fmt.Sprintf("%s: only https:// URLs without a user name can be fetched", u.Redacted())}, 0
	}
	if w.SameHost {
		ctx = context.WithValue(ctx, sameHostKey{}, true)
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, w.URL, nil)
	if err != nil {
		return fail(err)
	}
	req.Header.Set("Accept", "application/json")
	req.Header.Set("Accept-Encoding", "gzip")
	req.Header.Set("User-Agent", "opencrs-go/0.1")
	resp, err := c.Do(req)
	if err != nil {
		return fail(err)
	}
	defer resp.Body.Close()
	p := Page{
		Status:          resp.StatusCode,
		ETag:            resp.Header.Get("ETag"),
		CacheControl:    resp.Header.Get("Cache-Control"),
		LastModified:    resp.Header.Get("Last-Modified"),
		ContentType:     resp.Header.Get("Content-Type"),
		ContentEncoding: resp.Header.Get("Content-Encoding"),
		AllowOrigin:     resp.Header.Get("Access-Control-Allow-Origin"),
	}
	if resp.StatusCode != http.StatusOK {
		return p, 0
	}
	var read, wire int64
	var body io.Reader = countingReader{resp.Body, &read}
	gz := strings.EqualFold(p.ContentEncoding, "gzip")
	// What goes over the wire is capped too: compressed data can stream
	// without ever growing the decoded body (empty deflate blocks, say), and
	// incompressible data comes out a little larger than it went in.
	wireCap := maxBytes + maxBytes/16 + 4096
	if gz {
		body = countingReader{io.LimitReader(countingReader{resp.Body, &wire}, wireCap), &read}
		zr, err := gzip.NewReader(body)
		if err != nil {
			return Page{Error: err.Error()}, read
		}
		defer zr.Close()
		body = countingReader{zr, &read}
	}
	tooBig := func() (Page, int64) {
		return Page{Error: fmt.Sprintf("%s: larger than the %d-byte limit", u.Redacted(), maxBytes)}, read
	}
	b, err := io.ReadAll(io.LimitReader(body, maxBytes+1))
	if err != nil {
		if gz && wire >= wireCap {
			return tooBig()
		}
		return Page{Error: err.Error()}, read
	}
	if int64(len(b)) > maxBytes {
		return tooBig()
	}
	p.Body = string(b)
	return p, read
}

// SafeClient returns an HTTP client for fetching URLs a publisher chose. It
// connects only to public addresses: every address a name resolves to is
// checked as the connection is made, so a name that resolves to loopback, a
// private network, a link-local (cloud metadata) address, a shared (CGNAT),
// multicast, documentation or other reserved address is refused, and so is a
// redirect to one. It follows at most four redirects, never from HTTPS to
// HTTP, and, for a request made with FetchPage for a WantedURL with
// SameHost, never to another host. It ignores HTTP_PROXY and HTTPS_PROXY
// (a proxy would resolve names where they can't be checked), has a 2 minute
// timeout for a whole request, and leaves gzip to FetchPage.
func SafeClient() *http.Client {
	return safeClient(nil, nil)
}

// safeClient is SafeClient with a way to let some addresses through and a TLS
// config, for tests.
func safeClient(allow func(netip.Addr) bool, tlsConfig *tls.Config) *http.Client {
	d := &net.Dialer{
		Timeout:   30 * time.Second,
		KeepAlive: 30 * time.Second,
		// Control runs for every address a name resolves to, just before
		// the connection, so what is checked is what is connected to.
		Control: func(_, address string, _ syscall.RawConn) error {
			host, _, err := net.SplitHostPort(address)
			if err != nil {
				return err
			}
			ip, err := netip.ParseAddr(host)
			if err != nil {
				return err
			}
			if allow != nil && allow(ip) {
				return nil
			}
			if why := refusedAddress(ip); why != "" {
				return fmt.Errorf("refusing to connect to %s: it is %s", ip, why)
			}
			return nil
		},
	}
	return &http.Client{
		Timeout: 2 * time.Minute,
		Transport: &http.Transport{
			Proxy:                  nil,
			DialContext:            d.DialContext,
			TLSClientConfig:        tlsConfig,
			TLSHandshakeTimeout:    15 * time.Second,
			ResponseHeaderTimeout:  30 * time.Second,
			MaxResponseHeaderBytes: 64 << 10,
			DisableCompression:     true,
			MaxIdleConns:           4,
			IdleConnTimeout:        30 * time.Second,
			ForceAttemptHTTP2:      true,
		},
		CheckRedirect: checkRedirect,
	}
}

// checkRedirect refuses a redirect that leaves HTTPS, goes on too long, or
// (for a request marked SameHost) changes host.
func checkRedirect(req *http.Request, via []*http.Request) error {
	if len(via) >= maxRedirects {
		return errors.New("too many redirects")
	}
	prev := via[len(via)-1]
	if req.URL.Scheme != "https" || prev.URL.Scheme != "https" {
		return fmt.Errorf("refusing redirect from %s to %s: not HTTPS", prev.URL.Redacted(), req.URL.Redacted())
	}
	if req.URL.User != nil {
		return fmt.Errorf("refusing redirect to %s: it has a user name", req.URL.Redacted())
	}
	if same, _ := req.Context().Value(sameHostKey{}).(bool); same && !strings.EqualFold(via[0].URL.Host, req.URL.Host) {
		return fmt.Errorf("refusing redirect from %s to another host, %s", via[0].URL.Host, req.URL.Host)
	}
	return nil
}

var refusedV4 = []struct {
	prefix netip.Prefix
	why    string
}{
	{netip.MustParsePrefix("0.0.0.0/8"), "an unspecified address"},
	{netip.MustParsePrefix("10.0.0.0/8"), "a private address"},
	{netip.MustParsePrefix("100.64.0.0/10"), "a shared (CGNAT) address"},
	{netip.MustParsePrefix("127.0.0.0/8"), "a loopback address"},
	{netip.MustParsePrefix("168.63.129.16/32"), "a cloud metadata address"}, // Azure's WireServer: public, but only answers the VM beside it
	{netip.MustParsePrefix("169.254.0.0/16"), "a link-local address"},
	{netip.MustParsePrefix("172.16.0.0/12"), "a private address"},
	{netip.MustParsePrefix("192.0.0.0/24"), "an IETF protocol address"},
	{netip.MustParsePrefix("192.0.2.0/24"), "a documentation address"},
	{netip.MustParsePrefix("192.168.0.0/16"), "a private address"},
	{netip.MustParsePrefix("198.18.0.0/15"), "a benchmarking address"},
	{netip.MustParsePrefix("198.51.100.0/24"), "a documentation address"},
	{netip.MustParsePrefix("203.0.113.0/24"), "a documentation address"},
	{netip.MustParsePrefix("224.0.0.0/4"), "a multicast address"},
	{netip.MustParsePrefix("240.0.0.0/4"), "a reserved or broadcast address"},
}

var refusedV6 = []struct {
	prefix netip.Prefix
	why    string
}{
	{netip.MustParsePrefix("::/128"), "an unspecified address"},
	{netip.MustParsePrefix("::1/128"), "a loopback address"},
	{netip.MustParsePrefix("fc00::/7"), "a private (unique local) address"},
	{netip.MustParsePrefix("fe80::/10"), "a link-local address"},
	{netip.MustParsePrefix("fec0::/10"), "a site-local address"},
	{netip.MustParsePrefix("ff00::/8"), "a multicast address"},
	{netip.MustParsePrefix("2001:db8::/32"), "a documentation address"},
	{netip.MustParsePrefix("2001::/32"), "a Teredo address"},
	{netip.MustParsePrefix("100::/64"), "a discard address"},
	{netip.MustParsePrefix("64:ff9b:1::/48"), "a local NAT64 address"},
}

// refusedAddress says why ip may not be connected to, or "" if it may. IPv6
// forms that carry an IPv4 address (mapped, compatible, NAT64, 6to4) are
// judged by it. The same list as the command line's fetch.
func refusedAddress(ip netip.Addr) string {
	ip = ip.WithZone("")
	if ip.Is4In6() {
		ip = ip.Unmap()
	}
	if ip.Is4() {
		for _, r := range refusedV4 {
			if r.prefix.Contains(ip) {
				return r.why
			}
		}
		return ""
	}
	b := ip.As16()
	embedded := func(i int) netip.Addr { return netip.AddrFrom4([4]byte{b[i], b[i+1], b[i+2], b[i+3]}) }
	zeros := func(n int) bool {
		for _, x := range b[:n] {
			if x != 0 {
				return false
			}
		}
		return true
	}
	switch {
	case ip == netip.IPv6Unspecified():
		return "an unspecified address"
	case ip == netip.IPv6Loopback():
		return "a loopback address"
	case zeros(12): // ::a.b.c.d, compatible
		return refusedAddress(embedded(12))
	case b[0] == 0 && b[1] == 0x64 && b[2] == 0xff && b[3] == 0x9b && allZero(b[4:12]): // 64:ff9b::/96 (NAT64)
		return refusedAddress(embedded(12))
	case b[0] == 0x20 && b[1] == 0x02: // 2002::/16, 6to4
		return refusedAddress(embedded(2))
	}
	for _, r := range refusedV6 {
		if r.prefix.Contains(ip) {
			return r.why
		}
	}
	return ""
}

func allZero(b []byte) bool {
	for _, x := range b {
		if x != 0 {
			return false
		}
	}
	return true
}
