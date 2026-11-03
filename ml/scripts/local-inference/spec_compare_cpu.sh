#!/usr/bin/env bash
# Speculative decoding with the target model on the CPU (follow-up to
# spec_compare.sh, where multi-token verification on the iGPU was slow).
# The target runs on 4 performance-core threads without op offload; the
# draft runs on the CPU or on the iGPU. Same 6 extractions, same answer
# fingerprints.
# usage: ml/scripts/local-inference/spec_compare_cpu.sh <model.gguf> <adapter.gguf> <draft.gguf> <out-dir>
set -u
M=$1; A=$2; D=$3; OUT=$4; mkdir -p "$OUT"
ROOT=$(pwd)
SEQ=E,E,E,E,E,E
CPU=(-ngl 0 --threads 4 --threads-batch 4 --no-op-offload)

start() {   # start <name> <llama-server options...>
  local name=$1; shift
  taskset -c 0,2,4,6 llama-server --model "$M" --lora "$A" --alias local --host 127.0.0.1 --port 8000 \
    --ctx-size 4096 --parallel 1 "${CPU[@]}" "$@" > "$OUT/$name-server.log" 2>&1 &
  echo $! > "$OUT/server.pid"
  for _ in $(seq 1 180); do curl -s localhost:8000/health | grep -q ok && return 0; sleep 1; done
  echo "server $name did not start"; return 1
}
stop() { kill "$(cat "$OUT/server.pid")" 2>/dev/null; sleep 3; }

measure() {   # measure <name> <llama-server options...>
  local name=$1
  echo "== $name (options: ${CPU[*]} ${*:2}) power=$(cat /sys/class/power_supply/ADP1/online)"
  if start "$@"; then
    python3 -I "$ROOT/ml/scripts/local-inference/measure.py" "$ROOT" http://127.0.0.1:8000/v1 "$SEQ" | tee "$OUT/$name.txt"
  fi
  stop
}

measure cpu-baseline
measure cpu-draft-igpu-n3 --spec-type draft-simple -md "$D" --spec-draft-n-max 3 -devd Vulkan0 -ngld 99
measure cpu-draft-igpu-n8 --spec-type draft-simple -md "$D" --spec-draft-n-max 8 -devd Vulkan0 -ngld 99
measure cpu-draft-cpu-n3  --spec-type draft-simple -md "$D" --spec-draft-n-max 3 -ngld 0
