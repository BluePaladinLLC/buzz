# Decision Record — CortexOS Memory Stack Final State

**Status:** Active. Execution in progress.
**Owner (approvals):** Bruno — the only party who authorizes mutations.
**DRI / checkpoint judge:** Zenyatta · **Executor:** Winston · **Observer:** Mercy (see §8)
**Date:** 2026-08-19
**Supersedes:** the "Hindsight Memory Architecture — Consolidated Recommendation (FINAL)"
of 2026-08-18 on every point where the two conflict. This record is authoritative.

This document exists so the work can continue without the chat that produced it.
Anything not written here is an open question for Bruno, not something to improvise.

---

## 1. The end state — five boxes

Each box answers exactly one question. A component that cannot claim one of these
lines is not in the architecture.

| Question | Box |
|---|---|
| "What's true for the whole fleet?" | **Canon** — git repos + wiki, human-gated, every agent reads |
| "What have *I* learned?" | **Hindsight** — one private bank per agent |
| "What happened, and when does it expire?" | **ClickHouse** — events/conversations/logs, TTL = the decay system. Grafana on top |
| "What's happening right now?" | **Valkey** — cache + agent state fan-out |
| "What did we do, and can we prove it?" | **Receipts** — Supabase/Postgres + S3 |

**Canon is the shared memory layer.** Not a fallback — the design. It is the only
durable surface the whole fleet can read, and it was never broken.

**Governing rule for canon vs bank:** canon is authored and authoritative; a bank is
accumulated and disposable. If they disagree, canon wins. Test: *deleting every bank
should cost you speed, not information.* Anything important that lives only in a bank
is misfiled and should be promoted.

## 2. Closed decisions — do not reopen

1. **No shared Hindsight bank.** Hermes 0.20.2 cannot natively fold a second bank into
   an agent's recall (runtime-proven). `team:cortexos` is frozen; its 43 facts get
   human review, survivors go to canon, then the bank is retired.
2. **Graphiti/Neo4j is retired.** Its unique promise — temporal "what's current"
   ranking — was never wired into recall; the live path ran plain RRF hybrid search,
   which Hindsight already does. Graphiti's own graph returned the retirement decision
   as its rank-1 fact. Teardown order: name live readers → reroute → archive Neo4j data
   → stop container.
3. **Self-dreaming stays dead.** Archaeology showed its proven value was improving
   Bruno's digest, not the models. If missed, the rebuild is a cron job that reads agent
   banks, writes a digest, and proposes canon promotions — not a memory layer.
4. **Fix in place; no side-by-side rebuild.** The "108,685 failed consolidations" figure
   that justified a teardown was never produced by any command. Real number: **156**
   (149 `scribe`, 7 `thalamus`); instance healthy, zero restarts. See §6.
5. **Upgrade 0.8.2 → 0.9.1 in place.** Calibrate expectations: conflict policies,
   recency-wins supersession and entity consolidation already exist in 0.8.x. 0.9.1 is
   consolidation bugfixes plus the cross-bank leakage fix — hygiene, not a feature
   unlock. Cheap on a healthy instance; not worth drama.
6. **Keep every live agent bank.** The "tear down per-agent banks" proposal is rejected:
   `bank_id_template` makes per-agent banks the supported pattern, and there are six
   live agents, not one self-learner.
7. **`write_approval` does not gate Hindsight retains.** Confirmed — it gates built-in
   MEMORY.md/USER.md only. The Hindsight plugin has no approval hook. Any control on a
   Hindsight bank is `auto_retain:false` plus a broker, nothing else.
8. **Valkey is cache/state only.** Not a message bus. The `cortex:coordination` stream
   is a dormant legacy artifact (31 days idle, no writer in stats); decommission it and
   the `scribe_daemon` reading it.
9. **Config drift ≠ memory drift.** The Aug 16–18 loss was siloed *config*. Fleet config
   normalization is a separate, cheaper workstream and must not be used to justify
   memory architecture work.

## 3. The fleet — six agents, not twelve

**Six Hermes agents hit Hindsight.** Vellum (own built-in memory), the CLIs, Buzz,
Claude Code — none of them touch it. The entire memory stack serves six agents.

The 36-bank inventory decomposes as roughly **6 live, ~30 archaeology**. The naming
pattern is the tell: `agent:<name>` is the previous generation, `agent:<name>-hermes`
is current.

| Current (live) | Stale predecessor |
|---|---|
| `agent:cortex-hermes` (42k, read-hot) | `agent:cortex` (27k, last write Jul 2) |
| `agent:scribe-hermes` (123k, read-hot) | `agent:scribe` (4 facts) |
| `agent:thalamus-hermes` (9.8k) | `agent:thalamus` (dormant since May) |
| `agent:synapse-hermes` (539 recalls — **absent from inventory**, see §5) | `agent:synapse` (dormant since May) |
| `agent:axon` **or** `agent:axon-assistant` — mapping table decides | the other one |
| `agent:cortex-chief-of-staff` (8.5k, written today) — distinct agent or a cortex profile? | — |

Dead experiments: `agent:cortex-hermes-eval`, `agent:hindsight-route-canary` (4 facts).
Old-generation banks join the legacy/archive tier, not the live fleet.

## 4. Acceptance criteria — the scorecard

Checkpoints are reviewed against this table, **not against effort**. Anything still
present at Checkpoint D that is not on it requires written justification or it goes.
The burden of proof is on keeping, not on deleting.

| Metric | Today | Done |
|---|---|---|
| Hindsight banks | 36 | **~6** (one per live agent; no shared bank) |
| Memory-related services | Hindsight, Graphiti, Neo4j, Valkey, ClickHouse, scribe daemon | **4** (Hindsight, Valkey, ClickHouse+Grafana, Receipts) |
| Unknown callers on the memory API | ≥2 | **0** |
| Banks without a named owner | most | **0** |
| Doc claims describing non-live capabilities | ≥4 | **0** |
| Daemons reading dead streams | 1 | **0** |
| Places to look when memory misbehaves | unclear | **1** (the Grafana panel, §7) |

## 5. Open items

1. **The instance→bank mapping table.** Owed three times, delivered zero times. Six
   rows: agent → host → profile → configured `bank_id` → observed write bank. Every
   count confusion in this effort collapses the moment it exists. **Hard blocker on
   Checkpoint A.**
2. **The `agent:synapse-hermes` ghost** — 539 recalls, absent from the bank inventory.
   *Leading hypothesis: it is simply synapse's current bank and the inventory endpoint
   fails to list it — an inventory bug, not an unknown caller.* Test this first.
3. **The `team:cortexos` reader** — 22 direct-API recalls, caller unidentified. Check
   the Vagus/scribe curation path first. Bruno created the bank; the writer is known,
   the reader is not.
4. **Graphiti's live readers** — Gap 5 said reads "exist and are used." By what?
   Teardown waits on this.
5. **Graph tooling on wiki/canon/repos** — backlink indexes, knowledge-graph exports,
   anything. Never inventoried. Nothing gets torn down until it is named.
6. **Conflict policy per bank** — Hindsight supports recency-wins / source-wins /
   confidence-wins with explicit invalidation. Report what is set; target is
   recency-wins-with-invalidation where unset.
7. **Built-in memory status** — docs say built-in memory is always active alongside an
   external provider, which conflicts with the "built-in off" readback. Resolve.

## 6. Process rules — earned, non-negotiable

- **Raw output or it did not happen.** A summary claimed 108,685 failed consolidations.
  No command ever produced that number; the real figure is 156. It was caught only
  because the judge refused to accept a write-up in place of machine output. Summaries
  in this pipeline can confabulate numbers. This rule is permanent.
- **Claimed artifacts must exist.** Two consecutive reports cited saved files that were
  not on disk. First check at every checkpoint: open the file.
- **Every experiment gets an expiry date at creation**, recorded in canon. On expiry:
  promote, extend in writing, or tear down. No expiry, no experiment. The ~30-bank
  graveyard is entirely experiments nobody scheduled a funeral for.
- **New components must argue something out.** Any addition to the memory stack must
  name which of the five questions it answers — and all five already have an owner. A
  new component therefore displaces one or does not land.
- **Mutations are Bruno's alone**, batched at checkpoints. Advisory output from any
  agent — including Mercy — never authorizes a restart, teardown, prune, or write.

## 7. Visibility — a deliverable, not a nicety

"Working OK but I can't see it" is not done. Two pieces, both on existing infrastructure:

1. **One Grafana memory panel** (Grafana already sits on ClickHouse): consolidation
   failures per bank, recall counts per bank, Hindsight/Valkey up-down, and — once the
   hook lands — tokens injected per turn by source. One pane, one place to look.
2. **Weekly one-line digest**: bank count, failure count, any *new* client seen on the
   memory API. A new unknown caller is an alarm, not archaeology six weeks later.

The **`pre_llm_call` observability hook** (scoped in `HANDOFF.md`, Phase 1) is what makes
"Hindsight says it was upstream" a checkable claim instead of an unfalsifiable excuse.
That is why it is in the plan and not optional.

## 8. Roles

- **Bruno** — sole approver of mutations. Decides at checkpoints.
- **Winston** — executor. Works the phases in order, one thread, no committee.
- **Zenyatta** — checkpoint judge. Engaged only at phase boundaries: verifies artifacts
  exist, claims match raw evidence, scope was not exceeded, promised tables are present.
  Verdict is PASS or FAIL-with-deficiency. Does not redesign or reopen §2.
  *This role caught the 108,685 fabrication. It stays independent.*
- **Mercy** (Buzz) — **silent observer**. Reads the working channel; replies **only to
  Bruno by DM**, never in the room. No authority, no in-channel voice, no instructions
  to Winston or Zenyatta. Everything she surfaces reaches the team through Bruno. She
  starts from this record; anything not in it is a question for Bruno.

## 9. Execution plan

**Phase 0 — Pin the facts (read-only).** All of §5. Plus Hindsight version/health
confirmation and consolidation backlog. → **Checkpoint A**

**Phase 1 — Make the docs true (doc writes only).** Write the five boxes into canon.
Strike: Graphiti as temporal authority · Valkey as A2A bus · Hindsight score-demotion
(never deployed) · self-dreaming as live. Add: ClickHouse TTLs are the decay system, by
design. → **Checkpoint B**

**Phase 2 — Fix and upgrade in place (mutations; each needs Bruno's go).**
1. Full backup; verify restorability first. *The backup is the transplant option held in
   reserve — this is what caps the downside.*
2. Pull failure reasons for the 156 and report the distribution **before** retrying. 149
   concentrated in one 123k-fact bank is a pattern, not noise; likely one bug, not 156
   problems.
3. Clear/requeue; canary consolidation on `scribe` and `thalamus`.
4. Pin `hindsight-client` fleet-wide (the `lazy_deps` silent-downgrade landmine), then
   upgrade 0.8.2 → 0.9.1 in place.
5. Retain/recall canary on every live agent bank.
6. Export `team:cortexos`'s 43 facts for Bruno's review; survivors → canon; retire bank.
7. Set recency-wins-with-invalidation where unset.

**Abort rule, agreed in advance:** >3 working days, or backup integrity ever in doubt →
stop, escalate, side-by-side-from-backup becomes the path. Decided now so nobody argues
it mid-incident. → **Checkpoint C** (backup verified, failure distribution, canary
results)

**Phase 3 — Teardown in dependency order (each item a separate approval).**
Graphiti readers → reroute → archive → stop. Decommission `scribe_daemon` +
`cortex:coordination`. Prune empty banks after the ghost reconciliation. Old-generation
agent banks to archive. Quarantine/private banks presented **one at a time** — never
batched. → **Checkpoint D**, closing with a one-page "this is now the architecture"
statement matching §1 and §4, or documenting any approved deviation.

**Phase 4 — The two builds.** The `pre_llm_call` hook (HANDOFF.md Phase 1 scope) and the
Grafana panel + weekly digest. Then document the promotion habit in canon: an agent-bank
fact that proves fleet-true gets promoted to canon by a human. Manual first. No
automation until it is missed.

---

## 10. What "done" feels like

Six agents, six banks, four services, one health panel. Each box has one job, so a
failure points at one box. Every caller is named, so nothing is a mystery. The docs match
reality, so nobody debugs against a capability that was never wired up. Bruno glances at
one panel weekly, approves the occasional promotion to canon, and stops thinking about
memory.

Not zero incidents — **legible** ones.
