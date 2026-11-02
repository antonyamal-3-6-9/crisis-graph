#!/usr/bin/env bash
# Grafana with the CrisisGraph dashboard provisioned: http://localhost:3300
# (port 3000 is the legacy server). Anonymous read-only access; admin/admin
# to edit. Data in .ops-data/.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p "$ROOT/.ops-data/grafana/logs" "$ROOT/.ops-data/grafana/plugins"
export CRISISGRAPH_ROOT="$ROOT"
export GF_PATHS_DATA="$ROOT/.ops-data/grafana"
export GF_PATHS_LOGS="$ROOT/.ops-data/grafana/logs"
export GF_PATHS_PLUGINS="$ROOT/.ops-data/grafana/plugins"
export GF_PATHS_PROVISIONING="$ROOT/ops/grafana/provisioning"
export GF_SERVER_HTTP_ADDR=127.0.0.1
export GF_SERVER_HTTP_PORT=3300
export GF_AUTH_ANONYMOUS_ENABLED=true
export GF_AUTH_ANONYMOUS_ORG_ROLE=Viewer
export GF_ANALYTICS_REPORTING_ENABLED=false
export GF_ANALYTICS_CHECK_FOR_UPDATES=false
exec grafana server --homepath "${GRAFANA_HOME:-/usr/share/grafana}"
