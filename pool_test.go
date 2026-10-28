package opencrs

import (
	"bytes"
	"context"
	"errors"
	"strings"
	"testing"
	"time"
)

func poolSize() int {
	idleMu.Lock()
	defer idleMu.Unlock()
	return len(idle)
}

func emptyPools() {
	idleMu.Lock()
	idle = nil
	idleMu.Unlock()
}

// A reader that serves the hash and then fails the probe's range reads.
type failingReaderAt struct {
	data  []byte
	calls int
}

func (r *failingReaderAt) ReadAt(p []byte, off int64) (int, error) {
	r.calls++
	if r.calls > 1 {
		return 0, errors.New("disk on fire")
	}
	return bytesReaderAt(r.data).ReadAt(p, off)
}

// A probe that fails part way leaves a session open in the module, so its
// instance is closed, never returned to the pool, whichever pool it came
// from.
func TestFailedProbeDoesNotPoolTheInstance(t *testing.T) {
	png := harbourline(t, "lunch-portrait.png")
	live, cancel := context.WithCancel(context.Background())
	defer cancel()
	for name, ctx := range map[string]context.Context{"plain": context.Background(), "interruptible": live} {
		emptyPools()
		_, err := ProbeContext(ctx, &failingReaderAt{data: png}, int64(len(png)))
		if err == nil || !strings.Contains(err.Error(), "disk on fire") {
			t.Errorf("%s: err = %v", name, err)
		}
		if n := poolSize(); n != 0 {
			t.Errorf("%s: pool holds %d after a failed probe, want 0", name, n)
		}
		// And a good probe afterwards works and is kept.
		if _, err := ProbeContext(ctx, bytes.NewReader(png), int64(len(png))); err != nil {
			t.Errorf("%s: probe afterwards: %v", name, err)
		}
		if n := poolSize(); n != 1 {
			t.Errorf("%s: pool holds %d after a good probe, want one instance", name, n)
		}
	}
}

// A context that ends while a call is running stops it, on an instance taken
// from the pool, and the next call is unharmed. (The same for the 50 MB
// document is in opencrs_context_big_test.go, which -race leaves out.)
func TestDeadlineStopsARunningCall(t *testing.T) {
	doc := complexScreens() // about a second of work to resolve
	small := harbourline(t, "retail.screens.json")

	warm, stop := context.WithCancel(context.Background())
	defer stop()
	if _, err := LintContext(warm, small); err != nil {
		t.Fatal(err)
	}
	before := poolSize()
	if before == 0 {
		t.Fatal("no instance is pooled")
	}

	calls := map[string]func(context.Context) error{
		"Resolve": func(ctx context.Context) error {
			_, err := ResolveContext(ctx, doc, "x.example/n/f")
			return err
		},
		"ResolveStrictest": func(ctx context.Context) error {
			_, err := ResolveStrictestContext(ctx, doc, "x.example/n/f")
			return err
		},
	}
	for name, f := range calls {
		ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
		start := time.Now()
		err := f(ctx)
		took := time.Since(start)
		cancel()
		if !errors.Is(err, context.DeadlineExceeded) {
			t.Errorf("%s: err = %v after %v, want DeadlineExceeded", name, err, took)
		}
		if errors.Is(err, ErrTooComplex) {
			t.Errorf("%s: a stopped call is not too complex", name)
		}
		// The stopped call's instance is not kept, and the pool still serves.
		if after := poolSize(); after > before {
			t.Errorf("%s: pool grew from %d to %d", name, before, after)
		}
		res, err := LintContext(warm, small)
		if err != nil || !res.OK() {
			t.Errorf("%s: lint afterwards: %v", name, err)
		}
	}
}
