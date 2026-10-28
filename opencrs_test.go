package opencrs

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"
	"testing/fstest"
	"time"
)

func harbourline(t *testing.T, name string) []byte {
	t.Helper()
	b, err := os.ReadFile("examples/harbourline/" + name)
	if err != nil {
		t.Fatal(err)
	}
	return b
}

func TestVersion(t *testing.T) {
	if got := Version(); got != "0.1" {
		t.Fatalf("Version() = %q, want 0.1", got)
	}
}

func TestLintCleanDocument(t *testing.T) {
	res, err := Lint(harbourline(t, "retail.screens.json"))
	if err != nil {
		t.Fatal(err)
	}
	if res.Kind != "screens" {
		t.Errorf("kind = %q, want screens", res.Kind)
	}
	if len(res.SchemaErrors) != 0 || len(res.DocumentChecks) != 0 {
		t.Errorf("want no problems, got %+v", res)
	}
	if !res.OK() {
		t.Error("OK() = false")
	}
}

func TestLintReportsSchemaErrors(t *testing.T) {
	// Detected as a screens document, but missing everything it requires.
	res, err := Lint([]byte(`{"opencrs": "0.1", "publisher": "x.example", "networks": 3}`))
	if err != nil {
		t.Fatal(err)
	}
	if len(res.SchemaErrors) == 0 || res.OK() {
		t.Fatalf("want schema errors, got %+v", res)
	}
	if res.SchemaErrors[0].Message == "" {
		t.Errorf("empty message: %+v", res.SchemaErrors[0])
	}
}

func TestLintUnreadable(t *testing.T) {
	if _, err := Lint([]byte("nonsense")); err == nil {
		t.Fatal("want an error")
	}
	if _, err := Lint([]byte("{}")); err == nil {
		t.Fatal("want an error for a document of no kind")
	}
}

func TestProbePNG(t *testing.T) {
	png := harbourline(t, "lunch-portrait.png")
	p, err := Probe(bytes.NewReader(png), int64(len(png)))
	if err != nil {
		t.Fatal(err)
	}
	sum := sha256.Sum256(png)
	if p.SHA256 != hex.EncodeToString(sum[:]) {
		t.Errorf("sha256 = %s", p.SHA256)
	}
	if p.Bytes != int64(len(png)) || p.MediaType != "image/png" {
		t.Errorf("got %+v", p)
	}
	if p.Pixels == nil || p.Pixels[0] <= 0 || p.Pixels[1] <= 0 {
		t.Errorf("pixels = %v", p.Pixels)
	}
}

func TestProbeEmpty(t *testing.T) {
	p, err := Probe(bytes.NewReader(nil), 0)
	if err != nil {
		t.Fatal(err)
	}
	if p.Bytes != 0 || p.SHA256 != "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855" {
		t.Errorf("got %+v", p)
	}
}

// countingReaderAt serves size zero bytes and counts what was read.
type countingReaderAt struct {
	size int64
	read int64
}

func (c *countingReaderAt) ReadAt(p []byte, off int64) (int, error) {
	if off >= c.size {
		return 0, errEOF
	}
	n := int64(len(p))
	if off+n > c.size {
		n = c.size - off
	}
	clear(p[:n])
	c.read += n
	if n < int64(len(p)) {
		return int(n), errEOF
	}
	return int(n), nil
}

func TestLargeFileProbesByRange(t *testing.T) {
	const size = 50 << 20
	r := &countingReaderAt{size: size}
	p, err := Probe(r, size)
	if err != nil {
		t.Fatal(err)
	}
	if p.Bytes != size {
		t.Errorf("bytes = %d", p.Bytes)
	}
	// Hashing reads every byte once; anything beyond that is the session.
	if extra := r.read - size; extra > 64<<10 {
		t.Errorf("read %d bytes besides hashing, want under 64 KiB", extra)
	}
}

func TestConcurrentLint(t *testing.T) {
	doc := harbourline(t, "retail.screens.json")
	var wg sync.WaitGroup
	for range 50 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			res, err := Lint(doc)
			if err != nil || !res.OK() {
				t.Errorf("lint: %v %+v", err, res)
			}
		}()
	}
	wg.Wait()
}

func harbourlineFS(t *testing.T) fstest.MapFS {
	fsys := fstest.MapFS{}
	for _, n := range []string{"retail.screens.json", "creative.json", "lunch-portrait.png", "lunch-portrait-logo-right.png"} {
		fsys[n] = &fstest.MapFile{Data: harbourline(t, n)}
	}
	return fsys
}

var fixedNow = time.Date(2026, 10, 22, 12, 0, 0, 0, time.FixedZone("", 11*3600))

func TestHarbourlineMatchesFixture(t *testing.T) {
	res, err := CheckDir(harbourlineFS(t), []string{"retail.screens.json"}, "creative.json", fixedNow)
	if err != nil {
		t.Fatal(err)
	}
	var want []map[string]any
	if err := json.Unmarshal(harbourline(t, "reports.json"), &want); err != nil {
		t.Fatal(err)
	}
	if len(res.Reports) != len(want) {
		t.Fatalf("%d reports, want %d", len(res.Reports), len(want))
	}
	for i, r := range res.Reports {
		b, _ := json.Marshal(r)
		var got map[string]any
		_ = json.Unmarshal(b, &got)
		delete(got, "validator")
		delete(want[i], "validator")
		if !reflect.DeepEqual(got, want[i]) {
			t.Errorf("report %d differs:\n got %v\nwant %v", i, got, want[i])
		}
	}
}

func TestCheckRequirementsPerTarget(t *testing.T) {
	res, err := Check([][]byte{harbourline(t, "retail.screens.json")}, harbourline(t, "creative.json"), nil, fixedNow)
	if err != nil {
		t.Fatal(err)
	}
	for _, r := range res.Reports {
		if len(res.Requirements[r.Target]) == 0 {
			t.Errorf("no requirements for %s", r.Target)
		}
	}
	// The two totems differ in venue view and viewing, so each has an entry.
	var got [][]string
	for _, g := range res.Requirements["harbourline.example/retail/portrait-totem-75"] {
		got = append(got, g.Screens)
		if g.Screen != "" || len(g.Expand()) != len(g.Screens) {
			t.Errorf("group %+v", g)
		}
	}
	if !reflect.DeepEqual(got, [][]string{{"HC-L1-001"}, {"HC-L1-002"}}) {
		t.Errorf("groups = %v", got)
	}
}

func TestResolve(t *testing.T) {
	reqs, err := Resolve(harbourline(t, "retail.screens.json"), "harbourline.example/retail/portrait-totem-75")
	if err != nil {
		t.Fatal(err)
	}
	if len(reqs) == 0 || reqs[0].Pixels[0] <= 0 {
		t.Errorf("got %+v", reqs)
	}
}

func TestConcurrentChecks(t *testing.T) {
	fsys := harbourlineFS(t)
	var wg sync.WaitGroup
	errs := make(chan error, 50)
	for range 50 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			res, err := CheckDir(fsys, []string{"retail.screens.json"}, "creative.json", fixedNow)
			if err == nil && len(res.Reports) == 0 {
				err = errors.New("no reports")
			}
			errs <- err
		}()
	}
	wg.Wait()
	close(errs)
	for err := range errs {
		if err != nil {
			t.Error(err)
		}
	}
}

func TestCheckDirConfinement(t *testing.T) {
	dir := t.TempDir()
	outside := t.TempDir()
	if err := os.WriteFile(outside+"/secret.png", harbourline(t, "lunch-portrait.png"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(outside+"/secret.png", dir+"/link.png"); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink("../"+filepath.Base(outside)+"/secret.png", dir+"/rel.png"); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(dir+"/adir", 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(dir+"/retail.screens.json", harbourline(t, "retail.screens.json"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(dir+"/inside.png", harbourline(t, "lunch-portrait.png"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink("inside.png", dir+"/ok.png"); err != nil {
		t.Fatal(err)
	}
	manifest := func(name string) []byte {
		var m map[string]any
		if err := json.Unmarshal(harbourline(t, "creative.json"), &m); err != nil {
			t.Fatal(err)
		}
		files := m["files"].([]any)
		files[0].(map[string]any)["file"] = name
		b, _ := json.Marshal(m)
		return b
	}
	run := func(name string) error {
		if err := os.WriteFile(dir+"/creative.json", manifest(name), 0o644); err != nil {
			t.Fatal(err)
		}
		_, err := CheckDir(os.DirFS(dir), []string{"retail.screens.json"}, "creative.json", fixedNow)
		return err
	}
	for _, bad := range []string{"../x", "a/../../x", "/etc/passwd", "link.png", "rel.png", "adir"} {
		if err := run(bad); err == nil {
			t.Errorf("%q: want an error", bad)
		}
	}
	if err := run("ok.png"); err != nil {
		t.Errorf("in-tree symlink: %v", err)
	}
}

// writeCreative writes a copy of the Harbourline manifest naming file.
func writeCreative(t *testing.T, dir, file string) {
	t.Helper()
	var m map[string]any
	if err := json.Unmarshal(harbourline(t, "creative.json"), &m); err != nil {
		t.Fatal(err)
	}
	m["files"].([]any)[0].(map[string]any)["file"] = file
	b, _ := json.Marshal(m)
	for name, data := range map[string][]byte{"creative.json": b, "retail.screens.json": harbourline(t, "retail.screens.json")} {
		if err := os.WriteFile(filepath.Join(dir, name), data, 0o644); err != nil {
			t.Fatal(err)
		}
	}
}

// The review's M2: a link to "." followed by ".." looked inside when the
// target was cleaned as text, but the OS resolves the link first and
// leaves the directory.
func TestCheckDirResolvesSymlinksPhysically(t *testing.T) {
	parent := t.TempDir()
	dir := filepath.Join(parent, "pkg")
	if err := os.Mkdir(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(parent, "secret.png"), harbourline(t, "lunch-portrait.png"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(".", filepath.Join(dir, "d")); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink("d/../secret.png", filepath.Join(dir, "x.png")); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink("d/d/d/../../../secret.png", filepath.Join(dir, "y.png")); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"x.png", "y.png", "d/../secret.png"} {
		writeCreative(t, dir, name)
		if _, err := CheckDir(os.DirFS(dir), []string{"retail.screens.json"}, "creative.json", fixedNow); err == nil {
			t.Errorf("%s: want an error; the file outside was probed", name)
		}
	}
	// A link through a link that stays inside still works.
	if err := os.WriteFile(filepath.Join(dir, "in.png"), harbourline(t, "lunch-portrait.png"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink("d/in.png", filepath.Join(dir, "z.png")); err != nil {
		t.Fatal(err)
	}
	writeCreative(t, dir, "z.png")
	if _, err := CheckDir(os.DirFS(dir), []string{"retail.screens.json"}, "creative.json", fixedNow); err != nil {
		t.Errorf("z.png: %v", err)
	}
	// So does a missing file (judged on declared values), and os.Root.
	writeCreative(t, dir, "missing.png")
	if _, err := CheckDir(os.DirFS(dir), []string{"retail.screens.json"}, "creative.json", fixedNow); err != nil {
		t.Errorf("missing.png: %v", err)
	}
	root, err := os.OpenRoot(dir)
	if err != nil {
		t.Fatal(err)
	}
	defer root.Close()
	writeCreative(t, dir, "x.png")
	if _, err := CheckDir(root.FS(), []string{"retail.screens.json"}, "creative.json", fixedNow); err == nil {
		t.Error("os.Root: want an error for x.png")
	}
}

type shortReader struct{}

func (shortReader) ReadAt(p []byte, off int64) (int, error) { return 0, io.EOF }

func TestProbeShortReadIsAnError(t *testing.T) {
	// Hashing reads nothing (size 4 claimed, 0 given), so use a reader that
	// yields the hash input but fails the header read.
	if _, err := Probe(shortReader{}, 100); err == nil {
		t.Fatal("want an error for a short read")
	}
}

// The final review's I4: a file not found is judged on the manifest's own
// values, and the caller must be able to see that it happened.
func TestCheckDirListsMissingFiles(t *testing.T) {
	dir := t.TempDir()
	writeCreative(t, dir, "lunch-portrait.png")
	if err := os.WriteFile(filepath.Join(dir, "lunch-portrait.png"), harbourline(t, "lunch-portrait.png"), 0o644); err != nil {
		t.Fatal(err)
	}
	res, err := CheckDir(os.DirFS(dir), []string{"retail.screens.json"}, "creative.json", fixedNow)
	if err != nil {
		t.Fatal(err)
	}
	if want := []string{"lunch-portrait-logo-right.png"}; !reflect.DeepEqual(res.Missing, want) {
		t.Errorf("Missing = %q, want %q", res.Missing, want)
	}
	res, err = CheckDir(harbourlineFS(t), []string{"retail.screens.json"}, "creative.json", fixedNow)
	if err != nil {
		t.Fatal(err)
	}
	if res.Missing != nil {
		t.Errorf("Missing = %q, want none", res.Missing)
	}
}

func TestOversizedRequestIsRefusedBeforeAllocating(t *testing.T) {
	big := make([]byte, maxRequest) // untouched, so it costs no memory
	if _, err := Lint(big); err == nil || !strings.Contains(err.Error(), "memory limit") {
		t.Errorf("Lint of %d bytes: err = %v", len(big), err)
	}
	if _, err := Check([][]byte{big}, []byte("{}"), nil, time.Time{}); err == nil || !strings.Contains(err.Error(), "memory limit") {
		t.Errorf("Check: err = %v", err)
	}
	if _, err := Resolve(big, "a.example/n/f"); err == nil || !strings.Contains(err.Error(), "memory limit") {
		t.Errorf("Resolve: err = %v", err)
	}
}

// A reader that is fine for the hash and fails on the probe's range reads.
type panicReaderAt struct {
	data  []byte
	calls int
}

func (r *panicReaderAt) ReadAt(p []byte, off int64) (int, error) {
	r.calls++
	if r.calls > 1 {
		panic("reader failed")
	}
	return bytesReaderAt(r.data).ReadAt(p, off)
}

func TestPanickingReaderDoesNotPoolTheInstance(t *testing.T) {
	idleMu.Lock()
	idle = nil
	idleMu.Unlock()
	png := harbourline(t, "lunch-portrait.png")
	func() {
		defer func() {
			if recover() == nil {
				t.Error("the reader's panic should reach the caller")
			}
		}()
		_, _ = Probe(&panicReaderAt{data: png}, int64(len(png)))
	}()
	idleMu.Lock()
	n := len(idle)
	idleMu.Unlock()
	if n != 0 {
		t.Errorf("%d idle instances after a panic, want 0", n)
	}
	if got := Version(); got != "0.1" {
		t.Errorf("Version() = %q", got)
	}
}

func TestResolveStrictest(t *testing.T) {
	spec := harbourline(t, "retail.screens.json")
	both, err := ResolveStrictest(spec, "harbourline.example/retail/portrait-totem-75")
	if err != nil {
		t.Fatal(err)
	}
	if both.Screens != 2 || both.Ref != "harbourline.example/retail/portrait-totem-75" || both.Pixels != [2]int{1080, 1920} {
		t.Errorf("got %+v", both)
	}
	// The farther totem (3-10 m) sets the text size: the spec's 77 px.
	if both.MinCapHeightPX["headline"] != 77 || both.Sources["min_cap_height_px.headline"] != "HC-L1-002" {
		t.Errorf("headline %d from %q", both.MinCapHeightPX["headline"], both.Sources["min_cap_height_px.headline"])
	}
	if both.SafeArea == nil || both.SafeArea.Top != 120 || len(both.BlockedRegions) != 2 {
		t.Errorf("layout: %+v %+v", both.SafeArea, both.BlockedRegions)
	}
	one, err := ResolveStrictest(spec, "harbourline.example/retail/screen/HC-L1-002")
	if err != nil {
		t.Fatal(err)
	}
	if one.Screens != 1 || !reflect.DeepEqual(one.MinCapHeightPX, both.MinCapHeightPX) {
		t.Errorf("a screen's own: %+v", one)
	}
	if _, err := ResolveStrictest(spec, "harbourline.example/retail/nope"); err == nil || !strings.Contains(err.Error(), "not found") {
		t.Errorf("unknown format: %v", err)
	}
	if _, err := ResolveStrictest([]byte("{}"), "a.example/n/f"); err == nil {
		t.Error("an unreadable document should be an error")
	}
}

func TestResolveStrictestContext(t *testing.T) {
	spec := harbourline(t, "retail.screens.json")
	ref := "harbourline.example/retail/portrait-totem-75"
	if _, err := ResolveStrictestContext(context.Background(), spec, ref); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := ResolveStrictestContext(ctx, spec, ref); !errors.Is(err, context.Canceled) {
		t.Errorf("got %v, want context.Canceled", err)
	}
}
