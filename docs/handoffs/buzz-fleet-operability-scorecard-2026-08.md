# Buzz fleet operability scorecard

**Baseline date:** 2026-08-19 UTC
**DRI:** Sigma
**Purpose:** Compare every agent using the same acceptance gates while preserving legitimate architecture differences.

## Scoring rules

- **2 — PASS:** current end-to-end or human-visible acceptance for that exact surface.
- **1 — PARTIAL:** source/config/backend/prior evidence exists, but current installed UI acceptance is absent or stale.
- **0 — FAIL:** current evidence predicts failure or Bruno observed the surface fail.
- **— — N/A:** intentionally inapplicable because the agent uses a different supported architecture.

A total is directional only. **Any zero blocks fleet acceptance.** Prior evidence older than the current incident is PARTIAL, not PASS.

## Architecture calibration

| Agent | Runtime owner | Identity/discovery path | Expected native stages | Important difference |
|---|---|---|---|---|
| Sigma | Hermes / Thalamus | Relay kind `0` + kind `10100` | Presence `20001`, typing `20002`, observer `24200` | Sole Hermes canary; Stage 2 code exists only here |
| Tracer | Hermes / Synapse | Relay profile + directory | Presence + typing; observer not deployed | Separate runtime host; same observable contract after rollout |
| Baptiste | Hermes / Vagus | Relay profile + directory | Presence + typing; observer not deployed | Effective adapter historically differed from peers |
| Reinhardt | Hermes / Cortex | Relay profile + directory | Presence + typing; observer not deployed | Correct host is Cortex-01 `10.1.1.117`, not ingress |
| Winston | Hermes / Pons | Relay profile + directory | Presence + typing; observer not deployed | Current discovery and presence failures despite working messages |
| D.Va | Hermes / Axon | Relay profile + directory | Presence + typing; observer not deployed | Windows/WinSW runtime; prior authoritative presence missing |
| Zenyatta | Desktop-managed/local-keyed | Managed kind `30177`; kind `10100` not expected | Desktop-managed ACP activity | Legitimately different lifecycle; never copy Hermes keys or controls |

## Uniform scorecard

| Agent | Identity | `@` autocomplete | New DM/add | Presence | Messaging | Typing | Activity/sidebar | Lifecycle truth | Restart | Score* |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| **Sigma** | 1 | 1 | 1 | 1 | **2** | **0** | 1 | 1 | 1 | **9/18** |
| **Tracer** | 1 | 1 | 1 | 1 | 1 | 1 | 0 | 1 | 0 | **7/18** |
| **Baptiste** | 1 | 1 | 1 | 1 | 1 | 1 | 0 | 1 | 1 | **8/18** |
| **Reinhardt** | 1 | 1 | 1 | 1 | 1 | 1 | 0 | 1 | 0 | **7/18** |
| **Winston** | 1 | **0** | 1 | **0** | **2** | 1 | 0 | 1 | 1 | **7/18** |
| **D.Va** | 1 | 1 | 1 | **0** | 1 | 1 | 0 | 1 | 0 | **6/18** |
| **Zenyatta** | 1 | 1 | 1 | 1 | 1 | — | 1 | 1 | 1 | **8/16** |

\* Scores measure evidence strength, not percentage completeness. `1` means “not currently accepted.”

## Agent differences and next checks

| Agent | Credible evidence | Broken or unaccepted | Next calibration check |
|---|---|---|---|
| Sigma | Current DM messaging; prior canonical identity/directory/presence | Typing canary failed; activity sidebar unaccepted | Exact-DM typing authorization/delivery, then installed activity proof |
| Tracer | Canonical identity/directory, supervised runtime, typing-capable adapter | No current visual discovery/presence/typing/sidebar/restart acceptance | Standard read-only backend pack after Sigma contract is fixed |
| Baptiste | Canonical repaired profile; prior mention/silence/restart | No current visual typing/sidebar acceptance | Compare differing adapter against final Sigma contract |
| Reinhardt | Canonical profile/directory and correct runtime host known | No current visual discovery/presence/typing/sidebar/restart acceptance | Probe Cortex-01 and compare effective semantics |
| Winston | Current replies; New DM previously passed | `@Winston` fails; shown offline while replying | Current directory/archive/presence versus composer eligibility |
| D.Va | Canonical profile/directory and typing-capable adapter previously present | Prior relay presence omitted D.Va; current state unknown | Establish Axon reachability and current presence |
| Zenyatta | Canonical Desktop-managed identity path | Installed custody/presence/activity not consolidated | Validate as Desktop-managed control, not Hermes target |

## Common operability contract

Every Hermes-native agent must ultimately have:

1. One canonical active identity and one supervised responder.
2. Effective profile/directory with verified owner provenance.
3. Intended composer, New DM, and add-member discoverability.
4. Advancing presence plus truthful Desktop online/offline rendering.
5. Exactly one authorized reply and correct non-mention silence.
6. Sustained visible typing during a real turn.
7. Safe owner-visible activity/sidebar telemetry.
8. No Desktop lifecycle controls for externally managed Hermes runtimes.
9. Runtime and Desktop restart durability.

**Calibration does not mean identical binaries.** It means identical observable contracts, compatible event semantics, classified drift, and an explicit Zenyatta lifecycle exception.

## Current decision

- **Fleet:** RED / not calibrated.
- **Canary:** Sigma only for typing and Hermes observer activity.
- **Known comparative failures:** Sigma typing; Winston autocomplete/presence; D.Va prior presence.
- **Operator instruction:** **DO NOT TEST YET.**
