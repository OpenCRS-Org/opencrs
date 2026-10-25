# Roadmap

OpenCRS 0.1 is a draft. This page lists what comes next: the tools that
make it easy to publish and check against specs, then the work on the
specification itself on the way to 1.0.

Each tool has an issue, labelled
[`roadmap`](https://github.com/OpenCRS-Org/opencrs/issues?q=label%3Aroadmap),
where you can follow progress and comment. Dates are targets, not promises,
and the order can change when publishers or agencies tell us what they need
first. Sizes are rough: S is a few days, M about a week, L two weeks or more.

## Tools

### 1. Quick wins, by November 2026

[Milestone](https://github.com/OpenCRS-Org/opencrs/milestone/1)

| Tool | Size | What it does |
|---|---|---|
| [Calculators](https://github.com/OpenCRS-Org/opencrs/issues/12) | S | Legibility (distance to minimum text size) and dwell (speed to how much a driver can read). |
| [Publisher check](https://github.com/OpenCRS-Org/opencrs/issues/13) | S–M | Enter your domain, see every problem with your published specs and how to fix it. |
| [GitHub Action](https://github.com/OpenCRS-Org/opencrs/issues/14) | S | Check creative, or your own `screens.json`, on every push. |
| [Badge](https://github.com/OpenCRS-Org/opencrs/issues/15) | S | "Specs published with OpenCRS", for media owners' websites. |

### 2. Publisher adoption, by January 2027

[Milestone](https://github.com/OpenCRS-Org/opencrs/milestone/2)

| Tool | Size | What it does |
|---|---|---|
| [Spec builder](https://github.com/OpenCRS-Org/opencrs/issues/16) | L | A form that produces a valid `screens.json`, with safe areas and blocked regions drawn on the screen. |
| [Spec page generator](https://github.com/OpenCRS-Org/opencrs/issues/17) | M | Turns `screens.json` into a clear spec page and a printable PDF, so the PDF is never out of date. |
| [Public index](https://github.com/OpenCRS-Org/opencrs/issues/18) | M–L | A neutral, searchable directory of every publisher's specs, checked daily. |

### 3. Importer, templates and simulator, by March 2027

[Milestone](https://github.com/OpenCRS-Org/opencrs/milestone/3)

| Tool | Size | What it does |
|---|---|---|
| [PDF importer](https://github.com/OpenCRS-Org/opencrs/issues/19) | L | Upload an existing spec sheet, get a draft `screens.json` back to review. |
| [Design templates](https://github.com/OpenCRS-Org/opencrs/issues/20) | M, then L | Artboards for any format with safe areas and blocked regions drawn in: SVG, PDF and InDesign first, then Figma and Canva. |
| [View-from-distance simulator](https://github.com/OpenCRS-Org/opencrs/issues/21) | M | See a creative as a shopper does at 2, 5 or 10 m, or as a driver at 60 km/h. |

### Ongoing

[Milestone](https://github.com/OpenCRS-Org/opencrs/milestone/4)

| Work | What it covers |
|---|---|
| [Libraries](https://github.com/OpenCRS-Org/opencrs/issues/22) | Python bindings, then TypeScript types, alongside Rust, Go and WebAssembly. |
| [Rule packs](https://github.com/OpenCRS-Org/opencrs/issues/23) | Independent review of the six Australian packs, then New Zealand, the UK and US states. |

## Towards 1.0

The specification keeps its open questions in the
[spec](https://opencrs.dev/spec/0.1/) itself. These larger areas are planned
for before 1.0. Most wait for real publishers, screens or partners to design
against, so they aren't dated yet.

- **Trust:** signed documents, proof of domain ownership, and delegation, for example from a landlord to the media owner who sells its screens.
- **Composite and unusual surfaces:** multi-panel walls, wraps, curved and LED mesh screens, and synchronised playback.
- **Media depth:** audio, codec and bitrate profiles, colour space and HDR, and brightness by time of day.
- **Change feed:** a formal format for announcing what changed in a publisher's specs.
- **Dynamic creative:** sets of variants with the conditions for each and a required default.
- **Industry links:** OpenOOH venue types, programmatic fields, and CMS playlists.
- **Conformance:** a formal test suite and levels, and the rules for claiming conformance.
- **Extensions:** how vendor fields in `ext` are promoted into the core.
- **Governance:** approvals, versioning and deprecation, contribution terms, and the handover to an independent industry or standards body.
- **Accessibility and localisation:** multilingual creative, contrast and motion sensitivity.

If one of these matters to you, [open an issue](https://github.com/OpenCRS-Org/opencrs/issues)
with the case you need it for.
