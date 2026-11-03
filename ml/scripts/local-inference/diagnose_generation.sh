#!/usr/bin/env bash
# Why is generation in the real server request (~7.3 tokens/s) so much slower
# than llama-bench tg64 (~16 tokens/s on CPU)? Three differences are isolated:
#   1. context depth: llama-bench generating after a 2,400-token context (-d 2400)
#   2. the LoRA adapter: the server without --lora
#   3. the JSON-schema grammar: the server without response_format
# usage: ml/scripts/local-inference/diagnose_generation.sh <model.gguf> <adapter.gguf> <out-dir>
set -u
M=$1; A=$2; OUT=$3; mkdir -p "$OUT"
ROOT=$(pwd); P=0,2,4,6

echo "### 1. llama-bench generation at context depth 0 vs 2400 (3 repetitions)"
bench() {   # bench <name> <cpus> <options...>
  local name=$1 cpus=$2; shift 2
  taskset -c "$cpus" llama-bench -m "$M" -p 0 -n 64 -d 0,2400 -r 3 -o csv "$@" 2>/dev/null > "$OUT/bench-$name.csv"
  python3 -I "$ROOT/ml/scripts/local-inference/parse_depth.py" "$OUT/bench-$name.csv" "$name"
}
bench igpu-vulkan       0-11 -ngl 99 -t 8
bench cpu4-op-offload   $P   -ngl 0 -t 4
bench cpu4-no-op-offload $P  -ngl 0 -t 4 -nopo 1

server() {   # server <name> <cpus> <seq> <env> <llama-server options...>
  local name=$1 cpus=$2 seq=$3 env=$4; shift 4
  taskset -c "$cpus" llama-server --model "$M" --alias local --host 127.0.0.1 --port 8000 \
    --ctx-size 4096 --parallel 1 "$@" > "$OUT/$name-server.log" 2>&1 &
  local pid=$!
  for _ in $(seq 1 120); do curl -s localhost:8000/health | grep -q ok && break; sleep 1; done
  echo "== $name (env: ${env:-none}; options: $*)"
  env $env python3 -I "$ROOT/ml/scripts/local-inference/measure.py" "$ROOT" http://127.0.0.1:8000/v1 "$seq" | tee "$OUT/$name.txt"
  kill $pid; sleep 3
}
CPU=(-ngl 0 --threads 4 --threads-batch 4 --no-op-offload)
IGPU=(--threads 8 --threads-batch 8)
echo; echo "### 2. adapter cost: server with and without --lora"
server cpu-lora        $P   E,E,E,E ""          --lora "$A" "${CPU[@]}"
server cpu-no-lora     $P   E,E,E,E ""          "${CPU[@]}"
server igpu-lora       0-11 E,E,E,E ""          --lora "$A" "${IGPU[@]}"
server igpu-no-lora    0-11 E,E,E,E ""          "${IGPU[@]}"
echo; echo "### 3. grammar cost: CPU server without the JSON-schema grammar"
server cpu-lora-no-schema $P E,E,E,E NO_SCHEMA=1 --lora "$A" "${CPU[@]}"
