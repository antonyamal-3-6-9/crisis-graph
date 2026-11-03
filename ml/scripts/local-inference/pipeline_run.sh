#!/usr/bin/env bash
# The real CrisisGraph pipeline with local llama.cpp triage, end to end.
# Starts a fresh llama-server (so the first report is a cold request), the
# six roles from the release build, then a low-rate surge run below
# capacity. Samples memory, power and llama.cpp timings while it runs.
# usage: ml/scripts/local-inference/pipeline_run.sh <model.gguf> <adapter.gguf> <out-dir> <run-id> [steps]
set -u
M=$1; A=$2; OUT=$3; RUN=$4; STEPS=${5:-0.02222:600}
mkdir -p "$OUT"
ROOT=$(pwd); BIN=$ROOT/target/release
PIDS=()

cleanup() {
  for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null; done
  sleep 3
}
trap cleanup EXIT

wait_http() {   # wait_http <url> <seconds>
  for _ in $(seq 1 "$2"); do curl -sf "$1" >/dev/null && return 0; sleep 1; done
  echo "timeout waiting for $1"; return 1
}

echo "power: adapter=$(cat /sys/class/power_supply/ADP1/online) profile=$(powerprofilesctl get)" | tee "$OUT/env.txt"
free -m >> "$OUT/env.txt"
ps -eo rss,comm --sort=-rss | head -10 >> "$OUT/env.txt"

# The default configuration (iGPU via Vulkan), as in Findings 4 and 5.
llama-server --model "$M" --lora "$A" --alias local --host 127.0.0.1 --port 8000 \
  --ctx-size 4096 --parallel 1 --threads 8 --threads-batch 8 > "$OUT/llama-server.log" 2>&1 &
PIDS+=($!)
wait_http localhost:8000/health 180 || exit 1

for spec in relay:9104 api:3100 triage:9101 graph:9103 matcher:9102 driver:9105; do
  role=${spec%%:*}
  "$BIN/crisisgraph" --role "$role" > "$OUT/role-$role.log" 2>&1 &
  PIDS+=($!)
done
for port in 9104 3100 9101 9103 9102 9105; do wait_http "localhost:$port/readyz" 180 || exit 1; done
echo "all roles ready $(date -Is)" | tee -a "$OUT/env.txt"

# Memory and power every 5 s while the run lasts.
( while true; do
    echo "$(date +%s) $(free -m | awk '/Mem:/{print $3, $7}') $(cat /sys/class/power_supply/ADP1/online)"
    sleep 5
  done ) > "$OUT/memory.tsv" &
PIDS+=($!)

"$BIN/surge" run --run-id "$RUN" --seed 2026 --steps "$STEPS" --neighbours 0.1 --repeats 0.1 \
  --no-tag --extraction "local Qwen3-4B + SFT-V4 (llama.cpp, iGPU)" \
  --metrics localhost:9101,localhost:9102 --out "$OUT" \
  --notes "Local llama.cpp triage on the laptop (default iGPU configuration, one slot), fresh server so the first report is cold, INFERENCE_TIMEOUT_SECS from .env. Low rate, below capacity." \
  2>&1 | tee "$OUT/surge.log"

echo "power at end: adapter=$(cat /sys/class/power_supply/ADP1/online)" | tee -a "$OUT/env.txt"
