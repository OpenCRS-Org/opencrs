package opencrs

import "encoding/json"

// LintResult is what Lint finds in one document.
type LintResult struct {
	// Kind is "screens", "creative" or another document kind the schemas know.
	Kind         string        `json:"kind"`
	SchemaErrors []SchemaError `json:"schema_errors,omitempty"`
	// DocumentChecks are a screens document's own problems, found once it
	// matches the schema: broken references and rule conflicts.
	DocumentChecks []Finding `json:"document_checks,omitempty"`
}

// OK reports whether the document has no problems.
func (l *LintResult) OK() bool { return len(l.SchemaErrors) == 0 && len(l.DocumentChecks) == 0 }

// SchemaError is one place a document departs from its schema.
type SchemaError struct {
	Path    string `json:"path"` // JSON pointer into the document
	Message string `json:"message"`
}

func (e SchemaError) String() string {
	p := e.Path
	if p == "" {
		p = "/"
	}
	return p + ": " + e.Message
}

// Finding is one check result in a report (the brief calls it Check, which
// would clash with the Check function).
type Finding struct {
	Code     string   `json:"code"`
	Severity string   `json:"severity"` // error, warning, info or not_checked
	Message  string   `json:"message"`
	RectPX   *[4]int  `json:"rect_px,omitempty"`
	Element  string   `json:"element,omitempty"`
	Screens  []string `json:"screens,omitempty"`
	Source   string   `json:"source,omitempty"`
}

// ValidatorInfo names the validator that made a report.
type ValidatorInfo struct {
	Name    string `json:"name"`
	Version string `json:"version"`
}

// Report says whether one creative file meets one target's requirements.
type Report struct {
	OpenCRS       string        `json:"opencrs"`
	Validator     ValidatorInfo `json:"validator"`
	File          string        `json:"file,omitempty"`
	FileSHA256    string        `json:"file_sha256,omitempty"`
	Target        string        `json:"target"`
	SpecUpdatedAt string        `json:"spec_updated_at,omitempty"`
	CheckedAt     string        `json:"checked_at,omitempty"`
	Result        string        `json:"result"` // pass, warn, fail or incomplete
	Checks        []Finding     `json:"checks"`
}

// ProbeResult is what could be read from a creative file itself.
type ProbeResult struct {
	SHA256    string   `json:"sha256"`
	Bytes     int64    `json:"bytes"`
	MediaType string   `json:"media_type"` // sniffed from the signature; "" if unknown
	Pixels    *[2]int  `json:"pixels,omitempty"`
	DurationS *float64 `json:"duration_s,omitempty"`
}

// CheckResult is the outcome of checking a manifest against some specs.
type CheckResult struct {
	Reports []*Report `json:"reports"`
	Notes   []string  `json:"notes,omitempty"`
	// Requirements maps each report's target to the screens it covers.
	Requirements map[string][]Requirements `json:"requirements"`
	// Missing, set by CheckDir, lists the manifest's files that weren't
	// found, in manifest order. They were judged on the manifest's own
	// declared values (pixels, size, duration) rather than the files.
	Missing []string `json:"missing,omitempty"`
}

// Requirements is one screen's resolved requirements, with derived minimum
// text sizes.
type Requirements struct {
	Screen         string          `json:"screen,omitempty"`
	Format         string          `json:"format"`
	Venue          string          `json:"venue,omitempty"`
	Pixels         [2]int          `json:"pixels"`
	PhysicalMM     *[2]float64     `json:"physical_mm,omitempty"`
	Media          Media           `json:"media"`
	SafeArea       *Insets         `json:"safe_area,omitempty"`
	BlockedRegions []Region        `json:"blocked_regions,omitempty"`
	Segments       []Segment       `json:"segments,omitempty"`
	Viewing        *Viewing        `json:"viewing,omitempty"`
	Rules          json.RawMessage `json:"rules"` // the resolved rule set, as JSON
	Derived        map[string]any  `json:"derived,omitempty"`
}

// Media is what a screen accepts.
type Media struct {
	Static      []string `json:"static,omitempty"`
	Motion      []string `json:"motion,omitempty"`
	SlotS       *float64 `json:"slot_s,omitempty"`
	FPS         *float64 `json:"fps,omitempty"`
	Codecs      []string `json:"codecs,omitempty"`
	MaxBytes    *int64   `json:"max_bytes,omitempty"`
	ColourSpace string   `json:"colour_space,omitempty"`
	Audio       *bool    `json:"audio,omitempty"`
}

// Insets are margins in pixels.
type Insets struct {
	Top    int `json:"top"`
	Right  int `json:"right"`
	Bottom int `json:"bottom"`
	Left   int `json:"left"`
}

// Region is an area the screen covers or overlays; RectPX is [x, y, w, h].
type Region struct {
	Name   string `json:"name"`
	RectPX [4]int `json:"rect_px"`
	Reason string `json:"reason,omitempty"`
}

// Segment is one panel of a composite surface.
type Segment struct {
	ID       string `json:"id"`
	RectPX   [4]int `json:"rect_px"`
	Primary  bool   `json:"primary,omitempty"`
	Crossing string `json:"crossing,omitempty"`
}

// Viewing describes how the screen is seen.
type Viewing struct {
	DistanceM *[2]float64 `json:"distance_m,omitempty"`
	DwellS    *float64    `json:"dwell_s,omitempty"`
	Approach  string      `json:"approach,omitempty"`
	Light     string      `json:"light,omitempty"`
}
