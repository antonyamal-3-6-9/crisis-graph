## What was tested

Two kinds of run, both from the surge tool (`src/bin/surge.rs`), each recorded with its scenario, seed and starting commit:

- **Run A (capacity).**
  - Reports arrive at stepped rates of 1, 5 and then 20 per second, with roads flooding mid-run.
  - Extraction is **simulated**: each report carries its ground truth and the engine returns it after a seeded, realistic delay.
  - Everything after extraction is real: Rust urgency rules, the location policy, deduplication, incident creation, the matcher, routing, live verification, and the simulated responders.
  - This measures CrisisGraph itself, not a language model.
- **Run B (real model).**
  - 60 synthetic reports at 10 per minute, extracted by **Gemini** (`gemini-3.5-flash-lite`) through its OpenAI-compatible endpoint with the strict JSON schema.
  - Runs on the free tier, paced at 10 requests per minute.
  - The texts are written in English, Manglish and Malayalam. No real or frozen test-set messages are sent to a hosted model.

Every run is cleaned up afterwards (`surge cleanup`), and the database is checked to be back to its prior state.

## What the runs found, and what was fixed

The runs were used to find defects as well as to measure. Each run below is kept as the record of what it showed.

| Found by | Defect | Fix |
|---|---|---|
| Run A | Tasks no vehicle could carry filled the matcher's window, so nothing behind them was ever matched while the fleet sat idle (**starvation**). | The window counts only tasks some vehicle can carry; regression test. |
| Run A + dashboard | Time to assignment was never recorded: a shadowed Cypher variable left the histogram empty. | Fixed; covered by `observability_tests`. |
| Prometheus alert | `HazardOverlayStale` fired falsely: overlay age measured the last *change*, not the last confirmed check. | Age is now time since the last successful check. |
| Run A | Pickups too big for one vehicle, or with no route for the requested vehicle type (trucks on `hgv=no` lanes), waited for a dispatcher indefinitely. | **T8 group plans** (owner decision): switch vehicle type and/or send several vehicles automatically, with an alert. Compare `run-a-capacity-pre-t8` with `run-a-capacity`. |
| Run A | Victims reachable only over tracks unsuitable for every road vehicle fell between the cut-off check and T8. | Both checks use the same per-type reachability; such victims go to the boat check and then to a person at once. |
| Run B | The surge texts (written for the simulated engine) named no location and no vehicle, so with a real model every report rightly went to review (`run-b-gemini-vague`). | Callers say they are on scene and which vehicle they need, as real callers do. |
| Run B | On-scene callers located by GPS were still held for review for "location not provided", and missing hazard details blocked rescues (`run-b-gemini-pre-review-fix`). Two separate gates applied the old rule. | One readiness rule for both gates (owner decision): our own satisfied checks no longer block; a missing vehicle type and the model's own doubts still do. |
| Run B | A Gemini `503` sent a report to review. | `503` is retried like `429`, then fails closed. |

## How to read the numbers

- **Single machine.** Neo4j, Redis and every role run on one laptop with no GPU.
- **Run A is a stress test well above realistic load** for a 10 km pilot area. It shows where the system saturates:
  - triage capacity first;
  - then the size of the fleet, since 33 road vehicles each take a simulated mission time.

  Intake stays fast throughout, and nothing is lost while the backlog drains.
- **Time to assignment** is measured from the first report to the first vehicle committed, by urgency tier. Group plans add vehicles for large groups. The post-T8 run therefore serves several hundred more groups than the pre-T8 run, and its higher times for CRITICAL incidents reflect that extra work, not a regression.
- **Run B's latency is set by the free tier's request rate,** not by CrisisGraph. Once a task is queued, a vehicle is committed in under a second. The remaining reviews are reports where the model itself expressed doubt, which is held for a person by design.
- **"Overbooking attempts blocked" is 0 in every run** because one fenced matcher leader makes every assignment, so no two writers compete for a vehicle. That no vehicle can ever be booked twice is shown by the concurrency stress tests:
  - `ops_model_tests`: many concurrent holds on few vehicles;
  - `matcher_tests`: dispatcher actions racing the matcher, and a fenced former leader.

  In those tests the database's uniqueness constraints reject every second reservation.
- **The pre-T8 Run A and the vague Run B** are kept to show the effect of each fix, not as current performance.

Screenshots: `grafana-run-a.jpg` (dashboard during Run A), `jaeger-one-sos-trace.jpg` (one SOS as one trace across the roles), `prometheus-no-matcher-leader-firing.jpg` (alert fired when both matchers were stopped; it cleared after restart).
