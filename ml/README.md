# CrisisGraph ML Workspace

This directory contains offline model-engineering work for CrisisGraph. It does
not calculate routes, certify road safety, reserve resources, or authorize a
dispatch. Those responsibilities remain in the deterministic Rust system.

## Scope

- Versioned prompts and structured-output contracts
- Curated SOS triage evaluation data
- Accuracy, safety, latency, and concurrency evaluations
- Quantization comparisons
- Fine-tuning data preparation and training experiments
- Reproducible model and experiment configuration

The serving runtime is intentionally replaceable. Local CPU development may use
`llama.cpp`, while GPU deployments may use vLLM. Evaluations should exercise the
same logical model contract through either OpenAI-compatible endpoint.

Local model weights are stored under the ignored `model-cache/` directory on
the project drive by setting `HF_HOME="$PWD/model-cache/huggingface"` before
starting llama.cpp. This avoids filling the system-drive Hugging Face cache.

## Layout

```text
ml/
├── configs/                 # Model and experiment configuration
├── data/                    # Dataset policy and curated, non-operational fixtures
├── evals/triage/            # Triage evaluation cases and runners
├── src/crisisgraph_ml/      # Shared Python evaluation/training utilities
└── pyproject.toml           # Isolated Python environment
```

Generated datasets, downloaded model weights, checkpoints, experiment runs, and
operational exports are ignored. Do not put real personally identifiable SOS
messages in this directory.

## Initial milestone

Before fine-tuning, establish a Qwen3-4B baseline against a reviewed triage test
set. The frozen benchmark is `data/triage-eval-v2.0.0.jsonl`; never use it for
training or prompt tuning. Report results overall and by category and language.
Measure at least:

- JSON Schema validity
- Critical-field exact match
- Asset classification accuracy
- Human-review recall
- Unsupported-fact rate
- P50/P95 time to first token and end-to-end latency
- Throughput at concurrency 1, 2, and 4

The evaluation contract is `../schemas/triage-extraction-v2.json`. It permits
explicit unknown values so the model is never forced to invent a missing
location, headcount, asset, or hazard duration. The Rust runtime still uses V1
until its typed pipeline contract is migrated and verified separately.

Tracked experiment conclusions live in `reports/`. Full raw run artifacts stay
under the ignored `runs/` directory.

## Environment

```bash
cd ml
uv sync --extra dev
```

Start the Gemma development server while keeping its cache on the project
drive:

```bash
HF_HOME="$PWD/model-cache/huggingface" llama-server \
  -hf ggml-org/gemma-3-4b-it-GGUF:Q4_K_M \
  --alias google/gemma-3-4b-it \
  --host 127.0.0.1 --port 8000 \
  --threads 8 --threads-batch 8 --parallel 1 \
  --kv-unified-per-slot 2048 --cont-batching --metrics
```

With a compatible model server already running on port 8000:

```bash
uv run crisisgraph-triage-eval \
  --backend llama-cpp \
  --model google/gemma-3-4b-it \
  --prompt prompts/triage-extraction-v2.2.txt \
  --concurrency 1 \
  --output runs/gemma-3-4b-q4-k-m-c1-prompt-v2.2.json
```

Prompt versions remain separate files so an experiment changes one variable
without destroying its baseline. The current development comparison and known
failure cases are documented in `reports/prompt-v2.2-comparison.md`.

For cross-model evaluation, keep the prompt and contract frozen. The initial
Qwen3 4B versus Gemma 3 4B result is documented in
`reports/model-qwen3-4b-vs-gemma3-4b.md`.

The compact V2.1.1 review-rule experiment across both models is documented in
`reports/prompt-v2.1.1-cross-model-comparison.md`.

The Qwen3-8B comparison across V2.1, V2.1.1, and V2.2—including its required
non-thinking runtime configuration—is documented in
`reports/qwen3-8b-prompt-comparison.md`.

The first independently reviewed 120-case baseline is documented in
`reports/qwen3-4b-eval-v2.0.0-baseline.md`. It records overall, per-category,
and per-language results and the remaining fail-closed safety gap.

Training dependencies are intentionally not included yet. They should be added
only after the baseline evaluation identifies a fine-tuning requirement and the
target training environment is known.

The baseline identified a semantic-review gap, so a deliberately small SFT
pilot has been generated as separately reviewed train, validation, and dev
candidates. See `data/triage-sft-v1-review-guide.md`. The exporter refuses to
create chat-format training data from pending or rejected records. Its default
`independent` policy requires a completed second pass; the explicitly selected
`pilot-first-pass` policy accepts first-pass-reviewed synthetic data for this
controlled experiment.

The Colab-ready QLoRA workflow is in
`notebooks/crisisgraph-qwen3-4b-qlora.ipynb`. It verifies the exported dataset
hashes, defaults to a three-step smoke run, trains PEFT adapters over a 4-bit
NF4 Qwen3-4B base with TRL, and records adapter/run metadata for the later
frozen-benchmark comparison.
