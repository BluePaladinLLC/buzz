# Buzz Typing and CLI Activity Implementation Plan

> **For Hermes:** Use the `nostr-agent-platform-integration`, `software-delivery-workflows`, and `kanban-operations` skills while executing this plan.

**Goal:** Every team CLI agent visibly and truthfully shows when it is composing a reply and when it is working, waiting, completed, or failed in Buzz DMs, channels, and threads—without adding progress-message noise or giving Buzz Desktop ownership of the external runtime.

**Architecture:** Hermes owns the runtime signals. It emits ephemeral, conversation-scoped Nostr kind `20002` typing events while composing and owner-encrypted kind `24200` lifecycle/tool events while working. Buzz Desktop consumes those signals through its existing observer ingestion and sidebar activity surfaces; Desktop remains a viewer/controller only where explicitly supported, never the process owner.

**Repositories:**

- Hermes emitter: `NousResearch/hermes-agent`
- Buzz renderer and end-to-end regression: `block/buzz`

## Current-state findings

- Hermes already has a generic keep-typing loop. Upstream v0.20.2 and the Sigma/Pons installed baseline leave the Buzz adapter's `send_typing()` as a no-op, but this is not true fleet-wide: Cortex, Vagus, and Synapse carried the same uncommitted native-typing/presence patch.
- The focused Hermes implementation is on `feat/buzz-native-typing-current`; its initial adapter/busy-session suite passes `46/46`, and Ruff/diff checks pass.
- The fleet patch is now durable in the private `BluePaladinLLC/hermes-fleet-customizations` repository (`fb98674` through `1ccc8ec`). The three host copies were reported byte-identical; the captured `patches/buzz-presence-typing/cortex.patch` has SHA-256 `1c65e62e20c1f7cd4d1ed81c209b6aa0b005ab3f752b450f41e4f0515e71957a`.
- Reconciliation on 2026-08-16 found **one typing implementation, not two competing designs**: the fleet patch and `feat/buzz-native-typing-current` use the same kind `20002` publisher, authenticated socket reuse, ACK correlation, stale reconnect, cancellation-safe half-authenticated socket cleanup, and disconnect generation guard. The focused branch additionally extracts generic event signing into `nostr_auth.build_signed_event()` and has substantially deeper typing transport tests. Retain the focused branch as the upstream typing lane; retire the fleet typing copy after the combined canary succeeds.
- The fleet patch covers ground intentionally absent from the focused typing PR: runtime presence heartbeat/offline publication, subprocess cancellation cleanup in `_exec_buzz`, and stricter textual mention boundaries. Its canonical 26-test suite passed `26/26` against the fleet-applied adapter. The same suite passed `23/26` against the focused branch; the three failures are the known bare-name/longer-handle mention-boundary behaviors, not typing failures.
- Fleet feature coverage is split: Sigma/Pons have dynamic joined-room discovery but no presence/typing patch; Cortex/Vagus/Synapse have presence/typing but no dynamic joined-room discovery. No deployed adapter has both. Dynamic discovery is captured under `patches/buzz-dynamic-room/` and remains associated with upstream issue `#75107` / PR `#80038`.
- Synapse's divergent p-tag suite is an unimplemented strict dispatch specification, not current behavior: textual `@name` without the agent's Nostr `p` tag must not dispatch, another identity's `p` tag must not dispatch, and an exact self `p` tag dispatches once. This directly intersects dispatch and the reported compulsive-reply bug, so it is tracked separately and must be proven RED before implementation.
- The current draft contains unrelated busy-ack and onboarding changes. The upstream typing PR must be reduced to the adapter, signing helper, and focused tests unless a separate change is justified.
- Real thread scoping is not yet proven: inbound Buzz `e` tags must be propagated into `SessionSource.thread_id` before thread-specific typing tags can be claimed end to end.
- Buzz Desktop already ingests owner-encrypted kind `24200` events for viewer-owned relay agents and decorates DM and channel sidebar rows with active work. Relevant code includes:
  - `desktop/src/features/agents/useAgentObserverIngestion.ts`
  - `desktop/src/features/agents/observerRelayStore.ts`
  - `desktop/src/features/agents/activeAgentTurnsStore.ts`
  - `desktop/src/features/sidebar/lib/useActiveWorkingChannelsById.ts`
  - `desktop/src/features/sidebar/components/ChannelActivityPopover.tsx`
- Buzz's existing focused observer/sidebar tests passed `89/89` during discovery. What remains is production-shaped CLI-agent emission plus an end-to-end Desktop regression and installed-client acceptance.

## Acceptance criteria

1. **Typing everywhere**
   - A human sees a typing indicator while a Hermes CLI agent is composing in a normal channel.
   - The same indicator appears in a DM.
   - Thread activity is scoped to the correct root/reply convention and does not decorate unrelated conversation rows.
   - Typing expires quickly and never fails the agent turn if the relay is slow or unavailable.
2. **Truthful activity**
   - The sidebar shows `Working` for an owned external CLI agent during an active turn in both DM and channel rows.
   - The activity view shows tool start/progress/completion and clear waiting, completed, and failed outcomes.
   - A background job waiting for input is distinguishable from one actively progressing.
   - Completion/failure clears the active `Working` state without leaving a stale indicator.
3. **Noise and ownership**
   - Native activity replaces routine progress/redirect chatter where enabled; final replies and real blockers remain normal chat messages.
   - Buzz Desktop does not gain Start/Stop/Restart or secret custody for externally managed Hermes agents.
4. **Delivery proof**
   - Focused unit/integration tests pass in both repositories.
   - An independent review covers signing, ACK correlation, cancellation/socket cleanup, thread scoping, trust/owner filtering, and stale-state clearing.
   - A single safe Sigma canary visibly passes one real channel turn and one real DM turn before fleet rollout.
   - Changes are committed and pushed to durable repository branches with upstream-ready PR descriptions and rollback notes.

## Fleet follow-up register

Keep these items visible but separate from the acceptance path for typing plus truthful activity:

1. **Synapse mention-dispatch policy and configuration** — verify live `BUZZ_REQUIRE_MENTION`, `platforms.buzz.require_mention`, identity/profile name, and actual inbound `p` tags before changing code. The current adapter accepts a boundary-safe textual `@name` when mention gating is enabled; Synapse's undeployed divergent suite instead requires an explicit self `p` tag. Determine whether Synapse is chatty because mention gating is disabled/divergent or because textual matching is the wrong policy. Do not infer causality from the test patch alone.
2. **Fleet configuration convergence** — inventory the five agents' Buzz transport, room discovery, mention policy, allow-list, and presence settings with secrets redacted; define one canonical profile plus intentional exceptions.
3. **Combined adapter parity** — merge and test dynamic joined-room discovery, presence lifecycle, native typing, and cancellation cleanup before retiring host-local copies.
4. **Presence protocol scope** — decide whether presence belongs in the same upstream proposal or a stacked follow-up; typing and activity acceptance do not require permanent online state.
5. **Production test dependencies** — production hosts need not install pytest, but every fleet patch must be exercised in an isolated locked checkout before deployment.

## Non-goals

- Replacing final assistant replies with activity events.
- Giving Buzz Desktop lifecycle ownership of Hermes runtimes.
- Broad presence, directory, identity migration, dynamic-room discovery, mention-policy changes, or unrelated busy-ack refactors in the narrow typing PR. These may form a coherent follow-up integration proposal, but they must not obscure review of the typing transport.
- Installing an unsigned Desktop build or weakening Gatekeeper.

## Work plan

### 1. Reduce and finish the Hermes typing patch

**Files:**

- Modify: `plugins/platforms/buzz/adapter.py`
- Modify: `plugins/platforms/buzz/nostr_auth.py`
- Modify: `tests/gateway/test_buzz_adapter.py`

**Steps:**

1. Rebase the focused branch on current `origin/main` and remove unrelated files from the typing diff.
2. Keep canonical kind `20002` signing, authenticated socket reuse, serialized ACK handling, cancellation-safe cleanup, stale reconnect, and disconnect cleanup.
3. Preserve the generic `nostr_auth.build_signed_event()` extraction and deeper transport/race tests from the focused branch; do not create a third typing variant.
4. Add inbound root/reply `e`-tag parsing and propagate the thread root to `SessionSource.thread_id`.
5. Add focused tests for channels, DMs, thread propagation/tags, malformed/rejected/unrelated ACKs, cancellation, socket reuse, stale reconnect, and disconnect races.
6. Run:

```bash
.venv/bin/python -m pytest -q tests/gateway/test_buzz_adapter.py
.venv/bin/python -m pytest -q tests/gateway/test_keep_typing_timeout.py tests/gateway/test_typing_indicator_toggle.py
.venv/bin/python -m pytest -q tests/gateway/test_run_progress_topics.py
.venv/bin/ruff check plugins/platforms/buzz/adapter.py plugins/platforms/buzz/nostr_auth.py tests/gateway/test_buzz_adapter.py
git diff --check
```

### 1a. Reconcile the complete fleet adapter without widening the typing PR

1. Build and test one combined adapter containing dynamic joined-room discovery, native typing, presence lifecycle, `_exec_buzz` cancellation cleanup, and the accepted mention policy.
2. Keep the narrow typing PR reviewable; land combined behavior through separate commits/PRs or a clearly stacked integration branch.
3. Run the captured canonical fleet suite and the upstream Buzz adapter suites against the combined tree.
4. Canary the combined tree on Sigma before retiring host-local fleet patches.

### 1b. Prove or reject the strict p-tag dispatch model

1. Import Synapse's divergent p-tag tests as regression tests and first prove they fail against the current adapter for the intended reason.
2. Trace DM, channel, thread, and relay-subscription behavior so a dispatch gate does not break legitimate direct messages or explicit agent addressing.
3. Implement the smallest dispatch-only fix: self `p` tag required for shared-room dispatch; textual names alone do not authorize a turn; dispatch exactly once.
4. Keep this change separate from typing transport because both touch `adapter.py` but solve different user-visible contracts.
5. Verify the reported compulsive-reply symptom in Buzz. Treat Discord as a separate adapter/policy investigation rather than assuming the Buzz fix applies there.

### 2. Prove Hermes observer activity emission

**Files:** Determine exact emitter paths from the current Hermes observer branch before editing.

**Steps:**

1. Isolate the smallest kind `24200` emitter from prior exploratory work rather than porting the broad branch wholesale.
2. Emit owner-encrypted `turn_started`, tool call/update, liveness/waiting, `turn_completed`, and `turn_error` frames keyed to the exact Buzz `channelId`/DM ID/thread context.
3. Keep publication best-effort and prevent activity telemetry from changing turn outcome.
4. Document the publish-only MVP and explicitly exclude unsupported reverse controls.
5. Add unit/integration tests for encryption, owner targeting, lifecycle ordering, completion/error cleanup, and disabled/no-owner behavior.

**Implementation note (2026-08-17):** The publish-only MVP is implemented behind `BUZZ_OWNER_PUBKEY`. Hermes signs owner-targeted kind `24200` events, encrypts compact observer envelopes with NIP-44 v2, emits start/tool/liveness/terminal frames, drains queued tool publications before the terminal frame, and ignores reverse controls. The local interoperability test decrypts with `nostr-tools` when `NOSTR_TOOLS_CWD` is supplied. Live relay/Desktop acceptance remains a Sigma canary gate; implementation and tests alone do not prove sidebar rendering.

### 3. Add Buzz end-to-end regression coverage

**Files:**

- Modify: `desktop/tests/e2e/channel-activity-popover.spec.ts`
- Possibly modify: `desktop/src/features/agents/ui/agentSessionTranscript.ts`
- Possibly modify: `desktop/src/features/agents/ui/TurnLivenessIndicator.tsx`
- Modify corresponding focused tests only if literal completed/waiting rows are missing.

**Steps:**

1. Seed a relay-only, viewer-owned external CLI agent.
2. Feed `turn_started → tool_call → tool_call_update(completed) → turn_completed` through the production-shaped observer path.
3. Assert Working badge/popover and tool state in a channel.
4. Repeat for a DM and verify badge removal/completion behavior.
5. Add only the smallest renderer changes required for unambiguous waiting/completed/failed states.

### 4. Review, publish, and canary

1. Independently review both diffs against the acceptance criteria.
2. Push durable branches and open/update upstream-ready PRs with exact tests, risks, and rollback.
3. Preserve a rollback artifact before touching the live Sigma runtime.
4. Deploy one safe Hermes canary; do not install unsigned Buzz Desktop code.
5. Bruno verifies visible typing and activity in one channel and one DM.
6. Roll team agents one at a time only after canary acceptance.

## Rollback

- Hermes: restore the pre-canary adapter/runtime package and restart only the affected user-scope gateway.
- Buzz: no installed-client change without a signed artifact and explicit approval; source/test changes alone do not alter Bruno's app.
- Native typing and observer emission are best-effort and can be disabled/reverted without changing conversation history.

## Kanban source of truth

Hermes Kanban board: `buzz-cli-activity`

The board carries the execution state. This document carries the stable goal, architecture, acceptance criteria, and verification contract.
