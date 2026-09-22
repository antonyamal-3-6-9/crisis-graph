# Triage inference integration

CrisisGraph now consumes the same extraction contract used by the Qwen3-4B V3
evaluation: prompt V2.1.1 and schema `triage-extraction-v2`. The model produces
an unverified candidate report. Nullable or uncertain facts force human review;
they are never replaced with guessed dispatch values.

V3 is the selected **experimental integration candidate**, not a promoted
production model. On the exposed 120-case suite it was the strongest aggregate
adapter (94.0% critical-field accuracy, 98/120 exact matches, and 60/68 review
recall), but it failed the predeclared clear-case gate at 18/20. The runtime
integration tests the system boundary around that known model, not a claim that
the model itself is safe or generalizes to real Kerala emergency traffic.

## Safety boundary

The language model may extract a location, headcount, requested asset, and
candidate road hazard. It does not select a shelter, calculate a route, mutate
the operational hazard overlay, certify road safety, or authorize dispatch.

The runtime fails closed when:

- the inference endpoint is unavailable or times out;
- the OpenAI-compatible response is malformed;
- output violates the V2 schema or typed value bounds;
- a dispatch-critical scalar is null;
- any reported hazard is incomplete; or
- the model or deterministic guard requests human review.

There is intentionally no heuristic fallback that invents a default headcount,
vehicle, or hazard. Inference failure becomes `TriageExtractionError`, later
stages do not reserve resources or calculate a route, and the final status is
`EscalateHumanDispatcher`.

## Running the evaluated V3 adapter with llama.cpp

The imported weights and converted adapter are ignored local artifacts:

```text
ml/artifacts/qwen3-4b-triage-sft-v3/adapter/
ml/artifacts/qwen3-4b-triage-sft-v3/adapter-f16.gguf
```

Start `llama-server` with the same conservative single-slot settings used for
the frozen CPU evaluation. Replace the base-model placeholder with the local
Qwen3-4B-Instruct-2507 Q4_K_M GGUF path:

```bash
llama-server \
  --model /path/to/Qwen3-4B-Instruct-2507-Q4_K_M.gguf \
  --lora ml/artifacts/qwen3-4b-triage-sft-v3/adapter-f16.gguf \
  --alias Qwen/Qwen3-4B-Instruct-2507 \
  --host 127.0.0.1 --port 8000 \
  --ctx-size 2048 --parallel 1 \
  --threads 8 --threads-batch 8
```

Then configure the Rust process:

```bash
export INFERENCE_BACKEND=llama-cpp
export INFERENCE_BASE_URL=http://127.0.0.1:8000/v1
export INFERENCE_MODEL=Qwen/Qwen3-4B-Instruct-2507
export INFERENCE_TIMEOUT_SECS=60
export TRIAGE_ADAPTER_ID=qwen3-4b-triage-sft-v3
export TRIAGE_ADAPTER_SHA256=bcec4e21b232174b89dcd0dd2a800212136488ed32d962e810babae9efa18039
```

`TRIAGE_ADAPTER_ID` and `TRIAGE_ADAPTER_SHA256` are audit metadata attached to
the candidate report. They do not cause llama.cpp to load the adapter. The
server command is responsible for loading the actual weights, so deployment
automation must keep the command and metadata consistent.

Verify the extraction boundary without connecting to Neo4j or Redis:

```bash
cargo run --bin triage_probe -- \
  "Send an ambulance for 3 people at Aluva Railway Station."
```

The probe prints the typed candidate and inference provenance. It performs no
allocation, hazard mutation, routing, or dispatch.

The legacy `VLLM_BASE_URL` and `VLLM_MODEL` variables remain compatibility
aliases. New deployments should use the backend-neutral `INFERENCE_*` names.

The checked-in `.env.example` documents these variables without including
credentials or machine-specific model paths. Local `.env` values and model
artifacts remain ignored.

## vLLM-compatible deployment

Set `INFERENCE_BACKEND=vllm` and expose the selected base/LoRA combination under
the model name supplied by `INFERENCE_MODEL`. CrisisGraph will send vLLM's
`structured_outputs.json` constraint instead of llama.cpp's
`response_format.json_schema` constraint. Adapter loading and model aliases are
owned by the inference server, not the Rust application.

## Runtime provenance

Every successful `TriageReport` includes:

- inference backend and served model name;
- configured adapter ID and SHA-256;
- prompt and schema versions; and
- request latency in milliseconds.

The REST dispatch response includes the full candidate triage object so a
dispatcher or audit pipeline can inspect the extracted facts, uncertainty, and
inference provenance. This metadata describes the configured request path; it
does not cryptographically attest what weights a remote server actually loaded.

## Local integration verification

On 2026-09-22, the Rust probe was exercised against llama.cpp with the converted
V3 adapter and Qwen3-4B Q4_K_M base. A complete ambulance request returned the
expected location, headcount, asset, empty hazard list, and no review flag. A
second report explicitly stating that no vehicle was requested returned
`required_asset=null` and `needs_human_review=true`. Both responses included the
configured V3 adapter hash and V2.1.1/V2 contract provenance. Observed CPU
latencies were approximately 16.8 and 10.7 seconds; these two calls are
integration checks, not a performance benchmark.

The next gate is a controlled full-pipeline test with Neo4j and Redis. A
complete request must reach `RoutedVerified`; an incomplete request must reach
`EscalateHumanDispatcher` before any resource reservation or route calculation.
Because the complete case mutates operational allocation state, the test must
snapshot or isolate state and restore it deliberately rather than blindly
resetting a shared database.
