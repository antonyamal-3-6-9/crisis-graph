#!/usr/bin/env bash
# Prometheus for the local pipeline: http://localhost:9090 (data in .ops-data/).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p "$ROOT/.ops-data/prometheus"
# Packaged builds (e.g. Arch) read the web UI from web/ui/ relative to the
# working directory, as their own service does; every path below is absolute.
cd "${PROMETHEUS_HOME:-/usr/share/prometheus}"
exec prometheus \
  --config.file="$ROOT/ops/prometheus/prometheus.yml" \
  --storage.tsdb.path="$ROOT/.ops-data/prometheus" \
  --web.listen-address=127.0.0.1:9090 \
  --web.enable-lifecycle
