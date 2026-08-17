# HANDOFF — Hermes Context Observability

**Status:** committed to `BluePaladinLLC/buzz`, branch `claude/hermes-context-observability-699r57`.
Phase 1–4 implementation work targets Hermes fleet config (`~/.hermes/config.yaml`), not this repo —
see [Where the work lands](#where-the-work-lands).
**Prepared:** 2026-08-16, from a chat session evaluating DeepSeek Harness (dsh) as a Hermes alternative.

---

## Goal

Make Hermes' per-request context **visible** before deciding anything about harness migration. Today we can see outcomes (CortexOS receipts) but not what the model actually saw at decision time — so when an agent drops a constraint mid-loop, we can't tell whether compaction ate it or the model ignored it.

## Decision already made

**Do not migrate to DeepSeek Harness.** Investigated and rejected for now.

- dsh is MIT, real, and genuinely novel on traceability — append-only session log, `select`-style compaction as a swappable plugin, replayable trajectories.
- But it is **v0.1 developer preview with explicitly promised breaking changes**, single-node, and has no A2A bridge, no goal/judge loop, no cron chaining, no CortexOS receipt sink, and no Buzz ACP adapter. Parity is ~5–6 plugins against a moving API.
- The thing we actually wanted from it — pre-request context visibility — **Hermes already supports.**

Keep dsh on the watchlist. Reassess at 1.0.

## Current state — what Hermes already gives us

Verified against Nous Research docs (hermes-agent, main branch):

| Seam | What it does | Limitation |
|---|---|---|
| `pre_llm_call` hook | Fires at the same point as Claude Code's `UserPromptSubmit`; supports context injection. Declarable as a **shell script** in `~/.hermes/config.yaml` — no Python plugin authoring. | **Inject-only by design.** Appends to the user message, never rewrites the message list (preserves prompt-cache prefix). |
| Context Engine plugin — `select_context()` | The **only** verb that can replace which messages enter a request. Takes ownership of the session's compaction policy. | Heavier. Owns compaction — don't implement casually. |
| `session:compress` event | Carries `in_place` and `old_session_id`. | Observation only. |
| Compaction archive | With `compression.in_place: true` (default), pre-compaction turns are **soft-archived** under the same session id (`active=0, compacted=1`), still searchable via `session_search`, never deleted. | We are not reading this today. |

**Implication:** the pre-compaction history we thought we lacked already exists on disk. This is the main finding.

## Where the work lands

This document lives in `BluePaladinLLC/buzz` because that is the repo the handoff session was
scoped to. The *implementation* of Phases 1–4 does not belong here — it is Hermes fleet
configuration (`~/.hermes/config.yaml`, hook scripts, Context Engine plugins). Suggested home
remains a `hermes-config` repo on branch `feat/context-observability`.

Buzz's only contact surface with Hermes is the ACP harness: `hermes-agent` ships as a Tier-2
preset (`hermes-acp`) in `desktop/src-tauri/src/managed_agents/discovery/presets.rs`, and
`crates/buzz-acp/src/acp.rs` sets Hermes-specific env defaults (e.g.
`HERMES_ACP_SKIP_CONFIGURED_MCP`) when Buzz owns the process. If Phase 1's hook needs to fire for
Buzz-launched Hermes agents too, that env-default path in `buzz-acp` is where the wiring goes —
but confirm Phase 1 works standalone first.

## Work to do

### Phase 1 — Observe (do this first, ~1 file)

Wire a `pre_llm_call` shell hook that emits, per turn, to CortexOS:

- assembled request token count
- which plugins/sources contributed injected context (outputs are joined in plugin-discovery order, alphabetical by directory)
- mounted tool count and rough tool-schema token cost
- agent name, session id, model id

**Acceptance:** for any Hermes session, we can answer "how many tokens went into turn N, and where did they come from?" without re-running the turn.

### Phase 2 — Audit

Using Phase 1 data, answer the open questions below. Specifically: what model and what context do **heartbeat / auxiliary calls** use? This has been unknown for weeks and is a suspected token sink.

Also pull the archived pre-compaction turns via `session_search` and diff against post-compaction live context on a real FIU loop session. That diff is the governance-decay check.

### Phase 3 — Scope tools per role (the actual fix)

Separate axis from observability, and the higher-leverage one. Current fleet agents are all generalists that see every tool. Proposed role split, borrowed from how dsh's runtime modes work:

- **Scout** — read-only, cheap/fast model, no write tools mounted. Repo recon, issue triage, context gathering.
- **Builder** — full toolset, strong coding model, one workspace / one issue.
- **Judge** — minimal tools, **different vendor from Builder** (same-model review is self-agreement). Reviews diffs against acceptance criteria.
- **Conductor** — orchestration only, mid-tier reasoning. Least proven; defer.

Start with Scout → Builder → Judge. Conductor later.

### Phase 4 — Optional: port dsh's compaction policy

If Phase 2 shows Hermes' summarizer is dropping constraints, implement a Context Engine with dsh's ordering: **trim over-budget tool results first; only generate a summary node if that's insufficient.**

## Open questions

- **[genuine unknown]** What model do heartbeats and other auxiliary-model slots use? Phase 1 should answer this.
- **[genuine unknown]** Does `pre_llm_call`'s hook payload expose the *full assembled message list*, or only the user message? Docs say injection targets the user message; they don't state what the hook can *read*. **Check the source.** If it can't read the full list, Phase 1 escalates to a Context Engine with `select_context()`.
- **[genuine unknown]** Does dsh have an ACP adapter (would let it join Buzz)? `buzz-acp` currently drives Goose, Codex, and Claude Code — dsh is not on that list. Check the `dsh-plugin` GitHub topic. Only matters if we revisit dsh.
- **[decidable default]** Judge vendor: default to a non-OpenAI model via the existing OmniRoute pool with ZDR pinning. Proceed unless there's a client constraint.

## Assumptions

- CortexOS can accept an arbitrary structured log line from a shell subprocess. If not, Phase 1 writes JSONL to disk and CortexOS tails it.
- Fleet is on Hermes v0.20.0 and `compression.in_place` is at its default (`true`). **Verify before relying on the soft-archive behavior** — legacy `in_place: false` rotates session ids instead.
- GitHub remains the canonical artifact bus. Nothing here changes that.

## Guardrails

- **No Claude Max OAuth in any third-party harness** (Anthropic ToS, Feb 2026). Applies to Hermes, dsh, OmniRoute chains — all of it. Claude Max stays planning/advisory only.
- Client work (FIU) must stay on ZDR-pinned, Western-hosted routes. No free-tier rotation providers in a client combo.
- Redact keys. Hook output goes to logs — do not dump credentials or full file contents into the trace.

## Suggested skills for the picking-up agent

- `grill-with-docs` — before implementing, stress-test the Phase 1 hook design against the existing CONTEXT.md / ADRs.
- `to-issues` — break Phases 1–3 into tracer-bullet issues with the `cortex-ready` label.

## Pickup trigger (not created)

Proposed issue — **awaiting approval, do not create yet:**

- **Title:** Instrument Hermes pre-request context via `pre_llm_call` hook
- **Label:** `cortex-ready`
- **Body:** points at this HANDOFF.md, scoped to Phase 1 only.
