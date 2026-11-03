#!/usr/bin/env bash
# Pass 2: llama.cpp runtime options on the best device/thread setting from
# pass 1 (CPU, 4 threads, one per performance core: taskset -c 0,2,4,6).
# Each configuration: 5 repetitions of tg64 and pp512, raw CSV kept.
# usage: ml/scripts/local-inference/pass2.sh <model.gguf> <out.tsv>
set -u
M=$1; OUT=$2
RAW="${OUT%.tsv}-raw"; mkdir -p "$RAW"
PARSER="$(dirname "$0")/parse_bench.py"
printf "config\toptions\ttest\tmean_tps\tsd_tps\n" > "$OUT"

# run <name> <taskset cpus> <llama-bench options...>
run() {
  local name=$1 cpus=$2; shift 2
  local res
  res=$(taskset -c "$cpus" llama-bench -m "$M" -p 512 -n 64 -r 5 -o csv "$@" 2>/dev/null)
  echo "$res" > "$RAW/$name.csv"
  echo "$res" | python3 -I "$PARSER" | while IFS=$'\t' read -r test avg sd; do
    printf "%s\t%s\t%s\t%.2f\t%.2f\n" "$name" "$*" "$test" "$avg" "$sd" >> "$OUT"
  done
  echo "done: $name" >&2
}

P=0,2,4,6          # one thread per performance core
BASE=(-ngl 0 -t 4)

run base             $P    "${BASE[@]}"
run cpu-only-dev     $P    "${BASE[@]}" -dev none
run no-op-offload    $P    "${BASE[@]}" -nopo 1
run repack-off       $P    "${BASE[@]}" --repack 0
run flash-attn-on    $P    "${BASE[@]}" -fa on
run flash-attn-off   $P    "${BASE[@]}" -fa off
run poll-0           $P    "${BASE[@]}" --poll 0
run poll-100         $P    "${BASE[@]}" --poll 100
run strict-mask      0-11  "${BASE[@]}" -C 0x55 --cpu-strict 1
run prio-2           $P    "${BASE[@]}" --prio 2
run ubatch-256       $P    "${BASE[@]}" -ub 256
run ubatch-1024      $P    "${BASE[@]}" -ub 1024 -b 2048
run base-repeat      $P    "${BASE[@]}"
