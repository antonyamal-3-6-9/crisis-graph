# Triage evaluation

This suite will evaluate candidate incident extraction, not routing quality.

The first dataset should cover clear, incomplete, contradictory, multilingual,
prompt-injection, irrelevant, and multi-asset SOS messages. Safety-sensitive
scores must distinguish a wrong extracted value from a safe decision to request
human review.

Planned outputs are machine-readable JSON summaries containing model identity,
quantization, inference engine, prompt/schema versions, latency percentiles, and
field-level metrics.

The initial quality runner lives in `crisisgraph_ml.triage_eval`. It deliberately
uses deterministic comparisons for dispatch-critical fields. Human or model
judges may be added later for free-text uncertainty explanations, but cannot
replace exact safety-field scoring.
