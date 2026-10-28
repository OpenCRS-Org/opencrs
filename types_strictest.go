package opencrs

// StrictestRequirements is one set of requirements for everything a
// reference covers: a file that meets it passes on every one of those
// screens. See "Strictest requirements" in the spec for how each limit is
// combined.
type StrictestRequirements struct {
	// Ref is the reference that was resolved.
	Ref string `json:"ref"`
	// Screens is how many screens the reference covers (0 for a format no
	// screen uses).
	Screens    int         `json:"screens"`
	Format     string      `json:"format"`
	Pixels     [2]int      `json:"pixels"`
	PhysicalMM *[2]float64 `json:"physical_mm,omitempty"`
	// Media lists only the file types and codecs every screen accepts. A
	// list that is empty because no single file can satisfy every screen is
	// explained in Notes.
	Media          Media     `json:"media"`
	SafeArea       *Insets   `json:"safe_area,omitempty"`
	BlockedRegions []Region  `json:"blocked_regions,omitempty"`
	Segments       []Segment `json:"segments,omitempty"`
	// Viewing holds the nearest and farthest viewing distances, and the
	// shortest dwell, of the screens.
	Viewing *Viewing `json:"viewing,omitempty"`
	// MinCapHeightPX is the minimum text size for each role in pixels, the
	// largest of the screens' (each worked out at its own viewing distance).
	MinCapHeightPX map[string]int `json:"min_cap_height_px,omitempty"`
	Rules          StrictestRules `json:"rules,omitzero"`
	// ConditionalElementsLeftOut lists the required elements that apply only
	// when the creative is tagged for them, which are not in the rules.
	ConditionalElementsLeftOut []ConditionalElement `json:"conditional_elements_left_out,omitempty"`
	// Notes say where no single file can pass on every screen, and where a
	// size could not be worked out.
	Notes []string `json:"notes,omitempty"`
	// Sources maps a combined value, by its path in the result such as
	// "safe_area.top" or "min_cap_height_px.headline", to the id of the
	// first screen that set it.
	Sources map[string]string `json:"sources"`
}

// StrictestRules are the combined rules of a StrictestRequirements.
type StrictestRules struct {
	// RequiredElements are the elements every screen requires without a
	// condition, by role, with the overlap of their position limits.
	RequiredElements     []RequiredElement `json:"required_elements,omitempty"`
	MaxWords             *int              `json:"max_words,omitempty"`
	RestrictedCategories []string          `json:"restricted_categories,omitempty"`
	Motion               *MotionRules      `json:"motion,omitempty"`
	Brightness           *Brightness       `json:"brightness,omitempty"`
	Playback             *Playback         `json:"playback,omitempty"`
	RulePacks            []string          `json:"rule_packs,omitempty"`
	Notes                []string          `json:"notes,omitempty"`
	Conflicts            []string          `json:"conflicts,omitempty"`
}

// RequiredElement is something a creative must include.
type RequiredElement struct {
	Role         string  `json:"role"`
	WithinRectPX *[4]int `json:"within_rect_px,omitempty"`
	Notes        string  `json:"notes,omitempty"`
}

// ConditionalElement is a required element that applies only when the
// creative's tags say so.
type ConditionalElement struct {
	Role string `json:"role"`
	When string `json:"when"`
	// MinCapHeightPX and WithinRectPX are what the element asks of creative
	// with the tag: the largest size in pixels, and the overlap of the
	// position limits, over the screens.
	MinCapHeightPX *int    `json:"min_cap_height_px,omitempty"`
	WithinRectPX   *[4]int `json:"within_rect_px,omitempty"`
	Notes          string  `json:"notes,omitempty"`
}

// MotionRules limit animation.
type MotionRules struct {
	Allowed       *bool    `json:"allowed,omitempty"`
	NoFlashing    *bool    `json:"no_flashing,omitempty"`
	MaxFlashHz    *float64 `json:"max_flash_hz,omitempty"`
	MinFrameHoldS *float64 `json:"min_frame_hold_s,omitempty"`
}

// Brightness limits luminance.
type Brightness struct {
	MaxNightCdM2 *float64 `json:"max_night_cd_m2,omitempty"`
	MaxDayCdM2   *float64 `json:"max_day_cd_m2,omitempty"`
}

// Playback limits how the platform shows creative.
type Playback struct {
	MinDwellS      *float64 `json:"min_dwell_s,omitempty"`
	MaxTransitionS *float64 `json:"max_transition_s,omitempty"`
}
