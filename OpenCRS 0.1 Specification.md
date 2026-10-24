# OpenCRS 0.1 Specification

Oct 22, 2026 · @Nick

OpenCRS, the Open Creative Requirements Specification, is an open format for describing what creative needs to work on a digital screen. Working draft.

## Overview

**OpenCRS lets anyone who runs screens publish, in one machine-readable file on their own website, everything a creative needs to work on each screen. Anyone who makes creative can then check and build to it automatically.**

It defines three documents:

| Document | Who writes it | What it says |
| --- | --- | --- |
| `screens.json` (requirements) | Media owners, landlords, venues | The formats, screens and rules that creative must meet |
| `creative.json` (manifest) | Agencies, brands, creative tools | Which files are meant for which formats, and what each file contains |
| Validation report | Validators, such as the reference validator or a creative tool's built-in checks | Whether a creative meets a format's requirements, with coded errors and warnings |

**Design principles**

1. **The publisher's domain is the source of truth.** A file on harbourline.example is Harbourline's. No central authority is needed to trust it.
2. **Describe the screen as people see it,** not just its pixels: physical size, viewing distance, dwell time and blocked areas, as well as resolution.
3. **Formats are shared, screens are specific.** Forty identical totems point to one format, and each screen adds only what's different about it.
4. **Specs inherit; restrictions only tighten.** Network → venue → format → screen, so a legal line is set once, not 400 times. Specs such as pixels and media follow the most specific level, but no level can loosen a restriction set above it, so a landlord can trust that its rules reach every screen.
5. **Machine-checkable where possible, human-readable always.** Every rule has a code a validator can test and a sentence a designer can read.
6. **Small core, open extensions.** Version 0.1 covers what every screen needs; anything else goes in `ext`, as in OpenRTB.

**Not in scope for 0.1:** pricing and availability (booking systems and OpenRTB do that), audience measurement, venue classification (OpenOOH already covers it, and OpenCRS refers to it), and how creative is delivered to a player (each platform's job).

## How it fits together

**Publishers describe their screens once, on their own domain. Every tool that makes or checks creative reads the same description, so creative arrives right first time.**

&#91;embedded content: OpenCRS architecture · 6 parts\]

The top row is the publisher's side, the bottom row the creative side.

## Publishing and discovery

**Each publisher hosts one small index file at a fixed address on its own domain. The index points to one or more `screens.json` documents, which can live anywhere, including with a company that hosts them for the publisher.** The index on the publisher's domain is what makes them trustworthy.

**1. The index (required):** `https://<publisher-domain>/.well-known/opencrs.json`

```json
{
  "opencrs": "0.1",
  "publisher": { "id": "harbourline.example", "name": "Harbourline", "contact": "specs@harbourline.example" },
  "documents": [
    { "url": "https://harbourline.example/opencrs/retail.screens.json", "network": "retail", "visibility": "public" },
    { "url": "https://specs.cms-a.example/harbourline/airports.screens.json", "network": "airports", "visibility": "public" },
    { "url": "https://harbourline.example/opencrs/retail-locations.screens.json", "network": "retail", "visibility": "partner" }
  ],
  "delegates": [ { "domain": "baycrest.example", "scope": "network:retail" } ]
}
```

**2. Rules for documents**

- **Authority comes from the index.** A `screens.json` anywhere on the web is trusted only if an index on the publisher's own domain lists its exact URL. That lets a small landlord have its media owner or another service host the file without giving away control. An index vouches only for its own publisher: a listed document that declares a different `publisher` is rejected. "Exact" means exact, except that scheme and host are case-insensitive; a trailing slash or a different query string is a different URL.
- **One document per network** keeps files small and updates independent. Limits: UTF-8 JSON, served compressed, at most 50 MB per document.
- **Delegation:** when a media owner sells a landlord's screens, the landlord's index can name that media owner under `delegates`. Validators then accept the media owner's documents for those screens. This mirrors how ads.txt and sellers.json let one company authorise another. Both sides vouch:
  - The publisher that owns the screens names the delegate and a `scope` in its index: `network:<id>` for one of its networks, or no scope for all of them. A scope a validator doesn't recognise grants nothing.
  - The delegate writes documents that declare the **owning** publisher as `publisher`, so creative targets them under the owner's domain, and lists their exact URLs in the delegate's own index.
  - A validator trusts such a document only if every network in it falls inside the scope. Delegation goes one hop: a delegate's own delegates aren't followed.
  - *Provisional:* these delegation rules, and the partner-merging rules under Privacy, are a working design until they're tested against a real landlord and media-owner pair.
- **Partner documents** are listed in the index but served only to authenticated partners (see Privacy).
- **HTTPS only.** Document URLs and `changes_url` use `https://`, since trust rests on the domain. Validators may offer a local-testing option that accepts `http://localhost`, but a published index must not rely on it. Fetchers never follow a redirect from HTTPS to HTTP, and don't follow an index request, or any request carrying partner credentials, to another host.
- **Credentials stay with their issuer.** A token issued by a publisher is sent only to the partner documents listed in that publisher's own index, never to a delegate.

**3. Keeping copies fresh**

- Publishers set `updated_at` in each document and serve standard HTTP `ETag` and `Cache-Control` headers.
- Consumers re-check at least daily and must honour `Cache-Control`. A document past its `expires_at` must not be used for validation.
- An optional `changes_url` in the index points to a feed of recent changes, so registries don't need to poll large files.

**4. Schema:** each document declares `"opencrs": "0.1"` and validates against the published JSON Schema at `opencrs.dev/schema/0.1/`.

## Object model

**A document holds networks. Each network holds venues, shared formats and physical screens, and each screen points to one format. Rules can sit at any level and flow down.**

&#91;embedded content: OpenCRS object model · 6 objects, 4 rule levels\]

A screen also points to its venue, which is how centre-level rules (a landlord's legal line, say) reach it. Field details follow in the reference below.

## Field reference

**Five main objects: Document, Network, RuleSet, Format and Screen. Fields marked ✱ are required in 0.1; everything else is optional.** Every object can carry `ext` for extensions. Readers ignore fields they don't know, and the schemas never forbid them, so a 0.1 reader can read any 0.x document.

**Document** (the root of a `screens.json`)

| Field | Type | Meaning |
| --- | --- | --- |
| `opencrs` ✱ | string | Spec version, `"0.1"` |
| `publisher` ✱ | string | The publisher's domain, matching the index |
| `updated_at` ✱ | datetime | When this document last changed |
| `expires_at` | datetime | Don't validate against it after this time |
| `networks` ✱ | Network\[\] | The networks in this document |

**Network** (a set of screens sold or managed together)

| Field | Type | Meaning |
| --- | --- | --- |
| `id` ✱, `name` ✱ | string | Stable identifier and display name |
| `rules` | RuleSet | Rules for every screen in the network |
| `venues` | Venue\[\] | Centres, airports or stations, each with `id`, `name`, an OpenOOH `venue_type` (the numeric ID from OpenOOH 1.2.1, such as `205` for a mall), optional `rules` and `address` |
| `formats` ✱ | Format\[\] | Shared format definitions |
| `screens` ✱ | Screen\[\] | Physical screens |
| `delivery` | Delivery | How creative reaches this network |

**Format** (shared by every screen of that kind)

| Field | Type | Meaning |
| --- | --- | --- |
| `id` ✱ | string | Stable key, such as `portrait-totem-75` |
| `pixels` ✱ | \[w, h\] | Canvas size in pixels |
| `physical_mm` | \[w, h\] | Visible display size; needed for legibility checks |
| `media` ✱ | Media | Accepted static and motion types, slot length, frame rate, codecs, maximum file size, colour space, audio |
| `safe_area` | Insets | Margins in pixels where important content must not go |
| `blocked_regions` | Region\[\] | Areas the screen covers or overlays: bezels, network logo, clock, a physical obstruction |
| `segments` | Segment\[\] | For composite surfaces (wraps, cubes, multi-panel walls): each panel's rectangle within the canvas and how content crosses between panels |
| `rules` | RuleSet | Rules for this format |

**Screen** (one physical screen)

| Field | Type | Meaning |
| --- | --- | --- |
| `id` ✱ | string | Stable identifier within the network |
| `format` ✱ | string | The Format it uses |
| `venue` | string | Venue id |
| `placement` | Placement | `environment` (`indoor` or `outdoor`), level, zone, mounted height in mm, `facing` (a compass point or degrees), and `motion_allowed` (false near roads, for example). For rule packs: `speed_kmh`, `faces_road`, `school_zone` and `luminance_zone` |
| `viewing` | Viewing | Distance range in metres, typical dwell in seconds, approach (head-on or oblique), and `light`, the ambient light (`indoor`, `daylight`, `night`) |
| `overrides` | object | Format fields that differ for this screen, such as an extra blocked region (see Inheritance) |
| `rules` | RuleSet | Screen-specific rules |
| `platform_ids` | object | Keys per playback platform, such as `{"cms-a": "frame-123", "cms-b": "display-unit-456"}`, so creative can be published to the right place |
| `location` | Location | Latitude and longitude. Usually partner-only |

**RuleSet** (can sit on a network, venue, format or screen)

| Field | Type | Meaning |
| --- | --- | --- |
| `legibility` | Legibility | Minimum text size as millimetres on the screen, or as a legibility index (see Legibility) |
| `required_elements` | Element\[\] | Things every creative must include, such as a legal line, with size and position limits |
| `max_words` | integer | Word limit for the main message |
| `reading_rate_wps` | number | Words per second for deriving `max_words` from dwell; default 2 |
| `restricted_categories` | string\[\] | Advertising categories not allowed, using IAB content category codes |
| `motion` | Motion | Limits such as no strobe or flashing above a set rate, minimum hold on a frame |
| `brightness` | object | Maximum average brightness for night-time playback, where required |
| `playback` | object | Playback limits the screen platform enforces, such as a site consent's `min_dwell_s` and `max_transition_s` (see Regulatory rule packs) |
| `rule_packs` | string\[\] | Regulatory rule packs that apply, as `name@version` (see Regulatory rule packs) |
| `notes` | string | Human-readable guidance that isn't machine-checkable |
| `remove` | object | Inherited required elements that don't apply here, by role: `{"required_elements": ["legal"]}` |

**Delivery**

| Field | Type | Meaning |
| --- | --- | --- |
| `lead_time_hours` | integer | How long before go-live creative must arrive |
| `submit` | object | Where and how to submit: a portal URL, an API, or an email address |
| `file_naming` | string | Naming pattern, if required |

## Units, coordinates and conventions

**One unit per kind of measure, named in the field, so nothing is ever guessed.**

| Measure | Convention |
| --- | --- |
| Pixels | Integers. Origin top-left of the canvas, x right, y down. Fields end in `_px` or use the `pixels` name |
| Rectangles | `[x, y, width, height]` in pixels, relative to the canvas, or to the segment when inside a segment |
| Insets | `{ "top", "right", "bottom", "left" }` in pixels |
| Physical sizes | Millimetres, fields ending `_mm` |
| Distances | Metres, fields ending `_m`, as `[min, max]` |
| Durations | Seconds, fields ending `_s`, decimals allowed |
| File sizes | Bytes, fields ending `_bytes` |
| File types | IANA media types, such as `image/jpeg` or `video/mp4` |
| Dates | ISO 8601 with a time zone offset, or `Z` for UTC |
| Colour | Colour space named explicitly, such as `srgb` or `rec709` |
| Text | Language tags such as `en-AU` where text appears in rules or notes |

**Identifiers:** `id` values are unique within their parent and stable over time. They may use letters, digits, `.`, `_` and `-`, so existing CMS asset IDs such as `HC-L1-001` can be used as they are. IDs are case-sensitive and matched exactly: `hc-l1-001` and `HC-L1-001` are different screens. A globally unique reference joins them: `harbourline.example/retail/portrait-totem-75` for a format, or `harbourline.example/retail/screen/HC-L2-014` for a screen.

**Inheritance: specs follow the most specific level; restrictions only tighten.** The chain is network → venue → format → screen.

- **Specs** (pixels, physical size, media, safe area, segments) come from the format, and a screen's `overrides` replace them. `blocked_regions` in `overrides` add to the format's list: a region with the same `name` replaces it, and `overrides.remove.blocked_regions` drops one by name.
- **Restrictions** in a RuleSet can only get stricter going down, whichever level sets them. That's what lets a landlord trust that a network-wide format can't quietly drop a centre's alcohol restriction or a state's rule pack:
  - `restricted_categories` and `rule_packs` accumulate and can't be removed.
  - Numeric limits take the stricter value: the larger minimum text size, the smaller legibility `ratio`, the smaller `max_words` and `reading_rate_wps`, the smaller `max_flash_hz` and brightness limits, the longer `min_frame_hold_s` and `playback.min_dwell_s`, and the shorter `playback.max_transition_s`. Once motion is disallowed, a lower level can't allow it again.
  - A required element restated lower down (same `role` and `when`) merges with the inherited one, keeping the larger minimum size and the overlap of any position limits. If the position limits don't overlap, no creative can ever comply. That's an error in the document itself, `CRS-RULE-CONFLICT`, which validators report when a publisher checks its own `screens.json`, and repeat in every affected creative report.
  - `remove` is only for items that aren't regulatory: inherited required elements, and blocked regions in `overrides`.
- A screen's `overrides.rules` is merged as format-level rules, so it can only tighten too.

## Legibility

**This is what no existing spec has: minimum text size worked out from how far away people are and how long they look, not just a pixel count.** It's why `physical_mm` and `viewing` matter.

A publisher sets legibility in one of two ways:

1. **Directly:** `"legibility": { "min_cap_height_mm": 40, "min_legal_cap_height_mm": 14 }`
2. **As a ratio:** `"legibility": { "ratio": 150 }`, meaning text should be readable at 150 times its letter height. The validator then works out the size from the screen's viewing distance.

If both apply to a screen, after inheritance, the larger size wins. If the ratio can't be worked out (the screen has no viewing distance), the true minimum is unknown and the check is `not_checked`, unless the text is already below the direct minimum. That fails whatever the ratio says, so it's an error.

**How the ratio becomes pixels**

```latex
h_{mm} = \frac{d_{m} \times 1000}{\text{ratio}} \qquad h_{px} = h_{mm} \times \frac{\text{pixels}_{w}}{\text{physical}_{w,mm}}
```

- The key message (`headline`, `subhead`, `body`, `cta`, `offer`, `price`, `text`) uses the **far** end of the viewing range. Legal and fine print (`legal`, `disclaimer`, `fine_print`, `terms`) use the **near** end, since it only needs reading up close.
- **Minimum sizes round up** to the next whole pixel, so text below the minimum can never pass by rounding.
- *Worked example (illustrative):* a 1080 × 1920 totem 940 mm wide (1.15 px per mm), viewed from 2–6 m, ratio 150. The key message needs a cap height of at least 6000 ÷ 150 = 40 mm, which is 45.96 px, so 46 px. Legal text needs 2000 ÷ 150 = 13.3 mm, which is 15.3 px, so 16 px.

**Reading time:** `"max_words"` can be set directly, or derived from `viewing.dwell_s` at an assumed reading rate (default 2 words per second). A 4-second glance then allows about 8 words for the main message.

**Defaults to validate before 1.0:** the ratio of 150 and the reading rate are working assumptions based on sign-industry rules of thumb, not tested values. Version 0.1 should collect publishers' own numbers and calibrate against real campaigns, using data from real campaigns, such as which creative was corrected or rejected and why.

## Regulatory rule packs

**Roadside and transport screens follow published rules that vary by state, speed zone and site. OpenCRS encodes each set once as a shared, versioned rule pack that screens reference, instead of every publisher retyping it.**

**What the rules look like (examples)**

- **NSW** ([Transport Corridor Outdoor Advertising and Signage Guidelines](https://www.transport.nsw.gov.au/system/files/media/documents/2023/transport-corridor-outdoor-advertising-and-signage-guidelines.pdf), November 2017; page numbers as printed):
  - fully static ads, with no animated or video content facing drivers (pp. 32, 35, 41)
  - minimum dwell of 10 s below 80 km/h and 25 s at 80 km/h and over (p. 32)
  - transitions of at most 0.1 s, with a black screen if the image fails (p. 32)
  - nothing that could be mistaken for a traffic control device, or read as a driving instruction (p. 32)
  - no more text than a driver can read at a short glance (p. 33)
  - no message sequencing, and no interaction with phones or vehicles (pp. 32, 45)
  - a fixed display during school-zone hours for signs within 250 m of a classified road and visible from a school zone (p. 33)
  - luminance by lighting zone (Table 6, p. 44): no limit in full sun; by day, no limit in zone 1 and 6000 cd/m² in zones 2 to 4; 700 cd/m² at twilight (500 in zone 4); 350 cd/m² at night (200 in zone 4)
- **Queensland** ([Roadside Advertising Manual](https://www.tmr.qld.gov.au/-/media/busind/techstdpubs/Traffic-management/Roadside-advertising-manual/Roadside-Advertising-Manual---Technical-Volume.pdf?la=en), Edition 4, Technical Volume, December 2022), for electronic billboards visible from state-controlled roads:
  - static displays only, with no flashing and no sequential messages (s. 3.6.1.3)
  - minimum dwell of 10 s below 80 km/h and 15 s at 80 km/h and above, or 25 s if visible from a motorway (Table 3.6.1.4)
  - transitions in less than 0.5 s, and a blank screen on failure
  - luminance by the ambient light measured at the sign, from 150 cd/m² at 0–10 lux to 6000 cd/m² in full daylight (Table 3.6.1.5)
- **Western Australia** ([Policy and Assessment Guidelines for Digital Advertising Signs](https://www.mainroads.wa.gov.au/4a612c/globalassets/technical-commercial/technical-library/road-and-traffic-engineering/traffic-management/advertising-and-election-signs/policy-and-assessment-guidelines-for-digital-advertising-signs.pdf), Main Roads WA, May 2025):
  - no animation, flashing, road-sign imagery, driving instructions or sequential messages (ss. 8.3.2, 8.4)
  - dwell by crash risk level, the sign's offset from the road and speed: from 10 s up to 80 s (Table 6)
  - transitions within 0.1 s
  - luminance by AS/NZS 4282 environmental zone: 6000 cd/m² by day, 600 at dawn and dusk, and 350 to zero at night (Table 7)
- **South Australia** ([Advertising Signs Assessment Guidelines for Road Safety](https://dit.sa.gov.au/__data/assets/pdf_file/0007/172645/DIT-Advertising-Signs-Assessment-Guidelines.pdf), August 2014): no video or animation, a self-contained message understood at a glance, a 45 s minimum dwell, transitions within 0.1 s, and at most 200 cd/m² at night (Appendices 4 and 5).
- **Victoria** ([Requirements and Guidelines for Illuminated Outdoor Advertising Signage](https://www.vic.gov.au/sites/default/files/2023-10/DTP-Requirements-and-Guidelines-for-Illuminated-Outdoor-Advertising-Signage.pdf), Department of Transport and Planning, October 2023): luminance by sign category and, at night, district brightness. Planning clause 52.05 sets no dwell time; each permit does.
- **Tasmania** ([State Planning Provisions](https://www.planning.tas.gov.au/__data/assets/pdf_file/0005/784184/Tasmanian-Planning-Scheme-State-Planning-Provisions-effective-26-June-2024.PDF), Signs Code C1.6.2, June 2024): an illuminated sign visible from adjacent roads must not create the effect of flashing, animation or movement. Dwell and luminance are decided permit by permit.
- **Northern Territory and ACT:** no territory-wide rules for digital signs. The NT's road-reserve guidelines prohibit electronic signs in the road reserves it manages, and the NT Planning Scheme 2020 leaves signage to the local council. The ACT's Territory Plan 2023 judges each sign against a general outcome. Its benchmarks aren't mandatory: they allow third-party signs only at ground floor in commercial zones, and no illuminated or third-party advertising in other zones. None of them sets motion, dwell or luminance.

**How a rule pack works**

```json
{
  "rule_pack": "au-nsw-transport-corridor",
  "version": "2017-11",
  "source_url": "https://www.transport.nsw.gov.au/system/files/media/documents/2023/transport-corridor-outdoor-advertising-and-signage-guidelines.pdf",
  "status": "draft",
  "citations": { "playback.max_luminance_cd_m2": "p. 44, Table 6" },
  "creative": {
    "motion": { "allowed": false, "no_flashing": true },
    "no_traffic_device_imagery": true,
    "max_words_at_glance": true,
    "no_sequencing": true,
    "no_device_interaction": true
  },
  "playback": {
    "min_dwell_s": [ { "speed_kmh_below": 80, "value": 10 }, { "speed_kmh_from": 80, "value": 25 } ],
    "max_transition_s": 0.1,
    "failure_state": "black",
    "school_zone": "fixed_display_during_hours",
    "max_luminance_cd_m2": {
      "day": { "zone_1": null, "zone_2": 6000, "zone_3": 6000, "zone_4": 6000 },
      "twilight": { "zone_1": 700, "zone_2": 700, "zone_3": 700, "zone_4": 500 },
      "night": { "zone_1": 350, "zone_2": 350, "zone_3": 350, "zone_4": 200 }
    }
  }
}
```

- **Screens reference packs** with `"rule_packs": ["au-nsw-transport-corridor@2017-11"]` and add their own context on `placement`: `speed_kmh`, `faces_road`, `school_zone` and `luminance_zone`.
- **Packs accumulate down the chain and can't be removed** by a lower level, like all restrictions.
- **A referenced pack applies in full.** Context such as `faces_road: false` doesn't switch a pack's rules off: a publisher that doesn't want a pack's rules on a screen doesn't reference the pack there.
- **Site consent conditions** are written on the screen as normal rules: `playback` (`min_dwell_s`, `max_transition_s`) and `brightness`. A consent can be stricter than the pack (a longer dwell, say), and the stricter value wins. A consent can't loosen a pack.
- **Two kinds of rule:**
  - *Creative* rules are checked when creative is validated: static only, no flashing, nothing resembling a traffic device, readable at a glance. A rule against message sequencing is reported for information only, because sequencing depends on scheduling, which a single file doesn't show.
  - *Playback* rules are for the screen platform to enforce: dwell, transition, failure state, luminance, school-zone switching, and no interaction with phones or vehicles. Validators report them as `info` so everyone sees them, but don't fail creative on them.

**Pack fields.** `min_dwell_s` is a number, or speed bands chosen by `placement.speed_kmh`: a band covers `speed_kmh_from` (inclusive) to `speed_kmh_below` (exclusive), the first match wins, and a band can carry a `note`. `max_luminance_cd_m2` is keyed by lighting period (`day`, `twilight`, `night`), then by `placement.luminance_zone`, with `default` for any zone; `null` means no limit. Zone names are the source's own, and each pack's `notes` say which to use (`zone_1` to `zone_4` in NSW, `A0` to `A4` in WA, `cat_a_high` to `cat_b_low` in Victoria). `failure_state` is `black`, or `blank` where the source requires a blank screen without saying what colour. `citations` say where in the source each rule comes from, `notes` say what the pack leaves out, and `status_note` says what was checked, when.

**Names, versions and status**

- **Name:** `<country>-<region>-<source>`, such as `au-nsw-transport-corridor`. A pack encoded from a summary names the summary, so it never shares a name with a pack from the primary source: Queensland's rules as the OMA summarises them would be `au-qld-oma-digital-signage`, while the pack from Queensland's own manual is `au-qld-tmr-roadside-advertising`.
- **Version:** the source document's edition, as `YYYY-MM`. A new edition of the source is a new version, and screens move to it deliberately.
- **Status:**
  - `unverified`: taken from a summary or extract, not checked against the primary source
  - `draft`: every value checked against the primary source by the maintainer, with page citations
  - `reviewed`: also reviewed independently, by the regulator or an industry body, say
  
  A pack without a `status` is `unverified`. Validators report a pack short of `draft` as `info`, so a reader knows how far to trust it.

**Packs published with 0.1**

| Pack | Encodes | Status |
| --- | --- | --- |
| `au-nsw-transport-corridor@2017-11` | Everything in the NSW list above | `draft` |
| `au-qld-tmr-roadside-advertising@2022-12` | Static only, no flashing, traffic devices, sequencing; dwell by speed, transition, failure state | `draft` |
| `au-wa-mrwa-digital-signs@2025-05` | Static only, no flashing, traffic devices, sequencing; dwell floor by speed, transition, luminance by zone | `draft` |
| `au-sa-dit-advertising-signs@2014-08` | Static only, no flashing, traffic devices, glance text, sequencing; dwell, transition, failure state, night luminance | `draft` |
| `au-vic-dtp-illuminated-signage@2023-10` | Luminance by sign category and district brightness | `draft` |
| `au-tas-spp-signs-code@2024-06` | Static only, no flashing | `unverified`: checked against the June 2024 edition, but three later amendments couldn't be read |

Every value is checked against the primary source, with page or section citations, and no pack is yet `reviewed`. Each pack's `notes` record what it leaves out. Where the source sets something only per site, it is left to the screen's own rules:
- Queensland's luminance follows the ambient light measured at the sign, which no lighting period or zone can select.
- In Western Australia, Main Roads' approval sets each site's dwell from crash risk and offset. The pack gives the lowest value for each speed band, and a band note gives the highest.
- In Victoria and Tasmania, the permit sets the dwell.
- A motorway screen in Queensland needs 25 s, which `placement` can't express, so it is given as a band note.

The NT and the ACT have no pack, because neither has a territory-wide rule for digital signs to encode. Earlier drafts had Queensland, Victorian, WA and SA packs taken from the OMA's summary ([OMA Digital Signage Policy](https://oma.org.au/hubfs/digital_signage_policy_final_version_28022024_0.pdf), February 2024). The primary sources replace them, and differ from the summary:
- WA's dwell and night luminance differ from the summary.
- Victoria has no statewide 30 s dwell.
- Queensland's 25 s rule applies to motorways, not to a speed.
- Queensland, WA and SA all prohibit motion facing drivers.

**How a validator evaluates packs**

- A pack it doesn't know is reported `not_checked` under `CRS-RULE-PACK`. The others are still evaluated.
- `CRS-MOTION-PROHIBITED` is an error for a motion file on a screen whose pack disallows motion. It replaces `CRS-MOTION` for that file, and a screen's own `motion_allowed` can't override it. A pack's `no_flashing` sets a 0 Hz flash limit, checked as `CRS-MOTION`.
- `CRS-TRAFFIC-DEVICE` is always `not_checked`, because only a person can tell whether an image looks like a traffic sign. Reports for screens under such a pack are therefore `incomplete` at best until someone has looked.
- `CRS-SEQUENCING` is `info`. Sequencing is a property of how files are scheduled on a sign and across signs, which a validator checking one file can't see.
- `CRS-GLANCE-TEXT` checks words and text size against what a driver can read in one glance (below). Without the screen's `speed_kmh` and viewing distance it is `not_checked`.
- `CRS-PLAYBACK` is one `info` line per playback limit, with the value chosen for the screen's context and the strictest of the packs and the screen's own rules winning: "Dwell at least 40 s (screen rules); au-nsw-transport-corridor@2017-11 sets 10 s at 60 km/h". Missing context is stated, never guessed: "au-nsw-transport-corridor@2017-11 sets dwell by speed, and the screen has no placement.speed_kmh". A rule that doesn't apply (`school_zone: false`) isn't reported.

**Reading at a glance (CRS-GLANCE-TEXT).** A driver should look away from the road for at most one glance, taken as 2 s. The driver reads for that long, or less if the sign's viewing range is crossed sooner, at the reading rate (`reading_rate_wps`, default 2). To finish reading by the near end of the range, the driver must start at a distance from which the text is legible at the legibility `ratio` (default 150):

```latex
v = \frac{\text{speed}_{km/h}}{3.6} \qquad t = \min\left(2,\ \frac{d_{far} - d_{near}}{v}\right) \qquad \text{words} \le \lfloor t \times \text{rate} \rfloor \qquad h_{mm} = \frac{(d_{near} + v\,t) \times 1000}{\text{ratio}}
```

- Words count every key-message element (`headline`, `subhead`, `body`, `cta`, `offer`, `price`, `text`). The size applies to each of them, rounds up to whole pixels, and warns with little margin, as in Legibility.
- *Worked example:* at 80 km/h (22.22 m/s), seen from 20–150 m, the range takes 5.85 s to cross, so the driver gets the full 2 s: at most 4 words, read from 20 + 44.44 = 64.44 m, so a cap height of at least 429.6 mm at ratio 150. At 100 km/h, seen from 10–50 m, the range takes only 1.44 s: at most 2 words, read from 50 m.
- **A working assumption to validate before 1.0,** like the ratio and reading rate: the 2 s glance comes from driver-distraction research rules of thumb, not from any of the regulations, which say only "no more than a driver can read at a short glance".

**Who maintains packs:** published openly in the OpenCRS repository (`rulepacks/`), each linked to its source document and version, and built into the reference validator. Ideally also reviewed by the regulator or an industry body that tracks the rules across states. The publisher remains responsible for its own consents: a rule pack encodes published rules and doesn't replace legal or planning advice.

**Resolved:** the NSW luminance values, including the daytime zone 4 figure that was unclear in an earlier extraction, were checked against Table 6 of the guidelines (p. 44): 6000 cd/m² by day in zone 4. The OMA's policy (Table 2, p. 2) gives the same figures.

## Creative manifest and validation reports

**The manifest says what a creative is meant for. The report says whether it meets the requirements. Together with `screens.json`, they let any tool check any creative against any screen.**

**The manifest: `creative.json`** (written by the agency or the creative tool, delivered with the files)

```json
{
  "opencrs": "0.1",
  "campaign": { "id": "larkspur-lunch-nov", "advertiser": "Larkspur Cafe", "category": "IAB8" },
  "files": [
    {
      "file": "lunch-portrait.jpg",
      "sha256": "9f2c…",
      "media_type": "image/jpeg",
      "pixels": [1080, 1920],
      "targets": ["harbourline.example/retail/portrait-totem-75"],
      "variant": { "dimension": "weather", "value": "rain" },
      "elements": [
        { "role": "headline", "text": "Lunch specials.", "rect_px": [90, 210, 700, 260], "cap_height_px": 92 },
        { "role": "legal", "text": "Weekdays 11:30 to 2:30", "rect_px": [90, 1640, 520, 40], "cap_height_px": 18 },
        { "role": "logo", "rect_px": [90, 90, 260, 60] }
      ]
    }
  ]
}
```

- **`targets`** use the global format reference, so one file can be declared for many networks' formats at once.
- **`elements`** are optional, but they make the precise checks possible: legibility, required legal line, blocked regions. A creative tool or build pipeline writes them automatically from the layout. Without them, a validator can estimate from the image, or mark those checks *not checked*.
- **`variant`** supports dynamic creative: each file can declare the condition it is built for.
- **`sha256`** ties approvals and reports to the exact file, as approval systems already do.
- **`tags`** (on the campaign or a file) say which conditions the creative meets, matching `required_elements[].when`, such as `"offer"`. An empty list means none apply. Leaving `tags` out means unknown, so a conditional requirement is reported `not_checked` rather than guessed.
- **`bytes`, `duration_s` and `max_flash_hz`** are optional declared values, for when a validator can't read or measure the file itself.
- **The file wins over the manifest.** When the file is available, a validator reads its type, pixels, size and duration from it, and reports any disagreement with the manifest as `CRS-MANIFEST-MISMATCH`. A SHA-256 mismatch is an error, because it isn't the file that was declared; other mismatches are warnings.

**The validation report** (written by any validator)

```json
{
  "opencrs": "0.1",
  "validator": { "name": "opencrs", "version": "0.1" },
  "file_sha256": "9f2c…",
  "target": "harbourline.example/retail/portrait-totem-75",
  "spec_updated_at": "2026-10-20T09:00:00+11:00",
  "result": "fail",
  "checks": [
    { "code": "CRS-BLOCKED-REGION", "severity": "error", "message": "Logo overlaps the network logo area", "rect_px": [880, 40, 160, 80] },
    { "code": "CRS-LEGIBILITY", "severity": "warning", "message": "Legal text is 18 px; 16 px minimum at 2 m. Passes with little margin" }
  ]
}
```

**Results:** `pass`, `warn` (passes with warnings), `fail` (at least one error), or `incomplete` (some checks couldn't run). Each check is `error`, `warning`, `info` or `not_checked`.

- **Precedence:** any `error` gives `fail`; otherwise any `not_checked` gives `incomplete`; otherwise any `warning` gives `warn`; otherwise `pass`. `info` never changes the result. Reports list findings only, not checks that passed.
- **A rule that doesn't apply isn't "not checked".** If a screen has no legibility rule, legibility is skipped. `not_checked` means a rule exists but the creative or screen lacks the data to test it. Otherwise every Core-level publisher's reports would come back `incomplete`.
- **One report per file and target.** A format target is checked against every screen using that format, because text size and word limits depend on each screen's viewing distance and dwell. A finding that applies to only some screens lists them under `screens`. Checks can also carry `element` (the role concerned) and `source` (`file` or `manifest`); reports can carry `file` and `checked_at`.
- **Little margin:** text that passes but is below 1.25 times its minimum size is a `warning`.
- **Duration** must match `slot_s` within one frame (`1 / fps`, or 0.04 s when no frame rate is given). Too short and too long are both errors.
- **Flash rate** can't be checked without frame analysis. Unless the manifest declares `max_flash_hz`, the check is `not_checked`. `no_flashing` means a limit of 0 Hz.
- **Categories match hierarchically:** a restricted `IAB7` also blocks `IAB7-5`, but not `IAB70`.

**Standard check codes in 0.1**

| Code | Checks |
| --- | --- |
| `CRS-PIXELS` | Canvas size matches the format |
| `CRS-MEDIA-TYPE` | File type is accepted |
| `CRS-FILE-SIZE` | Under the maximum size |
| `CRS-DURATION` | Motion length matches the slot |
| `CRS-SAFE-AREA` | Key elements are inside the safe area |
| `CRS-BLOCKED-REGION` | Nothing important sits under a blocked region |
| `CRS-LEGIBILITY` | Text meets the minimum size for the viewing distance |
| `CRS-MAX-WORDS` | Main message is within the word limit |
| `CRS-REQUIRED-ELEMENT` | Required elements, such as a legal line, are present and big enough |
| `CRS-CATEGORY` | The advertising category is allowed |
| `CRS-MOTION` | No strobe or flashing above the limit; motion allowed at all on this screen |
| `CRS-SPEC-STALE` | The spec used was current, not expired |
| `CRS-MANIFEST-MISMATCH` | The file matches what the manifest declares |
| `CRS-TARGET` | The target's network, format or screen exists in the document |
| `CRS-RULE-PACK` | Reported `not_checked` when a referenced rule pack couldn't be evaluated, and `info` when a pack is `unverified` |
| `CRS-MOTION-PROHIBITED` | A rule pack disallows motion on this screen, and the file is motion |
| `CRS-TRAFFIC-DEVICE` | Nothing resembles a traffic sign or signal. Always `not_checked`: needs visual review |
| `CRS-GLANCE-TEXT` | Word count and text size are within what a driver can read in one glance, from speed and viewing distance |
| `CRS-SEQUENCING` | Reminds that a pack prohibits message sequencing. Always `info`: scheduling isn't visible in one file |
| `CRS-PLAYBACK` | Playback limits from rule packs and the screen's rules (dwell, transition, failure state, luminance, school zones), for the platform to enforce. Always `info` |
| `CRS-RULE-CONFLICT` | Inherited rules can be met at all, such as position limits that overlap. Also reported against the `screens.json` itself |
| `CRS-REFERENCE` | Every screen's format and venue exist. Reported against the `screens.json` itself, and in reports for creative targeting an affected screen |

## Privacy, visibility and trust

**Publishers decide what's public. Formats and rules are meant to be public; exact locations and commercial details can stay private.**

**Two visibility levels**

- **Public:** anyone can fetch the document. Intended for formats, rules and delivery details, which agencies need anyway.
- **Partner:** listed in the index but served only to authenticated callers. Intended for exact screen locations, platform IDs and anything a publisher doesn't want competitors to map. Authentication uses OAuth 2.0 client credentials issued by the publisher, or signed URLs that expire.

A public document can describe screens without locations; the matching partner document adds them under the same screen IDs. Consumers merge the two. They match networks, venues, formats and screens by `id`, and fill in any field only one document sets. Where both set a field to different values, that's a conflict to report, not something to resolve silently. Only trusted documents from the same publisher are merged, and the merge keeps the latest `updated_at` and the earliest `expires_at`.

**Trust**

- **Domain authority is the trust model.** A document counts only if the publisher's own `/.well-known/opencrs.json` lists its exact URL, or a delegate's index lists it under a delegation the publisher's index grants (see Publishing and discovery).
- **Optional signatures** for documents passed around offline or through third parties: a detached JSON Web Signature over the document, with the publisher's public key published in the index.
- **Reports cite the spec version they used** (`spec_updated_at`), so a dispute about whether creative "met the spec" can be settled against the version in force at the time.

**No personal data:** OpenCRS describes screens and creative, not people. Audience and measurement fields are out of scope for 0.1, which keeps the format clear of privacy law obligations.

## Versioning, conformance and governance

**Small, stable and open: easy to adopt now, safe to grow later, and not owned by any one company forever.**

**Versioning**

- `major.minor` versions. Minor versions only add optional fields, so a 0.1 reader can always read a 0.2 document by ignoring what it doesn't know. Major versions may break things and are rare.
- Pre-1.0 (0.x) versions may still change, while real publishers test them. 1.0 is declared once at least three publishers and two validators use it in production.
- Deprecated fields stay readable for at least one major version.

**Extensions**

- Every object accepts `ext`, keyed by the extender's domain: `"ext": { "cms-a.example": { … } }`. That's how OpenRTB grew without breaking.
- An extension used by several publishers becomes a candidate for the core spec.

**Conformance levels**

| Level | A publisher provides | A validator checks |
| --- | --- | --- |
| Core | Formats with `pixels` and `media`, screens, an index | Pixels, media type, file size, duration, stale spec |
| Layout | Core, plus safe areas, blocked regions and segments | Core, plus safe area and blocked regions |
| Full | Layout, plus physical size, viewing context and rules | Everything, including legibility, words, required elements, category, motion and the published rule packs |

Publishers can start at Core in an afternoon from existing spec sheets, and add detail over time.

**The conformance suite** turns this table into tests. It's a set of cases in the repository's `conformance/` folder: input documents plus the outcome the spec requires, each tagged with its level and the spec section it tests. A validator meets a level when it passes every case at that level and below. Reports must match on each result and on each finding's code and severity, and on its element and screens where a case gives them. Wording is left to each validator, and extra `info` findings are allowed. A validator in any language can be tested by an adapter that reads each case and prints the reports. Claims to be "OpenCRS-compliant" at a level should rest on passing the suite.

**Governance**

- **Licence:** spec text under CC BY 4.0, schemas and reference code under Apache 2.0, so anyone can implement it, including competitors.
- **Now:** Mestra edits the spec in a public GitHub repository, with issues and proposals open to anyone.
- **Later:** once several publishers and validators use it, hand it to an independent industry or standards body. Which one, and how proposals are decided until then, is an open question below.

## A complete example

**One network, one centre, two formats and three screens, at the Full conformance level.** All names and values are fictional and illustrative.

```json
{
  "opencrs": "0.1",
  "publisher": "harbourline.example",
  "updated_at": "2026-10-20T09:00:00+11:00",
  "expires_at": "2027-01-20T00:00:00+11:00",
  "networks": [
    {
      "id": "retail",
      "name": "Harbourline Retail Screens",
      "rules": {
        "legibility": { "ratio": 150 },
        "restricted_categories": ["IAB7-5"],
        "required_elements": [
          { "role": "legal", "when": "offer", "min_cap_height_mm": 12 }
        ],
        "motion": { "max_flash_hz": 3 }
      },
      "venues": [
        { "id": "harbour-central", "name": "Harbour Central", "venue_type": 205 }
      ],
      "formats": [
        {
          "id": "portrait-totem-75",
          "pixels": [1080, 1920],
          "physical_mm": [940, 1670],
          "media": {
            "static": ["image/jpeg", "image/png"],
            "motion": ["video/mp4"],
            "slot_s": 10, "fps": 30, "max_bytes": 52428800,
            "colour_space": "srgb", "audio": false
          },
          "safe_area": { "top": 120, "right": 60, "bottom": 160, "left": 60 },
          "blocked_regions": [
            { "name": "network-logo", "rect_px": [880, 40, 160, 80] }
          ]
        },
        {
          "id": "atrium-wrap",
          "pixels": [5760, 1080],
          "physical_mm": [12000, 2250],
          "media": { "static": ["image/png"], "motion": ["video/mp4"], "slot_s": 15, "fps": 25 },
          "segments": [
            { "id": "left", "rect_px": [0, 0, 1920, 1080] },
            { "id": "front", "rect_px": [1920, 0, 1920, 1080], "primary": true },
            { "id": "right", "rect_px": [3840, 0, 1920, 1080] }
          ],
          "rules": { "notes": "Keep the main message on the front panel; side panels are seen at an angle." }
        }
      ],
      "screens": [
        {
          "id": "HC-L1-001", "format": "portrait-totem-75", "venue": "harbour-central",
          "placement": { "environment": "indoor", "level": "L1", "zone": "food court", "mounted_height_mm": 0, "facing": "N" },
          "viewing": { "distance_m": [2, 6], "dwell_s": 4, "approach": "head-on", "light": "indoor" },
          "platform_ids": { "cms-a": "frame-4411" }
        },
        {
          "id": "HC-L1-002", "format": "portrait-totem-75", "venue": "harbour-central",
          "viewing": { "distance_m": [3, 10], "dwell_s": 3, "approach": "oblique", "light": "indoor" },
          "overrides": { "blocked_regions": [ { "name": "column", "rect_px": [0, 1700, 300, 220] } ] },
          "platform_ids": { "cms-a": "frame-4412" }
        },
        {
          "id": "HC-ATR-001", "format": "atrium-wrap", "venue": "harbour-central",
          "viewing": { "distance_m": [8, 30], "dwell_s": 6, "light": "daylight" },
          "platform_ids": { "cms-b": "du-77812" }
        }
      ],
      "delivery": { "lead_time_hours": 72, "submit": { "portal": "https://screens.harbourline.example/submit" } }
    }
  ]
}
```

The two totems share one format, but the second has a column blocking its bottom-left corner and is seen from further away. A validator would therefore ask for larger text on it, and keep content clear of the corner.

## Relationship to existing standards

**OpenCRS fills the gap between existing standards rather than replacing any of them: they say where a screen is and how it's bought; OpenCRS says what creative it needs.**

| Standard | What it covers | How OpenCRS relates |
| --- | --- | --- |
| [OpenOOH venue taxonomy](https://github.com/openooh/venue-taxonomy/blob/main/specification-1.2.md) (supported by the OAAA) | Standard names for venue types | Reused directly: `venue_type` takes OpenOOH 1.2.1 numeric IDs. OpenOOH deprecated its string values (such as `retail.mall`), so OpenCRS doesn't accept them. OpenCRS adds no venue taxonomy of its own |
| OpenRTB and its DOOH extensions (IAB Tech Lab) | Describing inventory and bids in programmatic buying | Complementary. A bid request can carry an OpenCRS format reference in `ext`, so a buyer knows exactly what creative the screen needs |
| [Google out-of-home creative specification](https://developers.google.com/display-video/out-of-home-seller/reference/rest/v1/CreativeSpecification) | Width, height, image or video, file type, maximum size | A subset of an OpenCRS Format. Converts both ways for those five fields |
| IAB content taxonomy | Advertising categories | Used for `restricted_categories` and the manifest's `category` |
| ads.txt and sellers.json (IAB Tech Lab) | Publishers declaring on their own domain who may sell their inventory | The model for OpenCRS's index file and delegation |
| Platform IDs (a CMS's frame or display-unit identifiers) | Where content plays | Referenced in `platform_ids`, so validated creative can be published to the right place |

**The positioning line:** "OpenOOH says where a screen is. OpenRTB says how it's bought. OpenCRS says what creative it needs."

## Roadmap and open questions

**Ship 0.1 with real publishers before polishing anything; the spec earns its design from use.**

1. **0.1 draft (now):** this document, a JSON Schema, a public GitHub repository, and a free online validator.
2. **First publisher:** a founding publisher publishes a network.
3. **First consumer:** at least one tool consumes it in production.
4. **Independent validators:** at least two independent validators pass the conformance suite.
5. **0.2:** calibrate legibility defaults from real campaigns; add fields publishers ask for.
6. **1.0 and governance:** three or more publishers and two or more validators in production, then offer it to an independent industry or standards body.

**Open questions**

- [ ] Should composite surfaces (wraps, cubes) get their own object in 0.1, or stay as `segments`?
- [ ] Should rules reference an advertising code body, such as Ad Standards in Australia, or stay generic?
- [ ] How should HTML5 and dynamic templates be described: as a media type, or as a separate creative kind?
- [ ] Should the legibility ratio be defined per script or language (Latin, Chinese and so on)?
- [ ] Which independent industry or standards body should OpenCRS move to at 1.0, and how should proposals be decided until then?

## Sources

- [OAAA releases updated OpenOOH taxonomy](https://billboardinsider.com/oaaa-releases-updated-openooh-taxonomy-to-advance-industry-standardization-transparency/) (Billboard Insider, February 2026)
- [Google Display & Video 360 out-of-home seller API: CreativeSpecification](https://developers.google.com/display-video/out-of-home-seller/reference/rest/v1/CreativeSpecification)
- [Transport Corridor Outdoor Advertising and Signage Guidelines](https://www.transport.nsw.gov.au/system/files/media/documents/2023/transport-corridor-outdoor-advertising-and-signage-guidelines.pdf) (NSW Department of Planning and Environment, November 2017; hosted by Transport for NSW)
- [OMA Digital Signage Policy](https://oma.org.au/hubfs/digital_signage_policy_final_version_28022024_0.pdf) (Outdoor Media Association, February 2024; its earlier address on oma.org.au/sites/default/files now returns 404)
- [Roadside Advertising Manual, Edition 4, Technical Volume](https://www.tmr.qld.gov.au/-/media/busind/techstdpubs/Traffic-management/Roadside-advertising-manual/Roadside-Advertising-Manual---Technical-Volume.pdf?la=en) (Queensland Department of Transport and Main Roads, December 2022)
- [Policy and Assessment Guidelines for Digital Advertising Signs](https://www.mainroads.wa.gov.au/4a612c/globalassets/technical-commercial/technical-library/road-and-traffic-engineering/traffic-management/advertising-and-election-signs/policy-and-assessment-guidelines-for-digital-advertising-signs.pdf) (Main Roads Western Australia, revision 01, May 2025)
- [Advertising Signs Assessment Guidelines for Road Safety](https://dit.sa.gov.au/__data/assets/pdf_file/0007/172645/DIT-Advertising-Signs-Assessment-Guidelines.pdf) (South Australian Department of Planning, Transport and Infrastructure, August 2014; now published by the Department for Infrastructure and Transport)
- [Requirements and Guidelines for Illuminated Outdoor Advertising Signage](https://www.vic.gov.au/sites/default/files/2023-10/DTP-Requirements-and-Guidelines-for-Illuminated-Outdoor-Advertising-Signage.pdf) (Victorian Department of Transport and Planning, October 2023) and [Victoria Planning Provisions clause 52.05](https://planning-schemes.app.planning.vic.gov.au/VPP/clauses/52.05)
- [Tasmanian Planning Scheme, State Planning Provisions](https://www.planning.tas.gov.au/__data/assets/pdf_file/0005/784184/Tasmanian-Planning-Scheme-State-Planning-Provisions-effective-26-June-2024.PDF) (effective 26 June 2024), C1.0 Signs Code
