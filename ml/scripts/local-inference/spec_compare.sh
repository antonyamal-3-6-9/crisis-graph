#!/usr/bin/env bash
# Speculative decoding on the real request: the same 6 extractions (the
# first is cold) against the default server and each speculative variant.
# Prints llama.cpp timings, draft acceptance and a fingerprint of each
# answer, so identical output can be checked against the baseline.
# usage: ml/scripts/local-inference/spec_compare.sh <model.gguf> <adapter.gguf> <draft.gguf> <out-dir>
set -u
M=$1; A=$2; D=$3; OUT=$4; mkdir -p "$OUT"
ROOT=$(pwd)
SEQ=E,E,E,E,E,E

start() {   # start <name> <llama-server options...>
  local name=$1; shift
  llama-server --model "$M" --lora "$A" --alias local --host 127.0.0.1 --port 8000 \
    --ctx-size 4096 --parallel 1 --threads 8 --threads-batch 8 "$@" > "$OUT/$name-server.log" 2>&1 &
  echo $! > "$OUT/server.pid"
  for _ in $(seq 1 180); do curl -s localhost:8000/health | grep -q ok && return 0; sleep 1; done
  echo "server $name did not start"; return 1
}
stop() { kill "$(cat "$OUT/server.pid")" 2>/dev/null; sleep 3; }

measure() {   # measure <name> <llama-server options...>
  local name=$1
  echo "== $name (options: ${*:2}) power=$(cat /sys/class/power_supply/ADP1/online)"
  if start "$@"; then
    python3 -I "$ROOT/ml/scripts/local-inference/measure.py" "$ROOT" http://127.0.0.1:8000/v1 "$SEQ" | tee "$OUT/$name.txt"
  fi
  stop
}

measure baseline
measure draft-n3   --spec-type draft-simple -md "$D" --spec-draft-n-max 3
measure draft-n8   --spec-type draft-simple -md "$D" --spec-draft-n-max 8
measure draft-n16  --spec-type draft-simple -md "$D" --spec-draft-n-max 16
measure ngram-simple --spec-type ngram-simple
measure ngram-mod    --spec-type ngram-mod
