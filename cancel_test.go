package opencrs

import (
	"context"
	"errors"
	"testing"
	"time"
)

// A call cancelled while it runs returns an error wrapping context.Canceled
// (not DeadlineExceeded), promptly, and leaves the package usable.
func TestCancelStopsARunningCall(t *testing.T) {
	doc := complexScreens() // over 100 ms of work to resolve, once warm
	if _, err := Resolve(doc, "x.example/n/f"); !errors.Is(err, ErrTooComplex) {
		t.Fatal(err) // also loads the module, which takes longer than the call
	}
	start := time.Now()
	_, _ = Resolve(doc, "x.example/n/f")
	whole := time.Since(start)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	timer := time.AfterFunc(30*time.Millisecond, cancel)
	defer timer.Stop()

	start = time.Now()
	_, err := ResolveContext(ctx, doc, "x.example/n/f")
	took := time.Since(start)
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("err = %v after %v, want context.Canceled", err, took)
	}
	if errors.Is(err, context.DeadlineExceeded) || errors.Is(err, ErrTooComplex) {
		t.Errorf("err = %v matches the wrong sentinel", err)
	}
	if took > whole*3/4 {
		t.Errorf("stopped after %v; the whole call takes %v", took, whole)
	}
	res, err := Lint(harbourline(t, "retail.screens.json"))
	if err != nil || !res.OK() {
		t.Errorf("lint afterwards: %v", err)
	}
}
