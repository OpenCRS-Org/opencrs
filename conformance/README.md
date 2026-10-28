# OpenCRS conformance suite

Test cases that pin down what the OpenCRS 0.1 spec requires of a validator. Each case is a set of input documents and the outcome the spec requires. Any validator, in any language, can run the cases and show which conformance level it meets.

## Levels

Each level includes the ones before it, matching the spec's conformance table.

| Level | What a validator must get right |
| --- | --- |
| Core | Pixels, media type, file size, duration (within one frame), stale specs, unknown targets and broken references, several targets and several publishers, the file winning over the manifest (`CRS-MANIFEST-MISMATCH`), screen overrides of specs, and reading later 0.x documents |
| Layout | Core, plus safe areas and blocked regions, including blocked regions screens add or remove in `overrides` |
| Full | Layout, plus legibility (the worked example, rounding up, per-screen minimums, direct and ratio minimums together, a direct minimum when the ratio can't be worked out), word limits and reading rate, required elements (conditional, sized, restated, removed, positioned), categories, motion and flash rate, restrictions that only tighten, rule conflicts, result precedence, and the published rule packs (unknown packs, motion prohibited whatever the screen says, glance text, unverified packs flagged) |

## A case

Each directory under `0.1/cases/` holds one case:

- `case.json` says what the case tests and what must happen:
  - `id`, which starts with the level.
  - `level`: `core`, `layout` or `full`.
  - `spec`: the spec section it tests.
  - `description`: what it checks, in plain words.
  - `now`: the validation time, used for staleness.
  - `specs`: the `screens.json` files to validate against.
  - `creative`: the `creative.json` to check.
  - `expect`: the required reports.
  - `strictest`: for a case about strictest requirements instead of a creative (see below); it has no `creative` or `expect`.
  - `optional`: set to `strictest` on those cases. A case with it is for a feature a validator may offer; it is not part of any level.
- The input documents, and any creative files a case needs to be read rather than taken from the manifest (for example `core-file-wins` and `core-sha-mismatch`).

## What has to match

- **Reports:** exactly one per expected file and target pair. A target belonging to another publisher gets no report.
- **Results:** each report's `result` must match.
- **Findings:**
  - Every expected finding must be present, with the same `code` and `severity`.
  - `element` must match where the case gives one.
  - `screens` must match where the case gives them. A finding with no `screens` must apply to every screen of the target, so it lists none. A finding lists at most 1,000 screens, in document order; when it applies to more, `screens_total` must give how many, and is otherwise absent.
  - Any other `error`, `warning` or `not_checked` finding fails the case. Extra `info` findings are allowed.
- **Left to each validator:** message wording, `rect_px`, `source` and the report's other fields.

## Strictest cases (optional)

Strictest requirements are an optional extra. A validator that doesn't offer them still meets Core, Layout and Full, and need not run these cases; one that does offer them runs them to show it follows the spec's rules for them, and can claim that separately. The cases are tagged `"optional": "strictest"` and live beside the others, with ids that start with the level they would sit at.

- **Skip them:** `opencrs conformance --no-strictest` (the verdict then says they were not tested). With `--cmd`, an adapter that doesn't support them should be run this way; the level verdict is the same either way.
- **Opt in:** run without the flag. The report gives the level verdict from the required cases, then a separate line for the optional ones, and the command fails if either has a failure. The adapter gets each such case directory and prints the results as described below. In another runner, filter on `optional`.
- The reference validator and the runners in this repository (CLI, Go, JS, npm and Python) run and pass them.

A case with `strictest` tests the combination of every screen's requirements into one (the spec's "Strictest requirements"). It has one `screens.json` in `specs`, and a list of `strictest` entries:

- `ref`: the format or screen reference to resolve.
- `expect`: members the result must have, with these values. An object is matched by the members given, others aren't looked at; a list must match exactly; numbers are equal if their values are.
- `notes`: how many `notes` the result must have. Their wording is left to each validator.
- `absent`: members, as dotted paths such as `media.motion`, the result must not have.
- `error`: instead of the above, text the error must contain, such as for a format whose screens differ in pixels.

An entry must give `expect`, `notes` or `error`.

A validator prints one result for each entry, in order, as a JSON array: the strictest requirements, or `{"error": "..."}`.

## Running it

The reference validator:

```sh
opencrs conformance                    # Full level, conformance/0.1
opencrs conformance -level core -v     # one level, listing passing cases too
```

Rule-pack cases reference the packs published in the repository's `rulepacks/` folder by `name@version`; another validator loads them from there.

Any other validator: give `-cmd` a command. It's run by the shell with each case directory as its last argument, and must print the reports as a JSON array on stdout (exiting non-zero for failed creative is fine), or for a strictest case, the results. An adapter usually reads `case.json` and calls the validator. For example, this adapter runs the `opencrs` CLI itself, and passes every case:

```sh
#!/bin/sh
dir="$1"
now=$(jq -r .now "$dir/case.json")
specs=$(jq -r '.specs[] | "-spec " + .' "$dir/case.json")
cd "$dir" && exec opencrs check -json -now "$now" $specs "$(jq -r .creative case.json)"
```

```sh
opencrs conformance -cmd ./my-adapter.sh   # from a source checkout: cargo run -p opencrs-cli -- conformance -cmd ./my-adapter.sh
```

## Coverage

Each normative rule in the spec, and the cases that test it.

| Spec rule | Cases |
| --- | --- |
| Canvas size (`CRS-PIXELS`) | `core-pixels`, `core-override-pixels`, `core-file-wins` |
| A finding lists at most 1,000 screens and counts the rest (`screens_total`) | `core-screens-truncated` |
| Accepted file types (`CRS-MEDIA-TYPE`) | `core-media-type` |
| Maximum file size (`CRS-FILE-SIZE`) | `core-file-size` |
| Duration matches the slot within one frame (`CRS-DURATION`) | `core-duration`, `core-file-duration` |
| Expired documents (`CRS-SPEC-STALE`) | `core-spec-stale` |
| Unknown targets (`CRS-TARGET`) and broken references (`CRS-REFERENCE`) | `core-target-unknown`, `core-broken-reference` |
| Targets: global references, other publishers, one report per file and target | `core-other-publisher`, `core-multiple-targets`, `core-two-publishers`, `full-legibility-per-screen` |
| The file wins over the manifest (`CRS-MANIFEST-MISMATCH`) | `core-file-wins`, `core-sha-mismatch`, `core-file-duration` |
| A 0.1 reader reads later 0.x documents | `core-forward-compatible` |
| Specs follow the most specific level | `core-override-pixels` |
| A rule that doesn't apply isn't reported as not checked | `core-pass`, `layout-no-elements`, `full-still-not-motion`, `full-non-text-elements` |
| Safe area (`CRS-SAFE-AREA`) | `layout-safe-area`, `layout-no-elements` |
| Blocked regions, added and removed by screen overrides (`CRS-BLOCKED-REGION`) | `layout-blocked-region`, `layout-override-adds-region`, `layout-no-elements` |
| Legibility from a ratio and viewing distance, the worked example | `full-legibility-worked-example`, `full-legibility-per-screen` |
| Minimum sizes round up | `full-legibility-round-up` |
| Direct and ratio minimums together: the larger wins | `full-legibility-direct-and-ratio` |
| A ratio that can't be worked out; a direct minimum still fails text below it | `full-legibility-direct-floor` |
| Warning below 1.25 times the minimum | `full-legibility-worked-example`, `full-legibility-direct-and-ratio`, `full-required-restated` |
| Which roles are text | `full-non-text-elements` |
| Word limit from dwell and reading rate (`CRS-MAX-WORDS`) | `full-max-words`, `full-reading-rate` |
| Required elements: tags, size, position (`CRS-REQUIRED-ELEMENT`) | `full-required-conditional`, `full-required-size`, `full-required-restated` |
| Restated required elements merge; `remove` drops them | `full-required-restated`, `full-remove-required` |
| Categories match hierarchically (`CRS-CATEGORY`) | `full-category-restricted`, `full-category-exact`, `full-category-boundary`, `full-category-missing` |
| Motion and flash rate (`CRS-MOTION`) | `full-motion-placement`, `full-flash-rate`, `full-still-not-motion` |
| Restrictions only tighten | `full-restrictions-tighten`, `full-legibility-direct-and-ratio`, `full-required-restated` |
| Position limits that can't be met (`CRS-RULE-CONFLICT`) | `full-rule-conflict` |
| Result precedence: error, then not checked, then warning | `full-result-precedence`, `full-required-conditional`, `full-legibility-direct-floor` |
| Unknown rule packs are not checked; the rest is still evaluated (`CRS-RULE-PACK`) | `full-pack-unknown` |
| A referenced pack applies in full; packs prohibit motion whatever the screen says (`CRS-MOTION-PROHIBITED`) | `full-pack-motion`, `full-pack-unverified` |
| Traffic-device resemblance always needs a person; sequencing is info (`CRS-TRAFFIC-DEVICE`, `CRS-SEQUENCING`) | `full-pack-motion`, `full-pack-glance-text` |
| Reading at a glance, the worked example (`CRS-GLANCE-TEXT`) | `full-pack-glance-text`, `full-pack-motion` |
| Playback limits are reported as info (`CRS-PLAYBACK`) | `full-pack-glance-text` |
| Packs short of `draft` are flagged | `full-pack-unverified` |
| Strictest requirements: largest safe-area insets and all blocked regions | `layout-strictest-safe-area` |
| Strictest requirements: minimum text sizes in pixels at each screen's own distance and density, the largest for each role | `full-strictest-cap-height` |
| Strictest requirements: conditional elements left out of the rules and listed, with their sizes in pixels and positions | `full-strictest-conditional` |
| Strictest requirements: a type any screen lists as motion is motion | `full-strictest-motion-type` |
| Strictest requirements: motion not allowed on a screen leaves no motion types, and flashing is 0 Hz | `full-strictest-motion-off` |
| Strictest requirements: smallest maxima, largest minima, unions | `full-strictest-limits` |
| Strictest requirements: file types and codecs every screen accepts; an empty intersection is noted | `full-strictest-media` |
| Strictest requirements: a screen reference gives that screen's own requirements | `full-strictest-screen-ref` |
| Strictest requirements: screens of different pixel sizes are an error | `core-strictest-pixels` |

**Not covered yet:**
- Individual pack values (dwell, luminance and transition by state): these are info for the screen platform, not creative checks, and are tested in the reference validator's own tests.
- Discovery and trust, which need served files rather than inputs on disk, and whose delegation rules are still provisional.
- Case-sensitive IDs.
- Segment rules, which the spec doesn't define yet.

## Adding a case

Work out the expected outcome from the spec text, not from what a validator happens to output, and cite the section in `spec`. Use fictional names on the reserved `.example` domain. `cargo test -p opencrs-cli --test conformance` checks that every case is well formed and that the reference validator passes it.

