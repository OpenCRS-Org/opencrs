// Package opencrs checks creative files against OpenCRS screen specs. It
// embeds the OpenCRS core, compiled to WebAssembly, and runs it with wazero,
// so it needs no cgo and behaves the same on every platform.
package opencrs

import (
	"context"
	"crypto/sha256"
	_ "embed"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"path"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/tetratelabs/wazero"
	"github.com/tetratelabs/wazero/api"
)

//go:embed opencrs.wasm
var wasmBytes []byte

var errEOF = io.EOF

// maxMemoryPages caps one instance at 256 MiB (64 KiB pages). Files are
// read by range, never loaded, so this only bounds the documents. At the
// spec's 50 MB limit (190,000 screens) the module peaks at about 55 MiB to
// Lint and 146 MiB to Check; a Check whose target covers all of those
// screens also writes a 73 MB answer and peaks at about 220 MiB. The cap
// leaves room above that. TestLintAndCheckLargeDocument keeps it honest.
const maxMemoryPages = 4096

var (
	once     sync.Once
	runtime_ wazero.Runtime
	compiled wazero.CompiledModule
	loadErr  error
)

func load() (wazero.Runtime, wazero.CompiledModule, error) {
	once.Do(func() {
		ctx := context.Background()
		cfg := wazero.NewRuntimeConfig().
			WithCompilationCache(wazero.NewCompilationCache()).
			WithMemoryLimitPages(maxMemoryPages)
		runtime_ = wazero.NewRuntimeWithConfig(ctx, cfg)
		compiled, loadErr = runtime_.CompileModule(ctx, wasmBytes)
	})
	return runtime_, compiled, loadErr
}

// instance is one module instance. It is not safe for concurrent use; each
// public call takes its own, from idle ones kept for reuse when there are any.
type instance struct {
	ctx context.Context
	mod api.Module
	// dirty is set when a call failed, which can leave the module holding
	// memory or a probe session; such an instance is closed, not reused.
	dirty bool
}

// Setting up a fresh instance costs most of a small call (the module builds
// its schema validators on first use), so a few idle instances are kept. An
// instance that grew past idleMemory pages, or failed, is closed instead.
const (
	maxIdle    = 2
	idleMemory = 256 // 16 MiB
)

var (
	idleMu sync.Mutex
	idle   []*instance
)

func newInstance() (*instance, error) {
	idleMu.Lock()
	if n := len(idle); n > 0 {
		in := idle[n-1]
		idle = idle[:n-1]
		idleMu.Unlock()
		return in, nil
	}
	idleMu.Unlock()
	rt, cm, err := load()
	if err != nil {
		return nil, fmt.Errorf("opencrs: loading the module: %w", err)
	}
	ctx := context.Background()
	mod, err := rt.InstantiateModule(ctx, cm, wazero.NewModuleConfig().WithName("").WithStartFunctions())
	if err != nil {
		return nil, fmt.Errorf("opencrs: instantiating the module: %w", err)
	}
	return &instance{ctx: ctx, mod: mod}, nil
}

// close ends a call: the instance goes back to be reused if it is in a clean
// state and small, and is closed otherwise.
func (in *instance) close() {
	if !in.dirty && in.mod.Memory().Size() <= idleMemory*65536 {
		idleMu.Lock()
		if len(idle) < maxIdle {
			idle = append(idle, in)
			idleMu.Unlock()
			return
		}
		idleMu.Unlock()
	}
	_ = in.mod.Close(in.ctx)
}

func (in *instance) fn(name string) (api.Function, error) {
	f := in.mod.ExportedFunction(name)
	if f == nil {
		return nil, fmt.Errorf("opencrs: module has no export %s", name)
	}
	return f, nil
}

// maxRequest is the most a single request may carry. The module could never
// hold more, and sizes are 32-bit in its ABI.
const maxRequest = maxMemoryPages * 65536

func errTooLarge(n int) error {
	return fmt.Errorf("opencrs: a request of %d bytes is more than the module's %d-byte memory limit", n, maxRequest)
}

// put copies data into a new allocation in module memory.
func (in *instance) put(data []byte) (ptr uint32, err error) {
	if len(data) >= maxRequest {
		return 0, errTooLarge(len(data))
	}
	alloc, err := in.fn("opencrs_alloc")
	if err != nil {
		return 0, err
	}
	r, err := alloc.Call(in.ctx, uint64(len(data)))
	if err != nil {
		return 0, err
	}
	ptr = uint32(r[0])
	if len(data) > 0 && !in.mod.Memory().Write(ptr, data) {
		return 0, errors.New("opencrs: writing to module memory failed")
	}
	return ptr, nil
}

func (in *instance) free(ptr, n uint32) {
	if f, err := in.fn("opencrs_free"); err == nil {
		_, _ = f.Call(in.ctx, uint64(ptr), uint64(n))
	}
}

// take reads and frees a response packed as ptr<<32 | len.
func (in *instance) take(packed uint64) ([]byte, error) {
	ptr, n := uint32(packed>>32), uint32(packed)
	b, ok := in.mod.Memory().Read(ptr, n)
	if !ok {
		return nil, errors.New("opencrs: reading module memory failed")
	}
	out := append([]byte(nil), b...)
	in.free(ptr, n)
	return out, nil
}

// call runs an export that takes a JSON request and returns JSON.
func (in *instance) call(name string, req any) ([]byte, error) {
	data, err := json.Marshal(req)
	if err != nil {
		return nil, err
	}
	f, err := in.fn(name)
	if err != nil {
		return nil, err
	}
	ptr, err := in.put(data)
	if err != nil {
		return nil, err
	}
	defer in.free(ptr, uint32(len(data)))
	r, err := f.Call(in.ctx, uint64(ptr), uint64(len(data)))
	if err != nil {
		return nil, fmt.Errorf("opencrs: %s: %w", name, err)
	}
	return in.take(r[0])
}

// callDocs runs an export that carries documents: a frame of a JSON header
// (req plus the documents' lengths) and then the documents themselves as
// raw bytes, written straight into module memory. The module takes the
// allocation and frees it once the documents are decoded.
func (in *instance) callDocs(name string, req map[string]any, docs ...[]byte) ([]byte, error) {
	lens := make([]int, len(docs))
	total := 4
	for i, d := range docs {
		lens[i] = len(d)
		total += len(d)
	}
	req["blobs"] = lens
	head, err := json.Marshal(req)
	if err != nil {
		return nil, err
	}
	total += len(head)
	if total >= maxRequest {
		return nil, errTooLarge(total)
	}
	f, err := in.fn(name)
	if err != nil {
		return nil, err
	}
	alloc, err := in.fn("opencrs_alloc")
	if err != nil {
		return nil, err
	}
	r, err := alloc.Call(in.ctx, uint64(total))
	if err != nil {
		return nil, err
	}
	ptr := uint32(r[0])
	mem := in.mod.Memory()
	ok := mem.WriteUint32Le(ptr, uint32(len(head))) && mem.Write(ptr+4, head)
	at := ptr + 4 + uint32(len(head))
	for _, d := range docs {
		ok = ok && mem.Write(at, d)
		at += uint32(len(d))
	}
	if !ok {
		return nil, errors.New("opencrs: writing to module memory failed")
	}
	r, err = f.Call(in.ctx, uint64(ptr), uint64(total))
	if err != nil {
		return nil, fmt.Errorf("opencrs: %s: %w", name, err)
	}
	return in.take(r[0])
}

// callRaw runs an export whose request is raw bytes, such as a document.
func (in *instance) callRaw(name string, data []byte) ([]byte, error) {
	f, err := in.fn(name)
	if err != nil {
		return nil, err
	}
	ptr, err := in.put(data)
	if err != nil {
		return nil, err
	}
	defer in.free(ptr, uint32(len(data)))
	r, err := f.Call(in.ctx, uint64(ptr), uint64(len(data)))
	if err != nil {
		return nil, fmt.Errorf("opencrs: %s: %w", name, err)
	}
	return in.take(r[0])
}

// decode unpacks a response into out, turning {"error": ...} into an error.
func decode(resp []byte, out any) error {
	var e struct {
		Error *string `json:"error"`
	}
	if json.Unmarshal(resp, &e) == nil && e.Error != nil {
		return errors.New(*e.Error)
	}
	return json.Unmarshal(resp, out)
}

func run(name string, req, out any) error {
	return runWith(out, func(in *instance) ([]byte, error) { return in.call(name, req) })
}

// runWith makes a fresh instance, makes the call on it and decodes the
// response into out.
func runWith(out any, call func(*instance) ([]byte, error)) error {
	in, err := newInstance()
	if err != nil {
		return err
	}
	defer in.close()
	resp, err := call(in)
	if err != nil {
		in.dirty = true
		return err
	}
	return decode(resp, out)
}

// Version returns the OpenCRS spec version the package implements. It
// panics if the embedded module cannot be loaded.
func Version() string {
	var v struct {
		Version string `json:"version"`
	}
	if err := run("opencrs_version", struct{}{}, &v); err != nil {
		// The module is embedded, so failing to load it is a broken build,
		// not a condition to handle.
		panic(err)
	}
	return v.Version
}

// Lint checks one document of any kind against its schema and, for a valid
// screens document, its own references and rules. An error means the
// document could not be read at all.
func Lint(doc []byte) (*LintResult, error) {
	var res LintResult
	err := runWith(&res, func(in *instance) ([]byte, error) { return in.callRaw("opencrs_lint", doc) })
	if err != nil {
		return nil, err
	}
	return &res, nil
}

// hashChunk is how much Probe reads at a time while hashing.
const hashChunk = 1 << 20

// Probe hashes a file, then answers the module's range requests to read its
// type, dimensions and (for MP4) duration. Only the hash reads the whole
// file; the rest touches the headers. r must hold exactly size bytes.
func Probe(r io.ReaderAt, size int64) (*ProbeResult, error) {
	h := sha256.New()
	if _, err := io.CopyBuffer(h, io.NewSectionReader(r, 0, size), make([]byte, hashChunk)); err != nil {
		return nil, err
	}
	in, err := newInstance()
	if err != nil {
		return nil, err
	}
	defer in.close()
	res, err := probeOn(in, r, size, hex.EncodeToString(h.Sum(nil)))
	if err != nil {
		in.dirty = true // a session may be left open
	}
	return res, err
}

type probeStep struct {
	Session uint32 `json:"session"`
	Need    *struct {
		Offset int64 `json:"offset"`
		Len    int64 `json:"len"`
	} `json:"need"`
	Probe *ProbeResult `json:"probe"`
}

func probeOn(in *instance, r io.ReaderAt, size int64, sum string) (*ProbeResult, error) {
	resp, err := in.call("opencrs_probe_start", map[string]any{"size": size, "sha256": sum})
	if err != nil {
		return nil, err
	}
	feed, err := in.fn("opencrs_probe_feed")
	if err != nil {
		return nil, err
	}
	for {
		var st probeStep
		if err := decode(resp, &st); err != nil {
			return nil, err
		}
		if st.Need == nil {
			if st.Probe == nil {
				return nil, errors.New("opencrs: probe finished without a result")
			}
			return st.Probe, nil
		}
		n := min(st.Need.Len, size-st.Need.Offset)
		if st.Need.Offset < 0 || n < 0 {
			return nil, fmt.Errorf("opencrs: probe asked for bytes %d+%d of %d", st.Need.Offset, st.Need.Len, size)
		}
		buf := make([]byte, n)
		if got, err := r.ReadAt(buf, st.Need.Offset); got < len(buf) {
			if err == nil || errors.Is(err, io.EOF) {
				err = io.ErrUnexpectedEOF
			}
			return nil, fmt.Errorf("opencrs: reading %d bytes at %d of %d: %w", len(buf), st.Need.Offset, size, err)
		}
		ptr, err := in.put(buf)
		if err != nil {
			return nil, err
		}
		res, err := feed.Call(in.ctx, uint64(st.Session), uint64(ptr), uint64(len(buf)))
		in.free(ptr, uint32(len(buf)))
		if err != nil {
			return nil, fmt.Errorf("opencrs: opencrs_probe_feed: %w", err)
		}
		if resp, err = in.take(res[0]); err != nil {
			return nil, err
		}
	}
}

// Check checks a creative.json manifest against one or more screens.json
// documents. probes maps each manifest file name to what Probe found; a file
// with no probe is judged on the manifest's declared values. A zero now
// means the current time.
func Check(specs [][]byte, manifest []byte, probes map[string]*ProbeResult, now time.Time) (*CheckResult, error) {
	if probes == nil {
		probes = map[string]*ProbeResult{}
	}
	if now.IsZero() {
		now = time.Now()
	}
	req := map[string]any{
		"probes": probes,
		"now":    now.Format(time.RFC3339),
	}
	var res CheckResult
	err := runWith(&res, func(in *instance) ([]byte, error) {
		return in.callDocs("opencrs_check", req, append([][]byte{manifest}, specs...)...)
	})
	if err != nil {
		return nil, err
	}
	if res.Reports == nil {
		res.Reports = []*Report{}
	}
	return &res, nil
}

// CheckDir is Check over files in fsys: the spec documents, the manifest,
// and the creative files the manifest names (relative to the manifest's
// directory). A named file that does not exist is judged on the manifest's
// declared values and listed in the result's Missing.
//
// Creative files must stay inside fsys: names with ".." or a leading "/"
// are refused, and symlinks are followed one element at a time and refused
// if they lead out. For a directory on disk, pass root.FS() from
// os.OpenRoot, which also confines the opens themselves; os.DirFS works,
// but between the check and the open a concurrent writer could swap in a
// symlink. Hard links can't be told from ordinary files by any host, so
// don't check a package extracted from an untrusted archive unless the
// archive was unpacked without them.
func CheckDir(fsys fs.FS, specs []string, manifest string, now time.Time) (*CheckResult, error) {
	var docs [][]byte
	for _, name := range specs {
		b, err := fs.ReadFile(fsys, name)
		if err != nil {
			return nil, err
		}
		docs = append(docs, b)
	}
	raw, err := fs.ReadFile(fsys, manifest)
	if err != nil {
		return nil, err
	}
	// Only the file names are needed here; the module validates the rest.
	var m struct {
		Files []struct {
			File string `json:"file"`
		} `json:"files"`
	}
	if err := json.Unmarshal(raw, &m); err != nil {
		return nil, fmt.Errorf("%s: %w", manifest, err)
	}
	probes := map[string]*ProbeResult{}
	var missing []string
	for _, f := range m.Files {
		if _, done := probes[f.File]; done || f.File == "" {
			continue
		}
		if !localName(f.File) {
			return nil, fmt.Errorf("%s: file name %q is absolute or leaves the manifest's directory", manifest, f.File)
		}
		p, err := probeFS(fsys, path.Join(path.Dir(manifest), f.File))
		switch {
		case err == nil:
			probes[f.File] = p
		case errors.Is(err, fs.ErrNotExist):
			missing = append(missing, f.File)
		default:
			return nil, fmt.Errorf("%s: %w", f.File, err)
		}
	}
	res, err := Check(docs, raw, probes, now)
	if err != nil {
		return nil, err
	}
	res.Missing = missing
	return res, nil
}

// localName reports whether a manifest file name stays inside the
// manifest's directory: relative, with no ".." element.
func localName(name string) bool {
	if name == "" || strings.HasPrefix(name, "/") || strings.Contains(name, `\`) {
		return false
	}
	return !slices.Contains(strings.Split(name, "/"), "..")
}

// resolve follows symlinks along name the way the operating system does,
// one element at a time, and returns the name with none left in it. It
// fails if a link is absolute or leads out of fsys. A ".." is taken
// against the directory reached so far, never by cleaning the text, so a
// link to "." followed by ".." can't pass for a name inside. A missing
// element is an fs.ErrNotExist error. If fsys can't report symlinks
// (it isn't an fs.ReadLinkFS), name is returned as it is.
func resolve(fsys fs.FS, name string) (string, error) {
	lfs, ok := fsys.(fs.ReadLinkFS)
	if !ok {
		return name, nil
	}
	todo := strings.Split(name, "/")
	var done []string // real directories (and finally the file), no links
	links := 0
	for len(todo) > 0 {
		part := todo[0]
		todo = todo[1:]
		switch part {
		case "", ".":
			continue
		case "..":
			if len(done) == 0 {
				return "", fmt.Errorf("%s leads outside the directory", name)
			}
			done = done[:len(done)-1]
			continue
		}
		cur := path.Join(append(done, part)...)
		st, err := lfs.Lstat(cur)
		if err != nil {
			return "", err
		}
		if st.Mode()&fs.ModeSymlink == 0 {
			done = append(done, part)
			continue
		}
		if links++; links > 40 {
			return "", fmt.Errorf("too many symlinks in %s", name)
		}
		target, err := lfs.ReadLink(cur)
		if err != nil {
			return "", err
		}
		if strings.HasPrefix(target, "/") || strings.Contains(target, `\`) {
			return "", fmt.Errorf("%s is a symlink to %s, outside the directory", cur, target)
		}
		todo = append(strings.Split(target, "/"), todo...)
	}
	if len(done) == 0 {
		return ".", nil
	}
	return path.Join(done...), nil
}

// probeFS probes one file in fsys. It checks that the file is a regular
// one before opening it, so a FIFO or device can't block the open.
func probeFS(fsys fs.FS, name string) (*ProbeResult, error) {
	real, err := resolve(fsys, name)
	if err != nil {
		return nil, err
	}
	var st fs.FileInfo
	if lfs, ok := fsys.(fs.ReadLinkFS); ok {
		st, err = lfs.Lstat(real)
	} else {
		st, err = fs.Stat(fsys, real)
	}
	if err != nil {
		return nil, err
	}
	if !st.Mode().IsRegular() {
		return nil, fmt.Errorf("%s is not a regular file", name)
	}
	f, err := fsys.Open(real)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	if st, err = f.Stat(); err != nil {
		return nil, err
	}
	if !st.Mode().IsRegular() {
		return nil, fmt.Errorf("%s is not a regular file", name)
	}
	if ra, ok := f.(io.ReaderAt); ok {
		return Probe(ra, st.Size())
	}
	b, err := io.ReadAll(f) // files that can't seek are read whole
	if err != nil {
		return nil, err
	}
	return Probe(bytesReaderAt(b), int64(len(b)))
}

type bytesReaderAt []byte

func (b bytesReaderAt) ReadAt(p []byte, off int64) (int, error) {
	if off >= int64(len(b)) {
		return 0, io.EOF
	}
	n := copy(p, b[off:])
	if n < len(p) {
		return n, io.EOF
	}
	return n, nil
}

// Resolve returns the requirements for a format or screen reference such as
// "harbourline.example/retail/portrait-totem-75". A format gives one result
// per screen using it.
func Resolve(spec []byte, ref string) ([]Requirements, error) {
	var res []Requirements
	err := runWith(&res, func(in *instance) ([]byte, error) {
		return in.callDocs("opencrs_resolve", map[string]any{"ref": ref}, spec)
	})
	if err != nil {
		return nil, err
	}
	return res, nil
}
