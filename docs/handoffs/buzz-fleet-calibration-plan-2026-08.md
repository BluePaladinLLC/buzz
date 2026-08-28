# Buzz fleet calibration and acceptance plan

**Date:** 2026-08-19 UTC
**DRI:** Sigma
**Mode:** Calibration first; no fleet mutation until the observed differences are classified.

## Goal

Establish one comparable operability baseline for Sigma, Tracer, Baptiste, Reinhardt, Winston, D.Va, and Zenyatta; determine which differences are defects versus intentional architecture; then repair and accept each user-visible contract in a controlled order.

## Non-goals

- No fleet-wide deployment before Sigma passes the native canary.
- No identity/key migration, Desktop reset, teardown, or credential rotation.
- No attempt to make Zenyatta use the Hermes-native lifecycle.
- No unsigned Desktop artifact.
- No user retesting while machine-verifiable prerequisites predict failure.

## Acceptance contract

For each Hermes-native runtime, calibrate these gates independently:

1. **Identity:** canonical pubkey, profile, owner provenance, active/non-archived state.
2. **Directory:** effective kind `10100`, intended policy, and client read-path eligibility.
3. **Discoverability surfaces:** composer `@`, New DM, add-member picker.
4. **Presence:** runtime heartbeat, relay readback, Desktop online/offline rendering.
5. **Messaging policy:** exact mention/reply, non-mention silence, exactly one responder.
6. **Typing:** real-turn kind `20002`, relay ACK, authorized second-identity delivery, installed Desktop rendering.
7. **Activity/sidebar:** safe owner-encrypted kind `24200`, owner subscription/decryption, installed UI projection.
8. **Lifecycle truth:** external Hermes agents have no Desktop Start/Stop/secret controls.
9. **Durability:** runtime and Desktop restart preserve all accepted behavior.

Zenyatta is scored against the same user-visible outcomes but through the Desktop-managed kind `30177`/local-key path. Hermes-native typing or external-lifecycle controls are not copied onto Zenyatta.

## Phase 1 — Read-only fleet calibration

Run the same evidence pack for every identity and timestamp every result:

- canonical identity and effective profile head;
- active/archive state;
- effective directory or managed-agent record;
- current supervised process and effective adapter provenance;
- explicit fleet presence query with returned and missing sets;
- current membership and response policy;
- installed Desktop version/provenance and whether the relevant UI code is included.

Deliverable: one normalized evidence row per agent. Classify every difference as:

- **intentional architecture**;
- **version drift**;
- **configuration drift**;
- **relay/data drift**;
- **client eligibility defect**;
- **runtime publisher defect**;
- **unknown, requiring a narrow probe**.

No repairs occur during this phase.

## Phase 2 — Repair common prerequisites

Repair only issues that the calibration proves:

- stale or incomplete effective directory/profile head;
- archived identity;
- missing/expired presence heartbeat;
- wrong effective adapter or unsupported event semantics;
- client-side eligibility mismatch between composer, DM, and add-member paths;
- installed-client provenance gap.

Each repair requires rollback, targeted tests, effective readback, and one DRI. Do not normalize harmless host/service differences merely for visual symmetry.

## Phase 3 — Close the three known red boundaries

### 3A. Winston discovery and presence

1. Read the current profile/directory/archive heads for canonical Winston.
2. Prove whether presence heartbeats are currently advancing at the relay.
3. Compare the relay result with Desktop's current presence lookup.
4. Trace composer autocomplete separately from New DM/add-member eligibility.
5. If client-side, land and independently review the smallest predicate fix and deliver only through a genuinely signed artifact.
6. Issue one combined Winston UAT only after the backend and installed-client paths predict success.

Pass: `@Winston` resolves, Winston renders online while active, DM reply remains exactly once, and restart preserves both.

### 3B. Sigma native typing

1. Reproduce a sustained real turn on the exact DM.
2. Capture publisher event IDs and relay ACKs.
3. Prove delivery to an authorized second identity subscribed to the exact DM.
4. Inspect relay authorization/filtering if ACK succeeds but subscriber receipt fails.
5. Only after second-identity receipt passes, prove installed Desktop subscription/expiry/rendering.
6. Remove temporary diagnostics after acceptance and rerun regression tests.

Pass: Bruno sees sustained “Sigma is typing…” during one real turn; no false/stale indicator remains afterward.

### 3C. Sigma ACP/activity sidebar

1. Prove the loaded Sigma runtime emits a bounded, safe kind `24200` frame during a real turn.
2. Prove relay acceptance and owner-authorized retrieval/decryption.
3. Prove installed Desktop indexing and activity/sidebar projection.
4. Verify no secrets, raw credential material, or unrestricted internal payloads render.
5. Verify typing remains the fallback when observer telemetry is absent.

Pass: one real Sigma turn shows safe live activity in the intended installed Desktop surface and clears correctly.

## Phase 4 — Canary acceptance

Run one short consolidated Sigma pass:

- discoverable in intended surfaces;
- online presence;
- exact mention/reply and negative silence;
- sustained typing;
- safe activity/sidebar;
- external lifecycle truth;
- runtime restart durability;
- Desktop restart durability.

A backend pass cannot substitute for the human-visible typing/activity checks.

## Phase 5 — One-at-a-time fleet rollout

Order: Tracer → Baptiste → Reinhardt → Winston → D.Va.

For each target:

1. Preserve rollback and record current effective provenance.
2. Deploy only the already accepted common runtime behavior.
3. Restart only that target's supervised gateway.
4. Re-run the explicit full-fleet presence query and confirm no regressions.
5. Run that target's discovery, presence, messaging, typing, lifecycle, and restart checks.
6. Stop on the first red gate; do not advance to the next agent.

Zenyatta receives a separate Desktop-managed validation and no Hermes-native deployment.

## Evidence and status discipline

- One row per agent, one owner per repair.
- States: `PASS`, `PARTIAL`, `FAIL`, `N/A`; include observation time and proof layer.
- Never promote source/config/relay ACK into installed UI acceptance.
- Every status update identifies what changed, what is still red, and whether Bruno should test.
- `TEST NOW` is issued only when all machine-verifiable prerequisites for one short UAT are green.

## Exit criteria

The project closes only when:

- every intended identity is currently discoverable and renders truthful presence;
- each Hermes runtime emits and renders native typing during a real turn;
- safe activity/sidebar works where supported;
- messaging policy is exactly-once and scope-correct;
- lifecycle ownership is truthful;
- restart durability passes;
- the final scorecard has no `FAIL` cells and all `PARTIAL` cells are either accepted or explicitly deferred with owner and scope.
