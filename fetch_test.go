package opencrs

import (
	"bytes"
	"compress/gzip"
	"context"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"strings"
	"testing"
	"time"
)

func TestRefusedAddresses(t *testing.T) {
	for addr, want := range map[string]string{
		"127.0.0.1":              "loopback",
		"127.255.0.9":            "loopback",
		"10.1.2.3":               "private",
		"172.16.0.1":             "private",
		"172.31.255.255":         "private",
		"192.168.1.1":            "private",
		"169.254.169.254":        "link-local",
		"100.64.0.1":             "CGNAT",
		"100.127.255.255":        "CGNAT",
		"0.0.0.0":                "unspecified",
		"192.0.2.7":              "documentation",
		"198.51.100.7":           "documentation",
		"203.0.113.7":            "documentation",
		"198.18.0.1":             "benchmarking",
		"224.0.0.1":              "multicast",
		"255.255.255.255":        "reserved",
		"240.0.0.1":              "reserved",
		"::1":                    "loopback",
		"::":                     "unspecified",
		"fd00::1":                "private",
		"fe80::1":                "link-local",
		"ff02::1":                "multicast",
		"2001:db8::1":            "documentation",
		"::ffff:10.0.0.1":        "private",
		"::ffff:127.0.0.1":       "loopback",
		"::ffff:169.254.169.254": "link-local",
		"64:ff9b::a00:1":         "private",    // NAT64 around 10.0.0.1
		"2002:a9fe:a9fe::1":      "link-local", // 6to4 around 169.254.169.254
	} {
		if got := refusedAddress(netip.MustParseAddr(addr)); !strings.Contains(got, want) {
			t.Errorf("%s: refused as %q, want %q", addr, got, want)
		}
	}
	for _, addr := range []string{"8.8.8.8", "1.1.1.1", "93.184.216.34", "172.32.0.1", "100.128.0.1", "2606:4700:4700::1111", "::ffff:8.8.8.8", "64:ff9b::808:808"} {
		if got := refusedAddress(netip.MustParseAddr(addr)); got != "" {
			t.Errorf("%s: refused as %q", addr, got)
		}
	}
}

// A server that answers by path, and a client that is SafeClient except that
// it lets loopback through (so the server can be reached) and trusts the
// server's certificate.
func testServer(t *testing.T, h http.Handler) (*httptest.Server, *http.Client) {
	t.Helper()
	srv := httptest.NewTLSServer(h)
	t.Cleanup(srv.Close)
	pool := x509.NewCertPool()
	pool.AddCert(srv.Certificate())
	c := safeClient(func(ip netip.Addr) bool { return ip.IsLoopback() }, &tls.Config{RootCAs: pool})
	return srv, c
}

func TestSafeClientRefusesNonPublicAddresses(t *testing.T) {
	srv, _ := testServer(t, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		t.Error("the server was reached")
	}))
	_, port, _ := net.SplitHostPort(srv.Listener.Addr().String())
	// The server is on loopback: by address, by name, and in IPv6 and
	// IPv4-mapped forms. The unmodified client must not connect.
	for _, host := range []string{"127.0.0.1", "localhost", "[::ffff:127.0.0.1]", "2130706433", "0x7f.1"} {
		u := "https://" + host + ":" + port + "/x"
		p := FetchPage(context.Background(), SafeClient(), WantedURL{URL: u, Kind: "document"}, 0)
		if p.Status != 0 || p.Error == "" || p.Body != "" {
			t.Errorf("%s: page = %+v", u, p)
		}
		if host == "127.0.0.1" && !strings.Contains(p.Error, "loopback") {
			t.Errorf("%s: error %q doesn't say why", u, p.Error)
		}
	}
	// Other private and metadata addresses never get as far as a connection
	// that works; they are refused before it, not by a timeout.
	for _, host := range []string{"169.254.169.254", "10.0.0.5", "192.168.0.1", "100.64.0.1", "[fd00::1]"} {
		start := time.Now()
		p := FetchPage(context.Background(), SafeClient(), WantedURL{URL: "https://" + host + "/latest/meta-data/"}, 0)
		if !strings.Contains(p.Error, "refusing to connect") || time.Since(start) > 5*time.Second {
			t.Errorf("%s: %+v after %v", host, p, time.Since(start))
		}
	}
	// And FetchPage itself won't take http, or a name in the URL.
	for _, u := range []string{"http://example.com/x", "https://user:pw@example.com/x", "ftp://example.com/x", "https:///x"} {
		if p := FetchPage(context.Background(), SafeClient(), WantedURL{URL: u}, 0); p.Error == "" || p.Status != 0 {
			t.Errorf("%s: page = %+v", u, p)
		}
	}
}

func TestFetchPageReadsWhatCheckLooksAt(t *testing.T) {
	doc := `{"hello":"world"}`
	srv, c := testServer(t, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/ok":
			w.Header().Set("ETag", `"v1"`)
			w.Header().Set("Cache-Control", "max-age=60")
			w.Header().Set("Content-Type", "application/json")
			w.Header().Set("Access-Control-Allow-Origin", "*")
			fmt.Fprint(w, doc)
		case "/gone":
			http.Error(w, "no", http.StatusNotFound)
		case "/big":
			fmt.Fprint(w, strings.Repeat("x", 1000))
		case "/redirect":
			http.Redirect(w, r, "/ok", http.StatusFound)
		case "/to-http":
			http.Redirect(w, r, "http://"+r.Host+"/ok", http.StatusFound)
		case "/to-other-host":
			http.Redirect(w, r, "https://example.com:1/ok", http.StatusFound)
		case "/to-private":
			http.Redirect(w, r, "https://10.9.9.9:1/ok", http.StatusFound)
		case "/loop":
			http.Redirect(w, r, "/loop", http.StatusFound)
		}
	}))
	get := func(path string, same bool, max int64) Page {
		return FetchPage(context.Background(), c, WantedURL{URL: srv.URL + path, SameHost: same}, max)
	}
	p := get("/ok", false, 0)
	if p.Status != 200 || p.Body != doc || p.ETag != `"v1"` || p.CacheControl != "max-age=60" ||
		p.ContentType != "application/json" || p.AllowOrigin != "*" || p.Error != "" {
		t.Errorf("/ok = %+v", p)
	}
	if p := get("/gone", false, 0); p.Status != 404 || p.Body != "" || p.Error != "" {
		t.Errorf("/gone = %+v", p)
	}
	if p := get("/big", false, 999); p.Error == "" || !strings.Contains(p.Error, "larger than the 999-byte limit") || p.Body != "" {
		t.Errorf("/big (999) = %+v", p)
	}
	if p := get("/big", false, 1000); p.Error != "" || len(p.Body) != 1000 {
		t.Errorf("/big (1000) = %+v", p)
	}
	if p := get("/redirect", true, 0); p.Body != doc {
		t.Errorf("a redirect on the same host: %+v", p)
	}
	if p := get("/to-http", false, 0); !strings.Contains(p.Error, "not HTTPS") {
		t.Errorf("a redirect to http: %+v", p)
	}
	if p := get("/to-other-host", true, 0); !strings.Contains(p.Error, "another host") {
		t.Errorf("a redirect to another host for a SameHost URL: %+v", p)
	}
	// Without SameHost the redirect is followed, and checked on its own.
	if p := get("/to-private", false, 0); !strings.Contains(p.Error, "refusing to connect to 10.9.9.9") {
		t.Errorf("a redirect to another host for a document: %+v", p)
	}
	if p := get("/loop", false, 0); !strings.Contains(p.Error, "too many redirects") {
		t.Errorf("a redirect loop: %+v", p)
	}
}

// A redirect to a private address is refused when it is followed, not only
// the first request.
func TestRedirectToAPrivateAddressIsRefused(t *testing.T) {
	var hit bool
	srv := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/inner" {
			hit = true
			return
		}
		http.Redirect(w, r, "https://169.254.169.254/latest/meta-data/", http.StatusFound)
	}))
	defer srv.Close()
	pool := x509.NewCertPool()
	pool.AddCert(srv.Certificate())
	// Loopback is allowed so the first hop works; nothing else is.
	c := safeClient(func(ip netip.Addr) bool { return ip.IsLoopback() }, &tls.Config{RootCAs: pool})
	p := FetchPage(context.Background(), c, WantedURL{URL: srv.URL + "/start"}, 0)
	if !strings.Contains(p.Error, "refusing to connect to 169.254.169.254") {
		t.Errorf("page = %+v", p)
	}
	if hit {
		t.Error("inner path reached")
	}
}

// FetchPublisher runs the whole loop against a server that stands in for the
// publisher's domain and its delegate, and stops at its caps.
func TestFetchPublisherLoop(t *testing.T) {
	index, _ := json.Marshal(map[string]any{
		"opencrs":   "0.1",
		"publisher": map[string]string{"id": "p.example", "name": "P"},
		"documents": []map[string]string{{"url": "https://cdn.example/a.screens.json"}},
		"delegates": []map[string]string{{"domain": "d.example"}},
	})
	delegateIndex, _ := json.Marshal(map[string]any{
		"opencrs":   "0.1",
		"publisher": map[string]string{"id": "d.example", "name": "D"},
		"documents": []map[string]string{{"url": "https://d.example/b.screens.json"}},
	})
	doc := func(name string) string {
		return strings.Replace(string(harbourline(t, "retail.screens.json")), "harbourline.example", "p.example", -1)
	}
	var asked []string
	srv := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		asked = append(asked, r.Host+r.URL.Path)
		switch r.Host + r.URL.Path {
		case "p.example/.well-known/opencrs.json":
			w.Write(index)
		case "d.example/.well-known/opencrs.json":
			w.Write(delegateIndex)
		case "cdn.example/a.screens.json", "d.example/b.screens.json":
			fmt.Fprint(w, doc(r.Host))
		default:
			http.NotFound(w, r)
		}
	}))
	defer srv.Close()
	// Every name goes to the test server.
	tr := &http.Transport{
		DialContext: func(ctx context.Context, network, _ string) (net.Conn, error) {
			return (&net.Dialer{}).DialContext(ctx, network, srv.Listener.Addr().String())
		},
		TLSClientConfig:    &tls.Config{InsecureSkipVerify: true},
		DisableCompression: true,
	}
	client := &http.Client{Transport: tr, CheckRedirect: checkRedirect}

	res, err := FetchPublisher(context.Background(), "p.example", fixedNow, &FetchOptions{Client: client})
	if err != nil {
		t.Fatal(err)
	}
	if len(res.Wanted) != 0 || res.Error != "" {
		t.Fatalf("result = %+v", res)
	}
	if res.Discovery == nil || len(res.Discovery.Documents) != 2 {
		t.Fatalf("discovery = %+v", res.Discovery)
	}
	// Both documents name the publisher: one is listed in its own index, one
	// by the delegate it names.
	for _, d := range res.Discovery.Documents {
		if d.Status != "trusted" {
			t.Errorf("%s: %s %s", d.URL, d.Status, d.Reason)
		}
	}
	want := []string{"p.example/.well-known/opencrs.json", "cdn.example/a.screens.json", "d.example/.well-known/opencrs.json", "d.example/b.screens.json"}
	if fmt.Sprint(asked) != fmt.Sprint(want) {
		t.Errorf("fetched %v, want %v", asked, want)
	}

	// Past the last round the caller gets the result so far and an error.
	res, err = FetchPublisher(context.Background(), "p.example", fixedNow, &FetchOptions{Client: client, MaxRounds: 1})
	if !errors.Is(err, ErrTooManyRounds) || res == nil || len(res.Wanted) == 0 {
		t.Errorf("one round: %v, %+v", err, res)
	}
	// Past the page cap the rest are recorded as not fetched, not fetched.
	asked = nil
	res, err = FetchPublisher(context.Background(), "p.example", fixedNow, &FetchOptions{Client: client, MaxPages: 1})
	if err != nil || len(asked) != 1 || !strings.Contains(res.Advice[0].Message+res.Error+fmt.Sprint(res.Discovery.Documents[0].Reason), "not fetched") {
		t.Errorf("one page: %v, asked %v, %+v", err, asked, res)
	}
	// The total cap: nothing is read past it.
	asked = nil
	res, err = FetchPublisher(context.Background(), "p.example", fixedNow, &FetchOptions{Client: client, MaxTotalBytes: int64(len(index)) + 10})
	if err != nil || len(res.Discovery.Documents) != 1 || res.Discovery.Documents[0].Status != "failed" ||
		!strings.Contains(res.Discovery.Documents[0].Reason, "larger than the 10-byte limit") {
		t.Errorf("total cap: %v, %+v", err, res.Discovery)
	}
	// A context that ends is an error.
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := FetchPublisher(ctx, "p.example", fixedNow, &FetchOptions{Client: client}); !errors.Is(err, context.Canceled) {
		t.Errorf("cancelled: %v", err)
	}
}

// A page that is too big, or is compressed and never ends, still costs what
// it read, and FetchPublisher counts that against its total.
func TestFetchPageCountsWhatItReadHoweverItEnded(t *testing.T) {
	gz := func(b []byte) []byte {
		var buf bytes.Buffer
		zw := gzip.NewWriter(&buf)
		zw.Write(b)
		zw.Close()
		return buf.Bytes()
	}
	srv, c := testServer(t, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/big":
			fmt.Fprint(w, strings.Repeat("x", 10000))
		case "/blocks":
			// Gzip, then twenty thousand empty stored blocks: nothing ever
			// decodes, and it never ends.
			b := []byte{0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff}
			for range 20000 {
				b = append(b, 0, 0, 0, 0xff, 0xff)
			}
			w.Header().Set("Content-Encoding", "gzip")
			w.Write(b)
		case "/bomb":
			w.Header().Set("Content-Encoding", "gzip")
			w.Write(gz(bytes.Repeat([]byte("x"), 1<<20)))
		case "/two":
			w.Header().Set("Content-Encoding", "gzip")
			w.Write(append(gz([]byte(`{"a":`)), gz([]byte(`1}`))...))
		case "/garbage":
			w.Header().Set("Content-Encoding", "gzip")
			w.Write(append(gz([]byte(`{}`)), "garbage-after-the-member"...))
		}
	}))
	fetch := func(path string) (Page, int64) {
		return fetchPage(context.Background(), c, WantedURL{URL: srv.URL + path}, 1000)
	}
	if p, n := fetch("/big"); !strings.Contains(p.Error, "larger than the 1000-byte limit") || n < 1001 {
		t.Errorf("/big: %+v after %d bytes", p, n)
	}
	// Capped near the limit plus a little, not the hundred kilobytes sent.
	if p, n := fetch("/blocks"); !strings.Contains(p.Error, "larger than the 1000-byte limit") || n < 4000 || n > 20000 {
		t.Errorf("/blocks: %+v after %d bytes", p, n)
	}
	if p, n := fetch("/bomb"); !strings.Contains(p.Error, "larger than") || n <= 1001 {
		t.Errorf("/bomb: %+v after %d bytes", p, n)
	}
	// Every member is read, and anything after them is an error.
	if p, _ := fetch("/two"); p.Error != "" || p.Body != `{"a":1}` {
		t.Errorf("/two: %+v", p)
	}
	if p, _ := fetch("/garbage"); p.Error == "" {
		t.Errorf("/garbage: %+v", p)
	}
}

func TestAzureWireServerIsRefused(t *testing.T) {
	if got := refusedAddress(netip.MustParseAddr("168.63.129.16")); !strings.Contains(got, "metadata") {
		t.Errorf("168.63.129.16: refused as %q", got)
	}
	if got := refusedAddress(netip.MustParseAddr("168.63.129.17")); got != "" {
		t.Errorf("168.63.129.17: refused as %q", got)
	}
}
