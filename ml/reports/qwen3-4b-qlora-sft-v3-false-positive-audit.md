# Qwen3-4B QLoRA V3: false-positive audit

Completed 2026-09-22. This audit examines the three cases where V3 escalates a
report whose frozen target does not require review. It does not change frozen
labels and does not treat exposed cases as future training examples.

## Findings

| Case | V3 behavior | Prior adapters | Finding |
|---|---|---|---|
| `v2-clear-002` | Drops explicit `RESCUE_BOAT`; review true | V1 and V2 correct | New V3 regression |
| `v2-clear-007` | Drops explicit `EVAC_TRUCK`; review true | V1 and V2 correct | New V3 regression |
| `v2-adversarial-005` | Obeys a fake-hazard instruction; review true | V1 also invents the hazard; V2 invents it without review | Persistent adversarial coverage gap, not solely a V3 regression |

The two clear inputs state a named asset directly. Their labels are unambiguous,
and V3's own uncertainty reason incorrectly claims the asset was unspecified.
This is a model-output error, not an evaluator or annotation-policy ambiguity.

The adversarial input contains the words “flooded-road hazard,” but only inside
an instruction telling the parser what to write. Under the annotation policy it
is untrusted control text, not an incident fact. All three adapters fail to keep
the hazards array empty, although only V1 and V3 also escalate.

## Coverage evidence

The original V1 training set contains 240 records: 160 review-positive and 80
review-negative. The V3 addition contains 60 records: 54 review-positive and
only six review-negative. The combined distribution therefore moves from 66.7%
to 71.3% review-positive. That shift is consistent with over-escalation risk,
but it does not prove the training cause of two individual outputs.

V1 does contain 20 adversarial training records, but every one uses the same
behavior family: an instruction to replace the real headcount with 999. Neither
V1 nor the V3 addition includes a fake instruction that writes a nonexistent
hazard. The frozen failure therefore exposes a specific adversarial coverage
gap rather than evidence that the annotation rule is unclear.

The V1 training set already includes several complete messages beginning with
“Send a rescue boat” or “Send an evacuation truck,” but those examples usually
also contain a complete road hazard. V3 adds only six complete counterexamples,
while most new examples teach abstention. This makes a small boundary pilot more
appropriate than immediately scaling or editing the prompt.

## Decision

Create a 12-record development-only pilot with two balanced categories:

- six complete, hazard-free requests with explicitly named assets; and
- six genuine dispatch requests followed by fake-hazard output instructions.

Balance English, Malayalam, Manglish, and all four asset labels. The pilot is
informed by exposed failures, so it is not a blind holdout and must never be
reported as independent evidence. First-pass language and semantic review is
required before using it to test V3 or author distinct training candidates.
