#!/usr/bin/env bash
# Clean device/thread/pinning matrix for Qwen3-4B Q4_K_M with llama-bench.
# Each configuration: 5 repetitions of tg64 (generation) and pp512 (prompt),
# with the P-core clock sampled every second during the run.
# usage: ml/scripts/local-inference/matrix.sh <model.gguf> <out.tsv>   (SMOKE=1 for a seconds-long check)
set -u
M=$1; OUT=$2
RAW="${OUT%.tsv}-raw"; mkdir -p "$RAW"
PARSER="$(dirname "$0")/parse_bench.py"
SMOKE="${SMOKE:-0}"
printf "config\tcpus\tngl\tthreads\ttest\tmean_tps\tsd_tps\tpcore_mhz_avg\tpcore_mhz_min\n" > "$OUT"

pcore_mhz() { awk '/cpu MHz/ {i++; if (i<=8) {s+=$4; if (min==""||$4<min) min=$4}} END {printf "%d %d\n", s/8, min}' /proc/cpuinfo; }

run() {
  local name=$1 cpus=$2 ngl=$3 threads=$4
  local log; log=$(mktemp)
  ( while true; do pcore_mhz; sleep 1; done ) > "$log.mhz" &
  local sampler=$!
  local res
  if [ "$SMOKE" = 1 ]; then reps=1; np=16; ng=4; else reps=5; np=512; ng=64; fi
  res=$(taskset -c "$cpus" llama-bench -m "$M" -ngl "$ngl" -t "$threads" -p $np -n $ng -r $reps -o csv 2>/dev/null)
  kill $sampler 2>/dev/null; wait $sampler 2>/dev/null
  local mhz
  mhz=$(awk '{s+=$1; n++; if (min==""||$2<min) min=$2} END {if (n) printf "%d\t%d", s/n, min}' "$log.mhz")
  # csv columns include n_prompt, n_gen, avg_ts, stddev_ts
  echo "$res" > "$RAW/$name.csv"
  echo "$res" | python3 -I "$PARSER" | while IFS=$'\t' read -r test avg sd; do
    printf "%s\t%s\t%s\t%s\t%s\t%.2f\t%.2f\t%s\n" "$name" "$cpus" "$ngl" "$threads" "$test" "$avg" "$sd" "$mhz" >> "$OUT"
  done
  rm -f "$log" "$log.mhz"
  echo "done: $name" >&2
}

ALL=0-11
run igpu-vulkan          $ALL      -1 4
run cpu-t4               $ALL       0 4
run cpu-t6               $ALL       0 6
run cpu-t8               $ALL       0 8
run cpu-t12              $ALL       0 12
run cpu-t4-pcores-ht     0-7        0 4
run cpu-t8-pcores-ht     0-7        0 8
run cpu-t4-one-per-pcore 0,2,4,6    0 4
run cpu-t4-repeat        $ALL       0 4
