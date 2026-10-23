#!/usr/bin/env bash
# Times the CLI on the generated 20,000-screen network.
#
#   bench/run.sh            build, generate, time; print a Markdown table
#   OPENCRS_GO=path bench/run.sh   also time a Go CLI (the retired validator) beside it
#   BENCH_ENFORCE=1 bench/run.sh   exit 1 if the per-screen check takes more than
#                                  3x the figure in bench/ci-baseline.txt
#
# Needs hyperfine (brew install hyperfine) and uv. The table goes to stdout and,
# in CI, to the job summary.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

work=${BENCH_DIR:-$(mktemp -d)}
mkdir -p "$work"
trap '[ -n "${BENCH_DIR:-}" ] || rm -rf "$work"' EXIT

cargo build --release -p opencrs-cli --locked >&2
bin=${OPENCRS:-$root/target/release/opencrs}

uv run bench/gen-network.py "$work" 20000 400 examples/harbourline/lunch-portrait.png >&2
S=$work/screens.json
C=$work/creative.json
CS=$work/creative-screens.json

# Peak resident set size in MB, of one run with stdout discarded.
if [ "$(uname)" = Darwin ]; then
  rss() { /usr/bin/time -l "$@" 2>&1 >/dev/null | awk '/maximum resident/ {printf "%.0f", $1 / 1048576}'; }
else
  rss() { /usr/bin/time -v "$@" 2>&1 >/dev/null | awk '/Maximum resident/ {printf "%.0f", $NF / 1024}'; }
fi

# mean_ms NAME CMD...: hyperfine's mean wall time in ms (exit status ignored:
# a failing report is not a failing run).
mean_ms() {
  local name=$1; shift
  hyperfine -N -i --warmup 3 --min-runs 10 --export-json "$work/$name.json" "$*" >&2
  # hyperfine 1.x writes "mean"; 2.x writes each run's measurements.
  python3 - "$work/$name.json" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))["results"][0]
if "mean" in r:
    mean = r["mean"]
else:
    t = [m["time_wall_clock"]["value"] for m in r["measurements"]]
    mean = sum(t) / len(t)
print(round(mean * 1000))
PY
}

declare -a rows
run() { # LABEL KEY CMD...
  local label=$1 key=$2; shift 2
  local ms mb
  ms=$(mean_ms "$key" "$@")
  mb=$(rss "$@" || true)
  rows+=("| $label | $ms ms | $mb MB |")
  echo "$key=$ms" >> "$work/results.txt"
}

run_all() { # NAME BIN
  local name=$1 b=$2
  run "$name: schema, screens.json + creative.json" "$name-schema" "$b" schema "$S" "$C"
  run "$name: check, 400 format reports" "$name-format" "$b" check -spec "$S" "$C"
  run "$name: check, 20,000 screen reports" "$name-screen" "$b" check -spec "$S" "$CS"
}

: > "$work/results.txt"
run_all rust "$bin"
if [ -n "${OPENCRS_GO:-}" ]; then run_all go "$OPENCRS_GO"; fi

table=$(echo '| Run | Mean wall time | Peak RSS |'; echo '| --- | --- | --- |'; printf '%s\n' "${rows[@]}")
echo "$table"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  { echo "### Benchmark: 20,000-screen network"; echo; echo "$table"; } >> "$GITHUB_STEP_SUMMARY"
fi

if [ "${BENCH_ENFORCE:-}" = 1 ]; then
  base=$(grep -v '^#' bench/ci-baseline.txt | tr -d ' \n')
  got=$(grep '^rust-screen=' "$work/results.txt" | cut -d= -f2)
  limit=$((base * 3))
  echo "per-screen check: $got ms; limit $limit ms (3 x baseline $base ms)"
  if [ "$got" -gt "$limit" ]; then
    echo "per-screen check regressed past 3x the recorded baseline" >&2
    exit 1
  fi
fi
