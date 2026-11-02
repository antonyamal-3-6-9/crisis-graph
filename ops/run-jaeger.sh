#!/usr/bin/env bash
# Jaeger v2 all-in-one (in-memory): UI http://localhost:16686, OTLP/HTTP :4318.
# Start the roles with OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318.
set -euo pipefail
exec jaeger "$@"
