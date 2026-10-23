# Benchmarks

`bench/run.sh` builds the CLI in release mode, generates the 20,000-screen
network with `gen-network.py` (400 formats, a 4.7 MB `screens.json`), and times
three runs with `hyperfine`, reporting mean wall time and peak resident set
size:

- `schema` on `screens.json` and `creative.json`;
- `check` with `creative.json` (400 format reports);
- `check` with `creative-screens.json` (20,000 screen reports).

```sh
bench/run.sh                                  # Rust only
OPENCRS_GO=/path/to/opencrs-go bench/run.sh   # beside a Go CLI built from `6784da9`, the last commit that had one
BENCH_ENFORCE=1 bench/run.sh                  # CI: fail past 3x bench/ci-baseline.txt
```

Needs `hyperfine` (`brew install hyperfine`) and `uv`.

## Results

Apple M5 Pro (15 cores), macOS, release builds, 2026-10-23. Go is the retired
validator, built with `go build ./cmd/opencrs` at `6784da9`. The design's targets: the
20,000-screen `check` under 0.1 s and memory no higher than Go's.

| Run | Rust | Go |
| --- | --- | --- |
| `schema` on screens.json + creative.json | 55 ms, 63 MB | 156 ms, 76 MB |
| `check`, 400 format reports | 49 ms, 74 MB | 174 ms, 79 MB |
| `check`, 20,000 screen reports | 87 ms, 80 MB | 536 ms, 79 MB |

The design's Go baseline was 0.15 s, 0.15 s and 0.48 s (80, 80 and 85 MB);
this table measures both CLIs side by side on one machine, so it is the one to
compare.

On memory, that means the 20,000-screen check meets the target against the
design's 85 MB but is 1 MB over the side-by-side Go figure (80 MB against
79 MB): within run-to-run noise, with no headroom. The side-by-side figure is
the baseline that counts; a change that adds memory to that run should win it
back.

The Rust CLI checks on all cores (the `parallel` feature, which uses rayon);
output is byte-identical to the serial build and to Go's. With `RAYON_NUM_THREADS=1`
the 20,000-screen check takes about 0.45 s.

## What got it there

- Checking and probing are parallel over (file, target) pairs and files, and
  results are gathered in input order.
- Text output is streamed in chunks of 2,048 pairs through a buffered stdout,
  so a network-sized check never holds all its reports.
- `Screen` boxes its rarely-set `overrides`, `rules` and `placement`, which
  took it from 1,264 to 232 bytes; 20,000 screens were 25 MB larger before.
- Peak memory is set by schema validation, which builds a `serde_json::Value`
  of the whole document (about 40 MB for `screens.json`).

## CI

The `bench` job in `.github/workflows/cli.yml` runs `BENCH_ENFORCE=1
bench/run.sh`, writes the table to the job summary, and fails if the
20,000-screen check exceeds 3x the figure in `ci-baseline.txt`. That figure is
provisional until the first CI run records the real one.
