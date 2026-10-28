package opencrs

// ScreenWidth is a screen's width in millimetres and pixels, for turning
// physical text sizes into pixels.
type ScreenWidth struct {
	WidthMM float64 `json:"width_mm"`
	WidthPX int     `json:"width_px"`
}

// LegibilityRequest asks for minimum text sizes. DistanceM is [near, far] in
// metres. Nil optional fields take the spec's defaults.
type LegibilityRequest struct {
	DistanceM [2]float64   `json:"distance_m"`
	Ratio     *float64     `json:"ratio,omitempty"`
	Screen    *ScreenWidth `json:"screen,omitempty"`
	DwellS    *float64     `json:"dwell_s,omitempty"`
	Rate      *float64     `json:"rate,omitempty"` // reading rate, words per second
}

// TextSize is a minimum text size and its working.
type TextSize struct {
	MM      float64  `json:"mm"`
	PX      *int64   `json:"px,omitempty"`
	PXExact *float64 `json:"px_exact,omitempty"`
	Basis   string   `json:"basis"`
}

// WordLimit is a word limit and its working.
type WordLimit struct {
	Value int64  `json:"value"`
	Basis string `json:"basis"`
}

// LegibilityResult is what Legibility works out.
type LegibilityResult struct {
	Ratio float64 `json:"ratio"`
	// Key is the key message, read from the far end of the range.
	Key TextSize `json:"key"`
	// Legal is legal text, read from the near end.
	Legal TextSize   `json:"legal"`
	Words *WordLimit `json:"words,omitempty"`
}

// DwellRequest asks what a driver can read. DistanceM is [near, far] in
// metres; RulePacks names built-in packs (see RulePacks) to report limits for.
type DwellRequest struct {
	SpeedKMH  float64      `json:"speed_kmh"`
	DistanceM [2]float64   `json:"distance_m"`
	Ratio     *float64     `json:"ratio,omitempty"`
	Rate      *float64     `json:"rate,omitempty"`
	Screen    *ScreenWidth `json:"screen,omitempty"`
	RulePacks []string     `json:"rule_packs,omitempty"`
}

// GlanceView is what a driver can read, rounded for display.
type GlanceView struct {
	SpeedMS   float64 `json:"speed_m_s"`
	CrossingS float64 `json:"crossing_s"`
	Seconds   float64 `json:"seconds"`
	Words     int64   `json:"words"`
	// ReadFromM is where reading must start to finish by the near end.
	ReadFromM float64 `json:"read_from_m"`
}

// PackLimits is one rule pack's limits for a sign at a given speed.
type PackLimits struct {
	Ref                    string   `json:"ref"`
	Title                  string   `json:"title"`
	Status                 string   `json:"status"`
	StatusNote             string   `json:"status_note,omitempty"`
	SourceTitle            string   `json:"source_title,omitempty"`
	SourceURL              string   `json:"source_url,omitempty"`
	MinDwellS              *float64 `json:"min_dwell_s,omitempty"`
	DwellBasis             string   `json:"dwell_basis,omitempty"`
	DwellUnknown           string   `json:"dwell_unknown,omitempty"`
	MaxTransitionS         *float64 `json:"max_transition_s,omitempty"`
	MotionAllowed          *bool    `json:"motion_allowed,omitempty"`
	NoFlashing             bool     `json:"no_flashing"`
	MaxWordsAtGlance       bool     `json:"max_words_at_glance"`
	NoSequencing           bool     `json:"no_sequencing"`
	NoTrafficDeviceImagery bool     `json:"no_traffic_device_imagery"`
	NoDeviceInteraction    bool     `json:"no_device_interaction"`
	FailureState           string   `json:"failure_state,omitempty"`
	SchoolZone             string   `json:"school_zone,omitempty"`
}

// DwellResult is what Dwell works out.
type DwellResult struct {
	SpeedKMH float64    `json:"speed_kmh"`
	Ratio    float64    `json:"ratio"`
	Rate     float64    `json:"rate"`
	Glance   GlanceView `json:"glance"`
	// Key text must be legible from where reading starts.
	Key   TextSize     `json:"key"`
	Packs []PackLimits `json:"packs"`
}

// RulePack summarises one built-in rule pack.
type RulePack struct {
	Ref       string `json:"ref"`
	Title     string `json:"title"`
	Status    string `json:"status"`
	SourceURL string `json:"source_url,omitempty"`
}

// Page is one page the caller has fetched, with the response headers the
// publisher check looks at. A fetch that failed has Status 0 and Error set.
type Page struct {
	Status          int    `json:"status"`
	Body            string `json:"-"` // sent to the module as raw bytes, not in JSON
	Error           string `json:"error,omitempty"`
	ETag            string `json:"etag,omitempty"`
	CacheControl    string `json:"cache_control,omitempty"`
	LastModified    string `json:"last_modified,omitempty"`
	ContentType     string `json:"content_type,omitempty"`
	ContentEncoding string `json:"content_encoding,omitempty"`
	// AllowOrigin is the Access-Control-Allow-Origin header.
	AllowOrigin string `json:"allow_origin,omitempty"`
}

// PublisherResult is what the publisher check found.
type PublisherResult struct {
	Domain   string `json:"domain"`
	IndexURL string `json:"index_url"`
	// Error says why the index couldn't be used; nothing else is checked then.
	Error     string                 `json:"error,omitempty"`
	IndexLint *LintResult            `json:"index_lint,omitempty"`
	Discovery *Discovery             `json:"discovery,omitempty"`
	Lints     map[string]*LintResult `json:"lints,omitempty"`
	Advice    []Advice               `json:"advice"`
	Errors    int                    `json:"errors"`
	Warnings  int                    `json:"warnings"`
	// Wanted lists the URLs discovery asked for that weren't in the pages
	// passed in, in the order they were found, each once. Fetch them (the
	// publisher chose them: see Publisher and FetchPage), add them to the
	// pages under exactly these strings and call Publisher again; when Wanted
	// is empty the result is final.
	Wanted []string `json:"wanted"`
	// WantedDetail is Wanted with what each URL is for, in the same order.
	WantedDetail []WantedURL `json:"wanted_detail"`
}

// WantedURL is a URL the publisher check wants fetched, and what for.
type WantedURL struct {
	URL string `json:"url"`
	// Kind is "index" for a publisher's or delegate's
	// /.well-known/opencrs.json, and "document" for anything else.
	Kind string `json:"kind"`
	// SameHost means the fetch must not follow a redirect to another host.
	SameHost bool `json:"same_host"`
}

// Advice is one thing for the publisher to know about one file.
type Advice struct {
	Severity string `json:"severity"`
	URL      string `json:"url"`
	Message  string `json:"message"`
}

// Discovery is everything discovered for one publisher.
type Discovery struct {
	Domain    string               `json:"domain"`
	IndexURL  string               `json:"index_url"`
	Index     Index                `json:"index"`
	Documents []DiscoveredDocument `json:"documents"`
	Delegates []DelegateResult     `json:"delegates,omitempty"`
}

// Index is a publisher's index (/.well-known/opencrs.json).
type Index struct {
	OpenCRS    string          `json:"opencrs"`
	Publisher  IndexPublisher  `json:"publisher"`
	Documents  []IndexDocument `json:"documents"`
	Delegates  []Delegate      `json:"delegates,omitempty"`
	ChangesURL string          `json:"changes_url,omitempty"`
}

// IndexPublisher names the publisher an index speaks for.
type IndexPublisher struct {
	ID      string `json:"id"`
	Name    string `json:"name"`
	Contact string `json:"contact,omitempty"`
}

// IndexDocument is one document an index vouches for.
type IndexDocument struct {
	URL        string `json:"url"`
	Network    string `json:"network,omitempty"`
	Visibility string `json:"visibility,omitempty"`
}

// Delegate names another domain that may write documents for the publisher.
type Delegate struct {
	Domain string `json:"domain"`
	Scope  string `json:"scope,omitempty"`
}

// DiscoveredDocument is one document and the core's trust decision on it.
type DiscoveredDocument struct {
	URL          string   `json:"url"`
	Network      string   `json:"network,omitempty"`
	Visibility   string   `json:"visibility,omitempty"` // public or partner
	ListedBy     string   `json:"listed_by"`
	DelegatedBy  string   `json:"delegated_by,omitempty"`
	Publisher    string   `json:"publisher,omitempty"`
	Networks     []string `json:"networks,omitempty"`
	Screens      int      `json:"screens,omitempty"`
	UpdatedAt    string   `json:"updated_at,omitempty"`
	ExpiresAt    string   `json:"expires_at,omitempty"`
	ETag         string   `json:"etag,omitempty"`
	CacheControl string   `json:"cache_control,omitempty"`
	LastModified string   `json:"last_modified,omitempty"`
	// Status is trusted, expired, rejected, skipped or failed. Only
	// "trusted" documents are safe to validate against.
	Status string `json:"status"`
	Reason string `json:"reason,omitempty"`
}

// DelegateResult is what following one delegate found.
type DelegateResult struct {
	Domain   string `json:"domain"`
	Scope    string `json:"scope,omitempty"`
	IndexURL string `json:"index_url,omitempty"`
	Error    string `json:"error,omitempty"`
}
