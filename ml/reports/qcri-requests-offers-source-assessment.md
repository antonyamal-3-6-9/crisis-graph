# QCRI Requests and Offers: source assessment

Inspected: 2026-09-20. Purpose: assess downloadable real messages for CrisisGraph triage training. No records were added to training or evaluation.

## Provenance and local files

- Publisher page: https://crisisnlp.qcri.org/requests_offers/
- Download: https://crisisnlp.qcri.org/downloads/requests_offers_data.zip
- Local archive: `data/raw/qcri-requests-offers/requests_offers_data.zip`
- SHA-256: `b42077b3b46e49a84ce162041e94562b5582cd74be8fc0501e773fef8427b42d`
- Extracted files: `data/raw/qcri-requests-offers/request_offers_data/`
- Existing `/data/raw/` Git ignore rule covers the download and extracted contents.
- The archive also contains taxonomy, source references, and six prompt files. These were not executed.

## Access terms

The publisher page states CC BY-NC-SA 4.0 for noncommercial research. Its additional Terms of Use link resolves to `https://crisisnlp.qcri.org/requests_offers/terms-of-use.html`, which returned HTTP 404 during inspection. No license file was present in the archive listing. Record this ambiguity before redistribution or training-artifact publication; this assessment does not establish unrestricted reuse rights. Raw posts remain local and ignored.

## Findings

| Measurement | Result |
|---|---:|
| Records in `real_data.json` | 307 |
| Records in `synthetic_data.json` | 1,346 |
| Request-only classification | 137 |
| Offer-only classification | 54 |
| Both request and offer | 25 |
| Other classification | 91 |
| Exact duplicate excess in real text | 2 (305 unique texts) |
| Real posts mentioning Sandy, case-insensitive | 243 |
| Real posts containing Malayalam Unicode characters | 0 |
| Real posts explicitly mentioning Kerala, Aluva, Malayalam, or Manglish | 0 |
| Real posts matching rescue/trapped/stranded/evacuation word patterns | 4 |
| Real posts matching ambulance/boat/helicopter/truck word patterns | 2 |
| Nonempty source location labels | 80 |
| Nonempty source time labels | 35 |

Counts were computed from the full real JSON file. Keyword counts are screening heuristics, not semantic labels or proof of geographic/language absence. No automatic language detector was run; sampled messages were English. Zero Malayalam-script matches does not prove absence of romanized Malayalam.

The 91 Other records contain only `type` in their classification objects; the remaining 216 have action/personnel/supplies request and offer labels, location, time, alpha, actionability, and explanation. These are not CrisisGraph extraction targets. The archive alone does not establish how independently the labels were verified.

Manual inspection of 17 selected records (distributed through the file and including every rescue/asset keyword hit) found donation drives, fundraising, volunteering, shelter information, and commentary. The four rescue-related keyword hits were shelter information or fundraising, not direct requests to rescue a specified group. The two truck hits described a volunteer travelling and a broadcasting truck. This was a targeted sample, not a full semantic audit of all 307 posts.

## Fit for CrisisGraph

This is a real-message source according to the publisher, but it is a weak source for the current local SOS extraction task. It does not establish Malayalam/Manglish coverage and contains no ready-made headcount, required-asset, road-hazard, or human-review ground truth for our schema.

Potential use: a small supplementary challenge set for distinguishing actionable victim SOS messages from donations, offers, news, and general disaster discussion. Review and deduplicate before selecting examples. Source `actionability` is about humanitarian requests/offers and must not be mapped directly to dispatch readiness or `needs_human_review`.

Recommendation: retain as an inspected supplementary source; do not use it as the principal V2 fine-tuning dataset. Do not translate English posts and describe the results as authentic Manglish. A full manual pass might find additional useful examples, but the observed evidence does not support a large local extraction corpus. No new GPU run is justified by this download alone.
