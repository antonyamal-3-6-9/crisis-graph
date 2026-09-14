# CrisisGraph AI engineering strategy

Status: Proposed architecture decision
Date: 2026-09-13

## Decision

CrisisGraph should not place an autonomous LLM agent in control of every
incident from intake through dispatch. The normal path should remain a typed,
event-driven workflow whose routing, allocation, mutation, and verification
steps are deterministic.

After the core workflow and model experiment are complete, CrisisGraph may add
one bounded **Escalation Copilot**. It activates only when the deterministic
pipeline requests clarification or produces
`EscalateHumanDispatcher`. It may gather evidence, call read-only tools, compare
contingencies, and draft an incident action plan. A dispatcher must approve any
consequential proposal, and deterministic services must revalidate it before a
state change or dispatch.

Do not add a multi-agent system merely to make the project appear agentic. Begin
with one orchestrated copilot and split distinct capabilities through tools.
Split it into multiple agents only when measurements show a real need for
independent contexts, permissions, parallel work, or failure isolation.

The existing Rust `CrisisOrchestrator` is a deterministic pipeline coordinator.
It is not an AI agent and should not be replaced by one.

## Why the fine-tuning experiment is justified

Fine-tuning is not required to make routing safe. The deterministic graph and
verification layers remain authoritative. The experiment asks whether a small
local model can improve candidate extraction and abstention enough to reduce
operator workload without weakening the safety boundary.

The frozen Qwen3-4B baseline provides a concrete reason to test this:

- 100% JSON Schema validity;
- 85.2% critical-field accuracy;
- 55.0% exact match;
- 66.2% raw human-review recall;
- 86.8% effective recall after deterministic null/schema guards; and
- nine required-review cases still escaping the deterministic guard because
  the model supplied plausible, correctly typed, but unjustified values.

Schema constraints solve structure, not semantic truth. A small QLoRA pilot is
therefore a valid hypothesis test for ambiguity handling, multilingual
extraction, and unsupported-fact reduction. It is not a commitment to deploy a
fine-tuned checkpoint.

The experiment must have a stop condition. Reject the adapter if it does not
materially improve review recall and unsupported-fact rate without regressing
schema validity or clear-case accuracy. The reviewed 240-record training pilot
exists to test whether a learning signal is present before spending time and
compute on a much larger corpus.

See `ml/reports/qwen3-4b-eval-v2.0.0-baseline.md` and
`ml/data/triage-sft-v1-review-guide.md` for the measured baseline and dataset
gate.

## Why an agent should not handle every request

Most incidents have a known sequence:

1. validate intake;
2. extract candidate facts;
3. resolve candidate locations and hazards;
4. allocate capacity atomically;
5. calculate a route deterministically;
6. verify every traversed segment; and
7. dispatch or escalate.

An LLM loop adds little value to this normal path. It increases latency,
non-determinism, cost, and the number of failure modes. It can also make the
system harder to audit while creating the false impression that fluent output
is an operational decision.

The valuable agentic problem begins where the deterministic workflow stops:
ambiguous reports, unavailable local resources, conflicting evidence,
unreachable destinations, and contingency coordination.

## Recommended boundary

```text
Connected intake and field reports
               |
               v
Typed deterministic CrisisGraph workflow
               |
       +-------+-------------------+
       |                           |
       v                           v
Verified normal dispatch     Clarification / escalation
                                   |
                                   v
                         Bounded Escalation Copilot
                         - gather evidence
                         - call read-only tools
                         - compare contingencies
                         - draft IAP / SITREP
                                   |
                                   v
                          Human dispatcher approval
                                   |
                                   v
                    Typed command + deterministic revalidation
                                   |
                                   v
                    Atomic reservation / verified dispatch
```

The copilot must never calculate a route, certify a segment, reserve an asset,
change shelter capacity, close a road, or authorize departure by itself.

## Useful copilot scope

The following responsibilities create a credible agentic-engineering problem
without weakening the core design.

### Evidence and clarification

- identify missing dispatch-critical facts;
- ask a dispatcher or caller a minimal clarification question;
- gather related reports and show their provenance;
- compare citizen, field-team, weather, GIS, and command-centre evidence; and
- surface contradictions without silently selecting one value.

### Contingency research

- query neighboring depots for mutual-aid candidates;
- find staging areas near locations inaccessible to large vehicles;
- request deterministic boat, road, or air feasibility checks;
- retrieve relevant SOP sections; and
- compare options by response time, evidence freshness, capacity, and stated
  constraints.

### Dispatcher support

- draft a structured incident action plan;
- produce a source-linked SITREP;
- explain why the normal pipeline escalated;
- present assumptions and unresolved risks; and
- submit, revise, or withdraw a proposal through a human-in-the-loop workflow.

These are planning and communication functions. The outputs are advisories, not
dispatch decisions.

## Tool and authority model

Initial agent tools should be narrow and typed:

| Tool | Authority | Expected behavior |
|---|---|---|
| `get_incident_context` | Read only | Return typed incident state and provenance |
| `query_related_reports` | Read only | Retrieve time-bounded evidence |
| `query_neighboring_depots` | Read only | Return mutual-aid candidates, not reservations |
| `find_staging_candidates` | Read only | Use deterministic geospatial search |
| `evaluate_transport_modes` | Read only | Invoke deterministic feasibility services |
| `retrieve_operational_procedure` | Read only | Return versioned SOP excerpts |
| `request_clarification` | Controlled side effect | Send an approved, auditable question |
| `submit_contingency_proposal` | Proposal only | Create a typed plan awaiting dispatcher approval |

The agent should have no direct Neo4j or Redis credentials. Approved proposals
must be converted into allow-listed typed commands. Before execution, the Rust
services reacquire locks, check freshness and authorization, recalculate any
route, and independently verify every segment.

Approval is not permanent. It must be bound to the incident, proposed action,
evidence versions, approver, and expiry time. Any material state change after
approval invalidates the proposal and requires revalidation or another human
decision.

## Single agent before multiple agents

A single tool-using copilot is sufficient for the first implementation. A
multi-agent graph becomes justified only if at least one of these conditions is
measured:

- evidence gathering and plan synthesis benefit from real parallel execution;
- different capabilities require separate credentials or data access;
- contexts become too large and degrade quality;
- specialist models materially outperform one shared model;
- independent critique catches failures that deterministic validation cannot;
  or
- one capability must fail without terminating the rest of the workflow.

If those conditions appear, possible bounded roles are Evidence Analyst,
Logistics Contingency Planner, and Communications/SITREP Drafter, coordinated by
a deterministic state machine. The so-called supervisor must not be an
unrestricted model with authority over the specialists or operational stores.

## Implementation sequence

1. Complete the Qwen3-4B fine-tuning pilot and make a measured keep/reject
   decision.
2. Finish and test the normal event-driven incident workflow, including stale
   evidence, concurrent reservations, and fail-closed escalation.
3. Define typed escalation reasons and the `ContingencyProposal` contract.
4. Implement read-only deterministic tools and replayable fixtures.
5. Add a single-agent ReAct-style loop with strict step, time, and tool budgets.
6. Add dispatcher review, amendment, approval, rejection, expiry, and audit
   events.
7. Revalidate every approved proposal through existing deterministic services.
8. Evaluate the copilot on recorded simulations before considering multiple
   agents.

## Agent evaluation gates

Do not assess the agent by how convincing its prose sounds. Measure:

- correct escalation-reason identification;
- appropriate tool selection and argument validity;
- evidence citation and provenance coverage;
- unsupported-claim rate;
- attempts to call forbidden or out-of-scope actions;
- compliance with human approval and proposal expiry;
- contingency feasibility after deterministic validation;
- clarification quality and number of interaction turns;
- latency, token use, and failure-recovery behavior; and
- dispatcher acceptance, amendment, and rejection rates in simulations.

Adversarial tests must include prompt injection inside reports and retrieved
documents, stale evidence, tool timeouts, conflicting sources, duplicate
incidents, approval replay, and state changes between proposal and execution.

## Go/no-go decision

The Escalation Copilot is worth building as an optional final milestone if it
reduces simulated escalation-handling time or improves evidence coverage while
producing zero unauthorized operational mutations and consistently respecting
human approval.

Close the project without a multi-agent layer if a deterministic escalation UI
and fixed contingency queries perform equally well. That is not a failed AI
project; it is evidence that the problem did not justify additional autonomy.

The strongest project story is not “AI runs disaster response.” It is:

> CrisisGraph combines constrained local language models with deterministic
> routing and verification, measures where model behavior fails, and introduces
> bounded agentic assistance only where open-ended evidence gathering and
> contingency planning justify it.

This wording describes the prototype honestly and demonstrates architectural
judgment rather than adding agents for presentation value.
