#!/usr/bin/env bash
# End-to-end llama-server comparison: the same request sequence against
# each configuration (cold first request, then warm extractions with
# landmark-refinement calls in between). Prints llama.cpp timings and a
# fingerprint of each answer, so identical output can be checked.
# usage: ml/scripts/local-inference/server_compare.sh <model.gguf> <adapter.gguf> <out-dir>
set -u
M=$1; A=$2; OUT=$3; mkdir -p "$OUT"
ROOT=$(pwd)
SEQ=E,E,E,R,E,R,E,E,E

start() {   # start <name> <taskset cpus> <llama-server options...>
  local name=$1 cpus=$2; shift 2
  taskset -c "$cpus" llama-server --model "$M" --lora "$A" --alias local --host 127.0.0.1 --port 8000 \
    --ctx-size 4096 --parallel 1 "$@" > "$OUT/$name-server.log" 2>&1 &
  echo $! > "$OUT/server.pid"
  for _ in $(seq 1 120); do curl -s localhost:8000/health | grep -q ok && break; sleep 1; done
}
stop() { kill "$(cat "$OUT/server.pid")" 2>/dev/null; sleep 3; }

measure() {   # measure <name> <taskset cpus> <llama-server options...>
  local name=$1
  start "$@"
  echo "== $name (options: ${*:3})"
  python3 -I "$ROOT/ml/scripts/local-inference/measure.py" "$ROOT" http://127.0.0.1:8000/v1 "$SEQ" | tee "$OUT/$name.txt"
  stop
}

measure default-igpu       0-11     --threads 8 --threads-batch 8
measure cpu4-no-op-offload 0,2,4,6  -ngl 0 --threads 4 --threads-batch 4 --no-op-offload
measure cpu4-dev-none      0,2,4,6  -ngl 0 --threads 4 --threads-batch 4 -dev none
