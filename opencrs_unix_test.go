//go:build unix

package opencrs

import (
	"os"
	"path/filepath"
	"syscall"
	"testing"
	"time"
)

// The review's L1: a FIFO named in the manifest blocked the open forever.
func TestCheckDirRefusesAFIFOWithoutOpeningIt(t *testing.T) {
	dir := t.TempDir()
	if err := syscall.Mkfifo(filepath.Join(dir, "pipe.png"), 0o644); err != nil {
		t.Skipf("mkfifo: %v", err)
	}
	writeCreative(t, dir, "pipe.png")
	done := make(chan error, 1)
	go func() {
		_, err := CheckDir(os.DirFS(dir), []string{"retail.screens.json"}, "creative.json", fixedNow)
		done <- err
	}()
	select {
	case err := <-done:
		if err == nil {
			t.Fatal("want an error for a FIFO")
		}
	case <-time.After(10 * time.Second):
		t.Fatal("CheckDir blocked opening a FIFO")
	}
}
