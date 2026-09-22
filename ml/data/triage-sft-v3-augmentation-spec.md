# CrisisGraph Triage SFT V3 augmentation specification

Status: design approved for candidate authoring; no V3 examples, exports, or
training artifacts exist yet.

## Experiment question

Can a small, targeted augmentation improve V1's abstention and human-review
behavior while preserving V1's hazard, multilingual, and clear-case behavior?

V3 is based on V1, not V2. It retains all reviewed V1 examples and adds new,
independently authored scenarios. It does not continue training the V1 adapter:
the V3 adapter must start from the same untouched Qwen3-4B base revision.

## Evidence motivating V3

On the frozen 120-case regression suite, V1 achieved 92.7% critical-field
accuracy, 77.5% exact match, 82.4% raw review recall, and 82.4% effective review
recall after deterministic null/schema checks. Its twelve effective missed
escalations were concentrated in missing/underspecified facts, contradictions,
one hazard, and Manglish contradictions.

V2 improved contradiction review recall from 53.3% to 73.3%, but weakened hazard
review recall from 90.9% to 45.5% and Malayalam exact match from 60.0% to 33.3%.
V2 omitted missing-hazard-duration and unknown-hazard-status examples from its
training split. This does not prove that those omissions alone caused the
regression, but it makes broad replacement of V1 a poor next experiment.

The frozen suite has already informed dataset design. It is now a regression
suite, not an unbiased estimate of generalization. No exposed case may be copied,
translated, lightly paraphrased, or have only its names/numbers changed for V3.

## Composition

V3 training is the exact V1 training export plus a separately versioned 60-record
augmentation:

| Component | Records | English | Malayalam | Manglish |
|---|---:|---:|---:|---:|
| Frozen V1 training | 240 | 144 | 48 | 48 |
| New V3 augmentation | 60 | 36 | 12 | 12 |
| Combined V3 training | 300 | 180 | 60 | 60 |

V3 validation is the exact V1 validation split plus 24 new validation records:

| Component | Records | English | Malayalam | Manglish |
|---|---:|---:|---:|---:|
| Frozen V1 validation | 48 | 24 | 12 | 12 |
| New V3 validation extension | 24 | 12 | 6 | 6 |
| Combined V3 validation | 72 | 36 | 18 | 18 |

V1 candidate/export bytes and review metadata remain unchanged. The combined
exports must record both V1 and augmentation hashes. V1 validation remains
validation and must never enter training.

Before authoring train/validation, create an 18-record development pilot: six
English, six Malayalam, and six Manglish records. These are distinct scenarios,
not translations of one shared set. Dev records never enter train or validation.

## Sixty-record training augmentation

Each subtype contains six independently authored situations. Six subtypes use
four English, one Malayalam, and one Manglish record. To preserve the approved
36/12/12 aggregate language balance, two subtypes use three English, two
Malayalam, and one Manglish record, and two use three English, one Malayalam,
and two Manglish records. A subtype may share an annotation rule, but must not
share a fill-in-the-slots sentence template. All review-required examples
preserve unrelated explicit facts while setting only unresolved fields to `null`.

| Subtype | Records | Required behavior |
|---|---:|---|
| Conflicting headcount | 6 | Same group has incompatible counts; `headcount=null`, review true. |
| Conflicting pickup location | 6 | Same incident has unresolved pickup places; `victim_location=null`, review true. |
| Conflicting requested asset | 6 | Same dispatch has incompatible explicit assets; `required_asset=null`, review true. |
| Approximate/range headcount | 6 | No exact verified count; `headcount=null`, review true. |
| Missing/deictic pickup location | 6 | “Here”, area-only, failed attachment, or absent locator; location null. |
| Need described without explicit asset | 6 | Do not infer a vehicle from injury, mobility, water, or terrain. |
| Unknown hazard status | 6 | Named/anchored road is explicit but OPEN/BLOCKED/FLOODED is unresolved; status null. |
| Missing hazard duration | 6 | Road and status are explicit; no elapsed duration is stated; duration null. |
| Unanchored hazard road | 6 | Keep explicit status/duration, but deictic road locator remains null. |
| Complete boundary counterexamples | 6 | Similar surface cues with all facts explicit; review false, preventing blanket escalation. |

The complete counterexamples must cover both conflicts that are explicitly
resolved by the reporter and hazards whose road, status, and duration are all
explicit. They cannot simply repeat V1 clear templates.

## Validation extension

The 24 new validation records use distinct events, discourse structures, place
names, and wording families. They contain 12 English, six Malayalam, and six
Manglish records with this allocation:

- nine contradiction cases: count, location, and asset;
- three missing/underspecified critical-field cases;
- nine incomplete-hazard cases: unknown status, missing duration, and unanchored road;
- three complete counterexamples.

Validation examples must not be derived from training scenarios. They are used
for checkpoint selection and training diagnostics, not final claims.

## Development pilot gate

The first artifact is the 18-record dev pilot. It must include at least:

- six contradiction cases spanning count, location, and asset;
- six incomplete-hazard cases spanning status, duration, and road identity;
- three missing/underspecified scalar cases;
- three complete counterexamples.

Each record is a distinct scenario. Review all fields, Malayalam/Manglish fluency,
uncertainty reasons, and confidence labels. Fix systematic generator/specification
issues before authoring the 84 train/validation-extension records.

## Independence and leakage rules

1. Do not read frozen regression inputs while authoring individual V3 messages.
   Evaluation results may identify abstract failure classes only.
2. Reject exact normalized input overlap with V1/V2 train, validation, dev, and
   frozen evaluation data.
3. Assign stable `scenario_family` and `wording_family` identifiers without split
   names. Each family belongs to exactly one of dev, train, validation, or blind test.
4. Do not create cross-split variants by translating, changing numbers, changing
   places, reordering clauses, or swapping assets in the same scenario.
5. Measure near-duplicate similarity across all splits, then manually inspect
   flagged pairs. Exact hashes alone do not establish semantic independence.
6. Do not use V2 candidate records as V3 training data. V2 results may inform only
   the abstract coverage requirements above.
7. Keep the frozen 120 cases and V3 dev records out of training and validation.
8. Use fictional incident details and never include real PII or live operations.

## Review and export gates

- Generate only the 18-record dev pilot first.
- After its first-pass corrections stabilize the policy and wording, generate the
  60 training and 24 validation-extension candidates.
- Review every new record. Track AI-assisted, human, and independent review
  accurately; do not call an AI review a domain-expert review.
- Export only accepted records under an explicitly recorded policy.
- The combined export must prove that the 240/48 V1 portions match their original
  exported SHA-256 hashes and that no dev/evaluation record is included.

## Training controls

Train a fresh QLoRA adapter from
`Qwen/Qwen3-4B-Instruct-2507` revision
`cdbee75f17c01a7cc42f958dc650907174af0554`. Do not initialize from V1 or V2.
Keep prompt V2.1.1, schema, LoRA rank/alpha/dropout, seed, quantization, maximum
length, optimizer, batch accumulation, and serving configuration fixed initially.

Because the dataset grows from 240 to 300 records, two epochs produce 25% more
example exposures. Record this as an intentional consequence of augmentation.
Do not describe V1/V3 as a strict data-only comparison unless the comparison
statement explicitly includes the changed record count and optimizer-step count.

## Decision gates

The exposed 120-case suite is a regression diagnostic. V3 must at minimum:

- retain 100% schema validity and zero inference errors;
- exceed V1's raw and effective review recall (both 56/68);
- retain V1's hazard raw review recall of at least 10/11;
- retain 20/20 clear-case exact matches;
- avoid regression below V1's 9/15 Malayalam and 12/15 Manglish exact matches;
- reach at least V1's 93/120 overall exact matches;
- reduce unsupported-fact cases from V1's 21/120, or at minimum not increase them;
- improve contradiction review recall beyond V1's 8/15.

These are experiment-continuation gates, not operational certification. The
documented 99% review-recall target on 68 positive cases effectively requires
68/68 on this suite; even that would be regression evidence because the suite is
exposed.

Only after V3 clears the regression gates should the project create and freeze a
new independently reviewed blind holdout. Promotion claims require that holdout,
category/language reporting, and deterministic fail-closed behavior. Expansion
toward 1,000 reviewed records is considered only if V3 demonstrates that targeted
augmentation preserves V1 strengths while repairing abstention behavior.

## Next action

Author the 18-record V3 dev pilot and its manifest. Do not generate the training
or validation extension until the pilot has completed first-pass review.
