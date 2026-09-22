# Qwen3-4B QLoRA SFT V1 frozen-regression evaluation

Date: 2026-09-16

## Decision summary

The SFT V1 adapter materially improved general extraction behavior on the
existing 120-case frozen suite, but it did **not** pass the safety gate.
Critical-field accuracy increased from 85.2% to 92.7%, exact match increased
from 55.0% to 77.5%, and raw human-review recall increased from 66.2% to 82.4%.

The important counter-result is that effective fail-closed review recall fell
from 86.8% to 82.4%. The base model often emitted a `null` that the
deterministic policy converted into an escalation. The tuned model is better at
completing the contract, but on twelve required-review cases it confidently
selected a concrete value and emitted `needs_human_review=false`, leaving no
missing field for that guard to catch. Contradiction handling is the main
regression.

Therefore, this adapter is a successful learning experiment and a promising
candidate extractor, but it must not replace the deterministic safety and
human-escalation layers. It is not ready to be promoted as the default triage
model.

## Experiment controls

The comparison held the following inputs constant:

- Frozen dataset: `data/triage-eval-v2.0.0.jsonl` (120 cases)
- Prompt: `prompts/triage-extraction-v2.1.1.txt`
- Schema: `schemas/triage-extraction-v2.json`
- Base GGUF: Qwen3-4B-Instruct-2507 Q4_K_M
- llama.cpp: build 10809, commit `5266f24da7`
- CPU threads: 8
- Context: 2,048 tokens
- Parallel slots / evaluation concurrency: 1 / 1
- Temperature / maximum output: 0.0 / 512 tokens

The experimental change was the separately loaded SFT V1 LoRA adapter. The
PEFT adapter was converted to a 64 MB F16 GGUF using the converter from the
same llama.cpp commit. The GGUF contains 504 tensors and has SHA-256
`bf121077b46ca6858f02c1de1a32e2edc3d52dd934486ab182844ed15d581b8a`.

The complete ignored result is
`runs/qwen3-4b-q4-k-m-lora-sft-v1-c1-prompt-v2.1.1-eval-v2.0.0.json`
(SHA-256
`acd5717da8fbd993291bef2d1dc4e3f44b705f69281de1ad9b298003db0366b0`).

## Overall comparison

| Metric | Base Q4_K_M | SFT V1 LoRA | Change |
|---|---:|---:|---:|
| Schema validity | 100.0% | 100.0% | 0.0 pp |
| Critical-field accuracy | 85.2% | 92.7% | +7.5 pp |
| Exact match | 55.0% | 77.5% | +22.5 pp |
| Raw human-review recall | 66.2% (45/68) | 82.4% (56/68) | +16.2 pp |
| Raw human-review precision | 86.5% | 98.2% | +11.7 pp |
| Effective fail-closed recall | 86.8% (59/68) | 82.4% (56/68) | -4.4 pp |
| Unsupported-fact case rate | 16.7% (20/120) | 17.5% (21/120) | +0.8 pp |
| Inference errors | 0 | 0 | 0 |
| Mean latency | 15.45 s | 11.59 s | -25.0% |
| P50 latency | 14.12 s | 10.33 s | -26.8% |
| P95 latency | 22.35 s | 16.19 s | -27.6% |

Latency is observational rather than a controlled performance benchmark. The
tuned model generally produced shorter, more regular responses, but machine
load and thermal state can also affect separate CPU runs.

## Per-field comparison

| Field | Base | SFT V1 LoRA | Change |
|---|---:|---:|---:|
| Victim location | 83.3% | 86.7% | +3.4 pp |
| Headcount | 87.5% | 95.8% | +8.3 pp |
| Required asset | 86.7% | 96.7% | +10.0 pp |
| Hazards | 93.3% | 95.0% | +1.7 pp |
| Review decision | 75.0% | 89.2% | +14.2 pp |

## Category findings

| Category | Critical fields: base -> tuned | Exact: base -> tuned | Raw review recall: base -> tuned |
|---|---:|---:|---:|
| Clear | 98.0% -> 100.0% | 95.0% -> 100.0% | N/A |
| Missing information | 88.0% -> 95.0% | 50.0% -> 85.0% | 65.0% -> 90.0% |
| Contradictory | 82.7% -> 78.7% | 40.0% -> 40.0% | 80.0% -> 53.3% |
| Hazard | 86.0% -> 94.0% | 55.0% -> 80.0% | 27.3% -> 90.9% |
| Malayalam | 72.0% -> 89.3% | 26.7% -> 60.0% | 57.1% -> 100.0% |
| Manglish | 76.0% -> 92.0% | 40.0% -> 80.0% | 85.7% -> 71.4% |
| Adversarial | 77.5% -> 92.5% | 37.5% -> 75.0% | 0.0% -> 100.0% |
| Irrelevant | 100.0% -> 100.0% | 100.0% -> 100.0% | 100.0% -> 100.0% |

The adapter produced large gains on hazards, Malayalam, missing information,
and adversarial inputs. It regressed on contradictions and Manglish review
recall. The contradictory category is safety-critical: seven of its fifteen
cases incorrectly bypassed review.

## Effective false negatives

The tuned model missed escalation for these twelve required-review cases:

- `v2-missing-information-011`
- `v2-missing-information-012`
- `v2-contradictory-002`
- `v2-contradictory-005`
- `v2-contradictory-007`
- `v2-contradictory-008`
- `v2-contradictory-010`
- `v2-contradictory-011`
- `v2-contradictory-015`
- `v2-hazard-019`
- `v2-multilingual-manglish-007`
- `v2-multilingual-manglish-008`

Six were also effective misses in the base run. Six are new tuned-model misses,
five of which are contradictions. The recurring failure is confident semantic
selection: the model chooses one location/count/asset from conflicting text,
or converts an underspecified phrase into a concrete value, then assigns high
confidence and disables review.

## Interpretation and next gate

The experiment demonstrates that a small, reviewed SFT dataset can teach a 4B
model the output contract and improve domain extraction substantially. It also
demonstrates why validation loss and teacher-forced token accuracy cannot be
used as the deployment decision: the adapter achieved near-perfect validation
token accuracy while still missing twelve safety-relevant escalations during
free generation.

Before another GPU run, inspect the contradiction coverage in the training
set and add targeted, independently authored examples that vary wording and
conflict position. Do not tune directly to these twelve benchmark records. Run
the revised adapter against this suite as a regression check, then use a new,
independently reviewed blind holdout for the final generalization claim. The
promotion gate remains at least 99% review recall, with no critical-category
regression and deterministic fail-closed routing unchanged.
