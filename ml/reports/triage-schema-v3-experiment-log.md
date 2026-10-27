# Triage schema V3: experiment log

Running record of the ML track (MLx) for triage extraction schema V3. The final
results and decision go in `triage-schema-v3-evaluation.md`; this log keeps the
story of how we got there, including what failed. Newest entries at the end.

## Goal

Extend triage extraction with `victim_profile` (who is reporting, whether they
are on scene, medical conditions, vulnerable groups, water level and trend, a
raise-only suggested urgency) so that urgency can be **derived deterministically
in Rust** and remote callers are not routed to their own phone position
(finding F-1). The model extracts facts; code derives judgements.

Pass criteria were fixed before any results were seen (core implementation
plan, ML track): critical under-triage <= 1 case; recall >= 90% for each
critical condition (UNCONSCIOUS, BREATHING_DIFFICULTY, CARDIAC, BLEEDING);
remote-caller miss rate <= 10%; V2 exact match no more than 2 cases below the
98/120 baseline and V2 review recall not below 60/68; p95 latency increase
<= 30%.

## 1. Contract (2026-10-25)

- `schemas/triage-extraction-v3.json`: V2 unchanged plus `victim_profile`.
- `ml/src/crisisgraph_ml/urgency.py` + `ml/data/urgency-rules-v1.jsonl`:
  deterministic urgency rules with hand-written expected results (written
  before the code, so the tests check the code rather than echo it). The Rust
  implementation (M3b) must pass the same file.
- `schemas/urgency-result-v1.json`: the derived result.
- Owner decisions during review:
  - UNKNOWN results get `UNKNOWN_REVIEW_REQUIRED` and a review priority (HIGH
    for low-mobility groups or rising water, MEDIUM for other groups, LOW
    otherwise).
  - New relation value `UNDER_REVIEW` (text points to the reporter's family or
    home but leaves open whether the reporter is a victim); resolved after
    extraction by comparing caller position with the described location.
  - Half-open water-level bands in metres and feet; elapsed time is not a
    hazard duration; CHRONIC_CARE vs acute episodes reconciled.
  - A low-mobility group with water >= WAIST and RISING is CRITICAL.
  - Gold urgency: a critical symptom resolved within 24 h is URGENT.
  - People on a roof with no stated flooding get `water_level: null`.

## 2. Data

| Set | Cases | Use |
|---|---|---|
| `triage-eval-v3-dev-candidates.jsonl` | 42 | prompt development only |
| `triage-eval-v3.0.1.jsonl` (frozen) | 120 | final reporting only |
| `triage-eval-v3-v2-carryover.jsonl` | 120 | V2 inputs with V3 labels: V2 non-regression, invented facts |

All three owner-reviewed. The test set was re-frozen once (v3.0.0 -> v3.0.1)
the same day, before any model run, for the roof rule. Tests fail if the frozen
file, its generator or the contract files change.

Separation safeguards: no input text or location shared between dev and test;
no 5-word sequence shared between the prompt and the test set (this test
caught two leaks in my own prompt text, both fixed). Some scenario types
(snakebite, labour, insulin) appear in both sets with different wording.

## 3. Evaluator

`crisisgraph-triage-eval-v3`: serial, checkpointed and resumable; reuses the V2
scoring and adds per-condition recall with Wilson intervals, urgency from the
**model's** profile through the rule table compared with gold, critical
under-triage, a rule-only and an oracle (gold labels through the rules)
ceiling, remote-caller miss rate and invented facts. Unusable outputs count as
empty profiles, so failures never hide from recall. It refuses to run unless
`--adapter` matches what the llama.cpp server actually applies (added after a
run nearly started against the wrong server).

## 4. Dev iterations (42 dev cases; small counts, read as patterns)

Serving: Qwen3-4B-Instruct-2507 Q4_K_M, llama.cpp build 11429 (the V2 baseline
used build 10809), CPU, one slot, context 4096 (V2 used 2048; the V3 prompt is
about twice as long), temperature 0, up to 768 output tokens.

| Run | Model | Prompt / order | V2 exact | Spurious reviews | Critical under-triage | p95 |
|---|---|---|---|---|---|---|
| 1 | base | v3.0 | 17 | 15 | 1 (035) | 22.8 s |
| 2 | base | v3.1 | 16 | 16 | 3 (011, 017, 035) | 23.6 s |
| 3 | base | v3.1, profile last | 23 | 7 | 3 | 25.8 s |
| 4 | SFT-V3 adapter | v3.1, profile last | **31** | **3** | **1 (011)** | 22.5 s |
| 5 | SFT-V3 adapter | **v3.2** (frozen) | **33** | **2** | 2 (011, 017) | 21.6 s |

**Run 1 findings.** The model asked for human review because profile facts
were unknown ("no water level stated") in 15 cases; obeyed a prompt injection
that told it to empty the medical fields (035, the one critical
under-triage); confused relatives with third parties; inferred ELDERLY from
"father"; put "kids are scared" and "food is over" in `other_conditions`;
turned water depth into road hazards; left Malayalam case endings on
locations. One cause was my own prompt bug ("cannot walk" listed under
DISABLED).

**Run 2 (prompt v3.1, a rule per cause).** No improvement on the base model:
stronger wording did not change the review behaviour or the injection.

**Run 3 (generation order).** The schema generated `victim_profile` before
`needs_human_review`, so the model wrote the review decision right after
listing profile unknowns. Moving the profile last (same meaning, different
grammar order) halved spurious reviews and raised V2 exact match by 7. This
was the most effective single change, and it was structural, not wording.

**Run 4 (adapter).** The SFT-V3 adapter, trained only on V2 outputs, handled
the new fields well and kept its V2 strength: all critical conditions found,
the injection resisted, 3 spurious reviews. Remaining: 011 (snakebite) stays
URGENT because `other_conditions` only reaches URGENT by rule and the model
gave no suggestion; "kidappilaanu" (bedridden) and "food is over" still land
in `other_conditions`; Malayalam case endings partly kept; relations about 83%.

**Owner decision: latency.** The latency gate is waived for local CPU
serving. CPU numbers measure this machine; the real latency budget belongs to
the GPU/vLLM deployment and is measured there.

**Decisions taken from the dev runs.** Adopt profile-last as the schema order
(recorded as a contract revision in the v3.0.1 manifest; labels and data
unchanged). Prompt v3.2 = v3.1 with the examples in the new order and at most
three uncertainty reasons. Prompt iteration stops after one dev check of v3.2,
because further rounds would start fitting these 42 cases.

**Run 5 (v3.2 check) and prompt freeze.** Roughly level with v3.1: better on
V2 exact match, reviews and latency; 017 (bedridden written into
`other_conditions`) is under-triaged again because this time the model's
suggestion said URGENT instead of CRITICAL. Same known weakness, different
luck on 42 cases, so not treated as a regression. **Prompt frozen at v3.2**,
as agreed before the run.

## 5. Final runs (started 2026-10-25)

Prompt v3.2 and the profile-last schema, everything else fixed. No changes
after the runs start.

**Interruption.** Run 1 stopped at case 73/120 (19:00) when the Claude Code
session and the llama.cpp server stopped. After restarting the server with the
same adapter, it resumed from the checkpoint at case 74. The run identity
(model, adapter file and the adapters the server reports, prompt, schema,
dataset, max tokens) matched, and decoding is at temperature 0, so the resumed
run is equivalent to an uninterrupted one; only latency differs for the first
resumed case (prompt cache cold).

1. Adapter on the frozen test set
2. Adapter on the V2 carry-over
3. Base model on the frozen test set
4. Base model on the V2 carry-over

Then `check_gates()` (latency waived), the evaluation report, and the owner's
decision: adapter into M3b, base model, or SFT-V4 from the measured failures.

## Lessons so far

- **Output order is part of the prompt.** With grammar-constrained decoding, a
  small model is steered more by the order it writes fields in than by
  instructions about them.
- **Prompt wording has a ceiling on a 4B base model.** Three rounds fixed
  little; the adapter fixed most of it. Fine-tuning taught behaviour that
  instructions could not.
- **A rule table needs a measured ceiling.** The oracle run (gold labels
  through the rules) shows which under-triage is a rules gap rather than a
  model error; snakebite is one.
- **Test your own test hygiene.** The leak test found overlaps I had written
  myself, and a stale bytecode cache (coarse timestamps on NTFS) briefly made
  a passing test look failing.

### Run 1 result: SFT-V3 adapter, frozen test set v3.0.1 (120 cases)

| Criterion | Result | Gate |
|---|---|---|
| Critical under-triage | **9 / 56** gold-CRITICAL | <= 1 — **fail** |
| Recall UNCONSCIOUS / CARDIAC / BLEEDING | 13/13 · 10/10 · 9/10 | >= 90% — pass |
| Recall BREATHING_DIFFICULTY | **8/10** | >= 90% — **fail** |
| Remote-caller miss | 1/12 (8%) | <= 10% — pass |
| p95 latency | 21.8 s | waived (CPU) |

V2 fields on these new messages: 83/120 exact, review recall 78/79, 9 spurious
reviews. Invented profile facts in 33 cases (e.g. CHILD from "son").
INJURY recall 3/15 (mostly dropped next to a more serious condition);
ELDERLY 14/21.

**The 9 critical under-triage cases split in two:**
- **Rule-table gaps (4: 053 snakebite, 055 swept away, 057 hypothermia,
  085 family on a roof):** the oracle (gold labels through the rules) also
  misses them. They depended on `suggested_urgency`, which **never raised a
  result** (0 raises in 120; the model only says CRITICAL where the rules
  already do). Finding: rules plus a raise-only suggestion do not cover
  life-threatening situations outside the categories.
- **Extraction misses (5):** 028 "chora nilkunnilla" (Manglish bleeding,
  remote caller), 065 "koodikondirikkunnu" (rising, would trigger the
  low-mobility rule), 067 "ground floor completely under water" (ABOVE_HEAD,
  named in the prompt), 068 "veedu muzhuvan vellathil" + terrace (ROOF), 070
  bedridden written into `other_conditions`.

The two BREATHING misses did not change urgency (both still CRITICAL through
another critical condition).

### Run 2 result: SFT-V3 adapter, V2 carry-over (120 cases)

| V2 regression | V2 baseline (SFT-V3, prompt v2.1.1, V2 schema) | V3 (SFT-V3, prompt v3.2) | Gate |
|---|---|---|---|
| Exact match | 98/120 | **91/120** | >= 96 — **fail** |
| Review recall | 60/68 | **58/68** | >= 60 — **fail** |
| Spurious reviews | 3 | 2 | — |

Adding the profile cost the adapter 7 exact matches and 2 missed reviews on
the V2 fields. The adapter was trained on V2 outputs only, so the longer
output it never learned costs some V2 accuracy. Invented profile facts in 13
of 120 plain V2 messages, mostly a guessed reporter relation (9) or on-scene
value (4). The 5 gold-CRITICAL carry-over cases all came out CRITICAL, but
**only through invented facts**: "critical patient" became UNCONSCIOUS (2) or
CARDIAC (1), and "on the roof" became `water_level: ROOF` (2), the inference
the owner's strict rule excludes. Right urgency for the wrong reason; those
facts would also reach a dispatcher as if stated.

**Candidate B overall: fails** (critical under-triage, BREATHING recall, V2
exact match, V2 review recall); passes UNCONSCIOUS, CARDIAC and BLEEDING
recall and the remote-caller miss rate. Latency waived.

### Runs 3–4 result: base model (no adapter)

Both completed without errors (the Claude Code session was interrupted again
afterwards; no run was affected). Critical under-triage 8/56 (fails); recall
UNCONSCIOUS 13/13, BREATHING 10/10, CARDIAC 10/10, BLEEDING 9/10 (all pass);
remote-caller miss 1/12 (pass); V2 set 71/120 exact with 46/68 review recall
(fails badly). Slightly better than the adapter on the profile, far worse on
V2. **Neither candidate passes.** Full comparison, case table and decision
options: `triage-schema-v3-evaluation.md`.

**Key finding:** the oracle (gold labels through the rules) already
under-triages 4 gold-CRITICAL cases (snakebite, swept away, hypothermia, roof
without stated flood), so the gate of <= 1 cannot be met by any extraction
model under urgency-rules-v1. The rules need a decision before more model work.

### Rules v2 and re-score (2026-10-26)

Owner decision: any `other_conditions` entry is CRITICAL with immediate human
review. Implemented as `urgency-rules-v2` next to an unchanged v1; all four
runs re-scored from saved outputs. Adapter critical under-triage 9 → 5, base
8 → 7; over-triage up by about 4 each. Remaining adapter misses: 085 (roof
rule) and four extraction gaps (028, 065, 067, 068). One fix (070) was an
accident of the model's own mistake. Details in the evaluation addendum.

### Frontier reference check: Gemini (2026-10-26, dev set only, partial)

Owner request: compare a frontier model against Qwen3-4B on the same prompt
(v3.2) and schema, through Gemini's OpenAI-compatible API (`--backend openai`).
`gemini-2.5-flash` is retired for new keys, `gemini-3.8-flash` was overloaded
(503), so `gemini-3.5-flash` was used; the quota error named the model
`gemini-3.6-flash`, so the alias may be served by a newer model. The free tier
allows **20 requests per day per model**, so the run stopped after 14 of 42
dev cases (checkpoint kept). The frozen test set was not sent: free-tier
inputs may be used for Google's training, which would contaminate the
benchmark.

| First 14 dev cases | Gemini 3.5 Flash | Qwen3-4B + SFT-V3 |
|---|---|---|
| V2 fields exact | 14/14 | 9/14 |
| Urgency matches gold (rules v2) | 13/14 | 13/14 |
| Critical under-triage | 0 | 0 |
| Cases with invented facts | 0 | 5 |
| Reporter relation | 13/14 | 11/14 |
| Median latency | 8.9 s (API) | 19.1 s (CPU) |

Directional only (14 cases). Gemini's one miss was 005, the UNDER_REVIEW
case. **Owner decision: stay with Qwen3-4B**, a local model; the frontier
result shows the headroom a small model could approach with better training
data.

### Roof rule (owner decision, 2026-10-26)

People stranded on a roof or at height awaiting rescue get
`other_conditions: ["stranded on a roof"]`, a stated situation with no inferred
flood, which rules v2 makes CRITICAL with immediate review. Recorded in
`data/triage-annotation-guidelines-v3.1-changes.md` because the main guideline
is pinned by the v3.0.1 manifest; applies from the next dataset version.
v3.0.1 is not relabelled: case 085 stays a documented gap, since changing
labels after seeing results would move the goalposts.

## 6. SFT-V4 preparation (2026-10-26)

- **Spec approved** (`data/triage-sft-v4-spec.md`): option (a), train and serve
  with prompt v3.2; ~530 training records (300 converted SFT-V3 + ~230 new
  profile-rich), ~100 validation; new blind test set built first.
- **Blind test set v3.1.0 frozen** (120 cases, sha256 `cc67e434…`). Drafted by a
  zero-context agent from the guidelines only (no access to other datasets or
  the urgency rules); my coverage and separation checks pass. Building the
  separation check found my own bug (Malayalam words split into letters by
  `\w+`, also in the prompt leak test) and one near-copy sentence (case 052),
  reworded after confirming the set was not derived from v3.0.1 (similarity at
  chance level). A second zero-context frontier model reviewed it; the owner
  accepted its 15 corrections (urgency judgements, an area hazard, rising-water
  consistency, Malayalam/Manglish wording). Not domain-expert certified.
- **Owner decisions from that review:** suspected life-threatening other
  conditions count as present; water still coming in counts as RISING
  (guideline changes 3 and 4); **urgency-rules-v3** makes a low-mobility group
  or a pregnant woman in chest-deep or deeper water CRITICAL. Under v3 the
  rules never rate a v3.1.0 gold label lower (oracle ceiling 0; 15 accepted
  over-triage cases).

## 7. SFT-V4 training data (2026-10-26 → 27)

- **Conversion:** all 372 SFT-V3 records converted to V3 targets (messages and V2
  labels unchanged, tested). Owner review set the twelve "We have N people who
  need …" rows to an unknown reporter (coordinator phrasing, guideline change 5)
  and aligned t273 with test case 105; only 8 converted records keep profile
  facts, so the new set grows to ~290 training records to keep ~50% profile-rich.
- **Batch 1 (50, water T1–T2):** AI first pass caught coordinator phrasing, a
  roof-convention slip, a river-gauge reading treated as depth, and ambiguous
  Manglish headcounts. Owner review then set about-2-ft-with-healthy-adults
  to gold STABLE (rule over-triage accepted), "just under a foot" to KNEE, a
  bus-shelter roof with stated depth to ABOVE_HEAD, and confirmed "ground floor
  submerged → CRITICAL" on its own merits.
- **urgency-rules-v4:** a child in chest-deep or deeper water is CRITICAL (from
  the batch 1 review). No change on v3.1.0: rules still never below gold.
- **Batch 2 (60, medical phrasing T3 and water trend T4):** 48 Malayalam /
  Manglish and 12 English records. Each critical condition gets six or seven
  native-language positives next to near misses (dizzy but talking, known
  asthma or heart disease with no episode, a cut that stopped bleeding,
  symptoms resolved earlier today). Trend records cover rising, falling,
  unchanged and hypothetical wording. AI first pass: the text and profile labels
  held up; it questioned bleeding golds and the bedridden case. Owner review:
  bleeding is gold CRITICAL only when heavy, not stopping or postpartum (030,
  031 to URGENT); a bedridden person in bed-height water is CRITICAL (045);
  "if the water keeps rising" is RISING (change 8); labour aligned to the
  frozen v3.1.0 gold, CRITICAL (change 7); an inner Malayalam genitive stays
  in the location.
- **Batch 3 (50, groups T5–T6 and injury T7):** 27 English, 12 Manglish, 11
  Malayalam. Groups versus medical fields (bedridden, wheelchair, blind,
  "cannot walk without help", dialysis / insulin / oxygen as CHRONIC_CARE);
  paired bare relations against stated ages ("my father" / "my 78-year-old
  father"); INJURY kept beside critical conditions and dropped under negation.
  AI first pass: one fix (a warden's position is not stated). Owner review:
  pregnancy in ankle-deep water is gold STABLE, like an infant at the same
  depth; autism is DISABLED (change 10); no comma inserted into a location.
