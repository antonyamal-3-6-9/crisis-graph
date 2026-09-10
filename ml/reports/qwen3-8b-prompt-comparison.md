# Qwen3 8B triage prompt comparison

Date: 2026-09-11

## Objective

Evaluate Prompt V2.1, V2.1.1, and V2.2 on an 8B model to determine whether
additional model capacity resolves the review-decision and extraction failures
observed with 4B models.

## Model and runtime

- Model: `Qwen/Qwen3-8B`
- Parameters reported by llama.cpp: 8.19B
- Quantization: Q4_K_M
- GGUF size: 5.02 GB
- Runtime: llama.cpp build 10809
- CPU threads: 8
- Context per slot: 2,048 tokens
- Concurrency: 1
- Reasoning: off
- Temperature: 0.0
- Maximum output: 512 tokens
- Schema: `triage-extraction-v2`
- Dataset: `triage-eval-v1.jsonl` (12 synthetic development cases)

The dataset influenced all three prompts. These results are development
evidence, not an unbiased test of generalization or production reliability.

## Why reasoning was disabled

Qwen3-8B is a hybrid thinking model. A preliminary request with its default
thinking mode generated all 512 permitted tokens and took 136.1 seconds because
thinking interacted poorly with the forced JSON grammar. The preliminary run
was cancelled and wrote no report.

All reported runs used llama.cpp `--reasoning off`. The first corrected request
completed in 84 tokens. Non-thinking mode is the appropriate comparison for a
short structured extraction task and is held constant across all prompts.

## Memory viability

Before loading, the machine had 8.9 GiB available RAM. With the model loaded it
retained approximately 4.0–4.2 GiB available. Swap settled around 0.7–0.9 GiB
and did not grow continuously during the runs. Qwen3-8B Q4 is viable at one
2,048-token slot on this 16 GB machine, but there is not enough demonstrated
headroom for concurrency testing while the desktop workload is active.

## 8B prompt results

| Metric | V2.1 | V2.1.1 | V2.2 |
|---|---:|---:|---:|
| Schema validity | 100.0% | 100.0% | 100.0% |
| Critical-field accuracy | 88.3% | 86.7% | **91.7%** |
| Critical exact match | 58.3% | 41.7% | **66.7%** |
| Human-review recall | 85.7% | **100.0%** | **100.0%** |
| Raw review-decision accuracy | 75.0% | 66.7% | **91.7%** |
| Cases with counted unsupported facts | 16.7% | **8.3%** | 16.7% |
| Counted unsupported facts | 2 | **1** | 2 |
| Mean service latency | 25.7 s | 25.8 s | 23.4 s |
| P50 service latency | 24.4 s | 24.5 s | 21.4 s |
| P95 service latency | 32.8 s | **32.2 s** | 32.5 s |

Latency differences are observational. The prompt runs were sequential rather
than randomized repeated trials, so warm state, prefix reuse, response length,
CPU activity, and memory pressure can affect them.

## Prompt behavior

### V2.1

V2.1 missed review for the missing-asset case and invented `AMBULANCE`. It also
selected a value from a contradictory headcount. It unnecessarily reviewed the
Malayalam and Manglish cases. The larger model improved review recall over the
4B V2.1 run but did not solve the decision boundary.

### V2.1.1

The stronger review rule eliminated review false negatives, but the model became
over-conservative. It requested review for four complete cases, misclassified a
rescue-boat request as an ambulance, and still chose a contradictory headcount.
Its 100% review recall therefore came with the worst review precision pattern
and lowest exact match of the three 8B runs.

### V2.2

V2.2 produced the best overall 8B result. It correctly classified every required
asset, caught every required review, and made only one wrong review decision.
Remaining failures were:

- selecting five instead of null for a contradictory headcount;
- a strict Malayalam location-normalization mismatch;
- inventing a `FLOODED` hazard from the prompt-injection phrase about saying a
  road is safe; and
- shortening `road near Bank Junction` to `Bank Junction`.

The prompt-injection failure is safety-relevant: the output escalated for review,
but it still introduced a hazard unsupported by the report.

## 4B versus 8B

| Prompt | Model | Critical accuracy | Exact match | Review recall | Review accuracy | Unsupported-case rate |
|---|---|---:|---:|---:|---:|---:|
| V2.1 | Qwen3 4B | 88.3% | 58.3% | 42.9% | 66.7% | **8.3%** |
| V2.1 | Qwen3 8B | 88.3% | 58.3% | **85.7%** | **75.0%** | 16.7% |
| V2.1.1 | Qwen3 4B | **88.3%** | **66.7%** | 85.7% | **83.3%** | **0.0%** |
| V2.1.1 | Qwen3 8B | 86.7% | 41.7% | **100.0%** | 66.7% | 8.3% |
| V2.2 | Qwen3 4B | 90.0% | 50.0% | 100.0% | 83.3% | 16.7% |
| V2.2 | Qwen3 8B | **91.7%** | **66.7%** | 100.0% | **91.7%** | 16.7% |

Increasing parameter count did not uniformly improve results. The 8B model
benefited strongly from V2.2's explicit structure and examples, but performed
worse than 4B with V2.1.1 on several aggregate metrics.

## Conclusion

Qwen3-8B plus Prompt V2.2 is the strongest raw model/prompt combination in the
current development experiments. It still does not meet the unsupported-fact
target and does not remove the need for deterministic review enforcement.

The experiment does not justify further tuning on these same 12 cases. A larger
untouched split is required before selecting 8B over 4B, because V2.2 was built
using failures observed on this development set. The final deployment decision
must also weigh its approximately 4 tokens/second CPU decode speed and higher
memory pressure against the measured accuracy gain.
