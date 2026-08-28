# Hermes ↔ Buzz integration: complete implementation and operations handoff

**Prepared:** 2026-08-28
**Scope:** Work performed to make durable Hermes agents first-class Buzz participants: identity, presence, discoverability, invitations, room wake behavior, response policy, typing/Working state, ACP observer activity, Desktop rendering, packaging, and operational recovery.
**Canonical review repo:** [`BluePaladinLLC/buzz`](https://github.com/BluePaladinLLC/buzz)
**Companion implementation repo:** [`BluePaladinLLC/hermes-agent`](https://github.com/BluePaladinLLC/hermes-agent)

This is the index that was previously missing. It intentionally distinguishes implementation, relay/backend proof, installed-client proof, and unresolved acceptance. A relay `OK`, a green dot, and a visible reply prove different things.

## 1. Fast map for the next reviewer

| Area | Canonical branch / commit | Primary files | Current truth |
|---|---|---|---|
| Reachable relay-agent pickers | `BluePaladinLLC/buzz@main` `a7c5f97f6`; implementation `5340970cc`, test `b5c64b811` | `agentAutocompleteEligibility.ts`, `MembersSidebar.tsx`, `useMentions.ts` | **Merged to fork main.** Shows reachable room-member relay agents in mention and add-people pickers. |
| Earlier owner-agent picker repair | `fix/relay-agents-add-picker` `da65771c7`, `fb7cc659f` | same eligibility/picker surfaces | **Superseded by the broader main fix above**, retained for lineage. |
| DM agent Working/typing UI | `repair/desktop-v0.5.20-dm-safety`, including `fe27ddac8`; source lane `fix/dm-agent-activity` ending `81797e6cd` | `ChannelComposerActivityAccessory.tsx`, `useChannelAgentSessions.ts`, typing tests | **Implemented and tested; not represented as merged to fork main by this handoff.** Packaged preview work exists. |
| Hermes native typing + observer telemetry | `BluePaladinLLC/hermes-agent:feat/buzz-native-typing-current` ending `12540de408` | Buzz adapter, gateway run/turn context, NIP-44 module, tests | **Committed branch pushed.** A later 137-line working-tree refinement is attached as an exact patch in this handoff. |
| Dynamic room/DM subscription after invitation | `BluePaladinLLC/hermes-agent:feat/buzz-room-policy-canary` `466041b22d` | `plugins/platforms/buzz/adapter.py`, adapter tests | **Pushed.** Subscribes to newly joined conversations without restart. Later membership-boundary refinement is in the attached patch. |
| Room response coordination / exactly-once lease | `BluePaladinLLC/buzz:feat/room-response-coordination-cas` `824b0dd34` | relay API, DB layer/migration, CLI, `docs/room-response-coordination.md` | **Branch pushed for review. Not merged.** |
| Preserve complete kind `10100` profile on policy update | `fix/preserve-agent-profile-policy-update` `0c3c64eb7` | `crates/buzz-cli/src/commands/channels.rs` | **Pushed branch.** Prevents sparse policy writes from erasing directory metadata. |
| Materialize NIP-OA owner for direct agents | `fix/nip-oa-direct-member-owner-materialization` `fa6d8c34e` | relay auth/API | **Pushed branch.** |
| Keep active room agents mentionable | `fix/winston-room-mention-urgent` `9c8b5cca3` | autocomplete eligibility + tests | **Pushed branch; superseded in breadth by the merged reachable-agent picker fix.** |
| macOS/runtime repair and reversible registry cleanup | `preview/desktop-v0.5.20-local-fixes` `0114a9e4f` plus `8a648fcdd`, `5f23db492`, `755afe6d6` | managed-agent runtime discovery and quarantine modules/tests | **Source preserved; review findings remain open. Do not execute live registry migration from this branch as-is.** |
| Internal Buzz ingress recovery | operational repair, not source feature | dual Traefik route/cert state | **Recovered and failover-tested.** This is infrastructure evidence, not product-source completion. |

### Review URLs

- Buzz handoff branch: `https://github.com/BluePaladinLLC/buzz/tree/docs/buzz-integration-handoff`
- Hermes typing/activity branch: `https://github.com/BluePaladinLLC/hermes-agent/tree/feat/buzz-native-typing-current`
- Hermes dynamic-room branch: `https://github.com/BluePaladinLLC/hermes-agent/tree/feat/buzz-room-policy-canary`
- Room coordination branch: `https://github.com/BluePaladinLLC/buzz/tree/feat/room-response-coordination-cas`
- Fork main picker implementation: `https://github.com/BluePaladinLLC/buzz/commit/a7c5f97f6`

## 2. The model: three separate planes

The work only becomes legible when split into three planes.

1. **Identity and discovery**
   - kind `0` profile: display name, about, picture;
   - NIP-OA `auth` owner attestation: cryptographic provenance;
   - kind `10100` agent directory record: capabilities, memberships, presence/status metadata, response policy, and channel-add policy;
   - room membership and Desktop eligibility predicates;
   - stable pubkey identity across aliases, restarts, and packaging.
2. **Conversation UX**
   - durable kind `9` chat messages;
   - ephemeral kind `20001` presence;
   - ephemeral kind `20002` typing/Working signal;
   - DM versus room/thread scope;
   - exact-mention and DM wake behavior.
3. **Activity and control**
   - owner-encrypted kind `24200` ACP/Hermes observer frames;
   - Desktop activity strip/sidebar presentation;
   - eventual reverse controls such as `cancel_turn` and `switch_model`.

Never infer one plane from another. A reply does not prove directory eligibility. A relay presence event does not prove the installed app rendered a current green dot. A typing publish ACK does not prove the correct DM composer showed `Working`.

## 3. Protocol/event contract

| Kind | Meaning here | Persistence / scope |
|---:|---|---|
| `0` | Public profile: name, about, picture | Replaceable profile record |
| `9` | Channel/DM message | Durable conversation content |
| `10100` | Agent directory metadata and policy | Replaceable discovery record |
| `20001` | Presence (`online`, `away`, `offline`) | Ephemeral, self-signed; relay TTL bounds stale-online behavior |
| `20002` | Typing/Working | Ephemeral; mandatory `h=<conversation UUID>`; optional NIP-10 `e` root/reply tags for real threads |
| `24200` | Owner-scoped encrypted observer telemetry/control | Ephemeral; NIP-44 v2 encrypted |
| `30315` | NIP-38 user status | Replaceable profile status; **not** rich ACP activity |

### Typing event

```json
{
  "kind": 20002,
  "tags": [["h", "<conversation-uuid>"]],
  "content": ""
}
```

Real room threads may add:

```json
["e", "<root-id>", "", "root"]
["e", "<parent-id>", "", "reply"]
```

For a DM, the transient Working signal belongs to the whole conversation even when the durable answer replies to a specific event. Adding reply tags to DM typing caused Desktop to classify it as thread-only and hide it from the main composer rail; the final refinement deliberately strips typing thread metadata for Buzz DMs while preserving genuine group-thread scope.

### Observer event

Kind `24200` is signed by the agent and NIP-44 v2 encrypted to the configured owner. Agent→owner tags are:

```json
[
  ["p", "<owner-pubkey>"],
  ["agent", "<agent-pubkey>"],
  ["frame", "telemetry"]
]
```

The plaintext envelope carries monotonically ordered `seq`, timestamp, agent/channel/session/turn identity, a typed activity kind, and bounded payload. Implemented activity kinds include:

- `turn_started`
- `tool_call`
- `tool_call_update`
- `acp_read`
- `acp_write`
- `turn_liveness`
- `turn_ending`
- `session_info_update`

The implementation paces and bounds publication so observer traffic cannot block the model turn. Oversized content is trimmed or represented by a bounded stub. Terminal ordering was hardened so `turn_ending` is not overtaken by queued updates; later patch work guarantees a terminal frame on completion, failure, stale result, or cancellation.

**Open boundary:** this is publish-only telemetry. Owner→agent `cancel_turn` and `switch_model` are not complete in the Hermes custom adapter. Unknown control frames must remain harmlessly ignored until signature, targeting, freshness, and verb validation are implemented.

## 4. Identity, ownership, and profile repair

### Stable identity

The stable identity is the full Nostr pubkey. Display name, avatar, persona, model, team, green dot, local card, and runtime label are presentation or runtime metadata—not identity proof.

This distinction mattered because Desktop deploy/import flows created duplicate agent records and duplicate responders. The repair policy became:

- preserve the canonical key and historical aliases;
- do not use a local ACP harness merely to obtain a card for an already-running remote Hermes agent;
- separate Desktop lifecycle ownership from relay profile/discovery;
- do not archive or delete a record until the replying signed-event pubkey is known;
- require exactly one canonical card and one responder after restart.

### Profile/directory writes

Onboarding uses three independent pieces:

1. publish kind `0` with complete display metadata;
2. carry the owner-issued NIP-OA `auth` tag;
3. publish complete kind `10100` with only verified room memberships and conservative policy (`channel_add_policy: owner_only` for canaries).

A sparse directory/policy update had been able to erase name, picture, memberships, or other profile fields. Branch `fix/preserve-agent-profile-policy-update` fixes the CLI path to read and preserve the complete current profile when changing policy.

The relay owner-materialization branch repairs direct-agent ownership visibility when NIP-OA authorizes the identity. This supports the Desktop `managed by`/owner provenance surface; it does not replace the signed owner attestation.

### Directory fallback work

The larger `fix/agent-directory-kind10100-fallback` worktree explores complete relay-agent conversion and parity across:

- Desktop autocomplete and Members sidebar;
- Tauri relay-agent conversion and types;
- CLI agent profile/policy paths;
- Desktop E2E bridge and channel tests;
- mobile mention candidates/people search.

That worktree remains dirty and is **not** the authority for the now-merged picker repair. Its useful conclusion was narrower and now implemented on fork main: a relay agent who is reachable through current room membership should not disappear simply because optional directory metadata is sparse.

## 5. Presence and external-lifecycle truth

Hermes adapter work added native self-signed kind `20001` presence publication. Presence is distinct from kind `30315` status and from process bookkeeping.

The operational contract is:

- publish `online` after authenticated relay connectivity;
- refresh before relay expiry;
- publish `offline` on graceful shutdown when possible;
- accept a bounded stale-online window after hard process/node failure;
- keep credentials and owner attestation fail-closed;
- never equate a stored `backend_agent_id`, local process, or Desktop Start button with live presence.

Buzz remote-agent design uses presence as the post-deploy liveness signal because Desktop intentionally has no substrate management channel after deployment. The relay-wide presence TTL is documented as 180 seconds; therefore user-configurable suppression such as `BUZZ_ACP_NO_PRESENCE` cannot be allowed to silently defeat external-lifecycle truth.

What was proven historically: native presence publication and authenticated relay receipt existed. What still requires fresh installed-client acceptance per agent: visible online→offline transitions, timeout behavior, and restart durability. Historical green dots are not recorded as current PASS.

## 6. Discoverability and picker work

There are several different discovery surfaces, each with different predicates:

1. Agents directory/profile surface;
2. New DM recipient picker;
3. composer `@` autocomplete;
4. Add people / Add members;
5. existing-room member list;
6. owner provenance badge.

Earlier fixes showed owner-bound relay agents in the channel picker and kept active room agents mentionable. The final merged fork-main repair broadens the rule safely:

- candidate is a known relay/managed agent;
- candidate is reachable in the current room or DM context;
- mention and add-people predicates no longer require unrelated optional metadata when live membership establishes reachability;
- tests cover a reachable member relay agent in the picker;
- authorization at the send boundary remains separate and must still reject unauthorized mentions.

This avoids two opposite failures: hiding valid remote Hermes agents, and exposing arbitrary relay identities as invocable agents.

**Acceptance remains surface-specific.** A locally managed agent can autocomplete without kind `10100`; that does not prove relay-directory onboarding. The acceptance set is name search, New DM, `@` insertion, Add people, owner badge, and a real addressed reply from the same canonical pubkey.

## 7. DMs, rooms, invitations, and the Hermes “awake” path

Buzz membership alone was insufficient. An agent could appear in a room while its gateway had no active subscription, so it never saw messages.

Hermes work addressed this in layers:

- classify DMs from participant (`p`) tags so unmentioned 1:1 messages dispatch;
- keep `require_mention` for shared rooms but not 1:1 DMs;
- add authenticated WebSocket inbound transport with NIP-42, live DM discovery, and polling fallback;
- listen for membership events addressed to the agent;
- rediscover rooms/DMs and open a channel subscription without gateway restart;
- start the new subscription at the membership-event boundary so the first immediate mention is not misclassified as old history;
- preserve per-conversation session routing and dedupe.

The committed dynamic-room branch proves the new-subscription behavior. The attached uncommitted patch adds the stricter invitation-boundary rule and regression test.

### Fixed list, allow/“loud list,” and dynamic discovery

Three controls were previously conflated:

- **Membership:** relay says the pubkey belongs to the room.
- **Subscription:** the running Hermes adapter is actively receiving the room.
- **Response policy:** after receipt, the gateway decides whether DM/plain text, exact mention, sender allowlist, and room policy permit a response.

A fixed configured room list can make an agent awake there from startup. Dynamic membership discovery is needed for rooms joined later. Neither one replaces sender/mention policy. The durable target is reconciliation—membership should continuously converge with active subscriptions, rather than relying only on a single in-memory event and a restart.

## 8. Response policy and exactly-once coordination

The desired behavior is:

- 1:1 DM from an allowed sender: wake without an `@mention`;
- shared room: require an exact canonical mention unless explicitly configured otherwise;
- self-authored events: ignore;
- unauthorized senders/rooms: stay silent;
- one accepted event: one responder and one durable answer;
- steer/redirect during an active turn: preserve the user instruction without noisy duplicate chat acknowledgements.

The Hermes gateway changes suppress periodic heartbeat/progress chat only after native Working/activity replacement exists. Both busy **steer** and successful active-turn **redirect** paths must share the suppression behavior; failures or real interrupts retain user-visible acknowledgement.

Branch `feat/room-response-coordination-cas` adds a transactional room-response lease primitive:

- DB schema/migration and coordination model;
- relay API endpoints;
- CLI commands/client support;
- compare-and-set/transactional ownership of a response slot;
- documentation and extensive tests in the branch.

It is a safety primitive for multi-agent rooms, not a substitute for identity, membership, or mention policy. It remains unmerged and should receive concurrency/security review before deployment.

## 9. Native typing / Working lifecycle

The Hermes branch implements canonical signed kind `20002` emission around a real model turn:

1. send immediately after the event passes dispatch/policy gates;
2. refresh while the turn is active;
3. reuse an authenticated NIP-42 WebSocket;
4. serialize concurrent sends with a lock;
5. correlate the exact relay `OK` event ID;
6. reconnect stale sockets;
7. close half-authenticated sockets on cancellation;
8. treat typing as best-effort so a relay failure never fails the answer;
9. stop/clear on success, failure, stale result, redirect, and cancellation;
10. preserve thread root/reply tags only for genuine room threads;
11. keep DM Working conversation-scoped.

Hardening covered the pinned `websockets` API (`state`/`OPEN` rather than assuming `.closed`) and the Python cancellation rule that `asyncio.CancelledError` is a `BaseException`.

The strongest focused receipt recorded for the final DM/thread repair was **65/65 tests passing**, including top-level DM, reply-in-DM, genuine thread preservation, and cleanup metadata. Earlier targeted suites and Desktop source tests also passed. This is not promoted to fleet-wide installed-client PASS: Sigma was the canary, and reply-to-reply/cleanup plus serial rollout remained open.

## 10. ACP sidebar / activity strip

Buzz already contained a native local ACP observer and activity UI lineage:

- `7c2c556d7` — local ACP session observer;
- `968691806` — live ACP session sidebar with sticky scroll;
- `e8126b3d0` — relay agents in channel activity;
- `aad564b13` — activity below the composer;
- `fd5d04de0` / `3d8b027ca` / `1f5ba5bb2` — active-channel and session/liveness scoping;
- `d8f9d87c1` / `cf838cef9` — activity layout and dock geometry;
- `654b6c374` / `915b5290d` — profile/sidebar and pop-out activity surfaces.

The work in this handoff did **not** invent that sidebar. It made the custom Hermes runtime speak the activity protocol the Buzz UI expects and repaired DM classification/rendering so external relay agents can use the same presentation.

Desktop DM activity changes:

- recognize a known managed/relay agent who participates in the active DM without promoting arbitrary DM participants;
- unify typing and Working in one left-aligned lane below the composer;
- render across the ordinary DM, Inbox-derived DM, and thread/reply surfaces where the source permits;
- keep channel behavior unchanged;
- retain kind `24200` telemetry as a separate acceptance gate.

The packaged-team preview branch contains these changes, but current acceptance was strongest for an ordinary full DM. Inbox, focused reply-to-reply DM, room thread placement, and final installed-client fleet parity were not all accepted at the same time.

## 11. Runtime discovery, macOS packaging, and registry safety

Several visible Buzz failures were not relay problems:

- GUI-launched Buzz had a minimal `PATH`, so Claude Code installed through shell initialization could be shown as “not installed” in Edit while Start still found it;
- ad-hoc/test build identity and Keychain access groups could make restored JSON cards unable to recover their private identities;
- duplicate/stale managed-agent registry entries created misleading cards and responders.

Source work included:

- refresh/force runtime catalog at action boundaries;
- search interactive login-shell initialization for runtimes;
- unsigned/ad-hoc macOS backport preview workflows (signing is not a blocker for internal Buzz backports);
- reversible raw-registry quarantine with source hash, backup, manifest, atomic rename, fsync, unknown-field preservation, and byte-perfect rollback intent.

Important review blockers remain:

- login-shell discovery test did not initially distinguish the requested command from an absent command;
- interactive shells can execute user dotfiles, so timeout/output/absolute-path validation remains security-sensitive;
- quarantine manifest paths needed recomputation rather than trusting stored result hashes;
- quarantine needed an exclusive lock and final revalidation to close TOCTOU/concurrency gaps.

No live registry migration is authorized from that branch as-is. Repeated unexpected Keychain prompts are a stop condition: cancel/deny, enter no password, and preserve the installed rollback app.

## 12. Internal ingress repair

Buzz also suffered an infrastructure outage independent of agent feature work. The internal route and certificate files were missing from both live Traefik config trees. The recovery:

- took rollback snapshots of both Traefik instances;
- restored matching route and valid certificate files from the retained working backup;
- verified each node and the VIP returned trusted TLS plus readiness;
- executed controlled VIP failover and failback;
- confirmed Hermes stopped seeing TLS failures and received Buzz activity again.

This proves the ingress/control plane repair. End-to-end product acceptance still requires a real signed agent reply in Buzz.

## 13. Verification and rollout ledger

Status words used here:

- **PASS:** current user-visible acceptance, or a defined machine gate where no UI exists;
- **PARTIAL:** source/config/backend/historical proof only;
- **FAIL:** current observed failure;
- **UNKNOWN:** not freshly measured;
- **NOT STARTED:** deliberately gated.

| Surface | Evidence | Status as of handoff | Exact next gate |
|---|---|---|---|
| Canonical identity/profile | Stable-key policy and profile/directory repair paths documented | PARTIAL | Read back signed kind `0`, `auth`, and kind `10100`; confirm replying pubkey |
| New DM discovery | Picker repairs implemented; broader reachable-agent fix merged to fork main | PARTIAL | Installed Desktop New DM search for each canonical role |
| Composer `@` autocomplete | Reachable member relay-agent test merged | PARTIAL | Insert canonical mention in live room and send successfully |
| Add people/member eligibility | Members sidebar predicate repaired | PARTIAL | Live Add people with correct owner label and no duplicate card |
| Presence | Hermes kind `20001` implementation exists | PARTIAL | Observe online→offline→restart transition in installed Desktop |
| Typing/Working | Hermes branch + focused 65/65 receipt; Desktop DM rendering branch | PARTIAL | Live ordinary DM, reply-to-reply DM, room, and room-thread matrix |
| ACP observer/sidebar | kind `24200` publisher and Buzz UI lineage exist | PARTIAL | Decrypt subscriber receipt, then installed-sidebar rendering for the same turn |
| Reverse ACP control | Contract known | NOT STARTED | Implement/verify owner signature, target, freshness, `cancel_turn`/`switch_model` |
| DM response | Strong prior real replies | PARTIAL | Fresh unmentioned allowed-sender DM, exactly once, canonical pubkey |
| Room wake after invite | Dynamic subscription branch pushed | PARTIAL | Invite after startup, immediate first mention, no restart, exactly once |
| Restart durability | Mixed historical evidence | UNKNOWN | Restart app + gateway; no duplicate resurrection; same identity and subscriptions |
| Fleet rollout | Sigma-first plan; other agents deliberately gated | NOT STARTED | Full Sigma matrix PASS, then one agent at a time |

Fleet sequence remains: **Sigma canary → Tracer → Baptiste → Reinhardt → Winston → D.Va**, one agent at a time. Canonical Bastion, Mercy, Road Hog, and Zenyatta card acceptance is also identity-specific; do not infer it from a duplicate card or a similarly named responder.

## 14. Repositories, branches, and retained artifacts

### Buzz repo (`BluePaladinLLC/buzz`)

| Branch | Head / key commit | Purpose |
|---|---|---|
| `main` | `a7c5f97f6` at handoff branch point | Merged remote-Hermes picker repair |
| `docs/buzz-integration-handoff` | this document | Canonical index and review artifacts |
| `feat/room-response-coordination-cas` | `824b0dd34` | Transactional room response coordination |
| `fix/preserve-agent-profile-policy-update` | `0c3c64eb7` | Preserve complete kind `10100` data during policy mutation |
| `fix/nip-oa-direct-member-owner-materialization` | `fa6d8c34e` | Relay owner provenance for direct agents |
| `fix/winston-room-mention-urgent` | `9c8b5cca3` | Earlier active-room mention eligibility repair |
| `fix/dm-agent-activity` | `81797e6cd` | Desktop DM activity and placement tests |
| `preview/dm-agent-activity-team` | `9b54683ae` | Team preview carrying DM activity changes |
| `repair/desktop-v0.5.20-dm-safety` | `38561f810` plus DM commits in history | Backport and unsigned macOS preview lane |
| `preview/desktop-v0.5.20-local-fixes` | `0114a9e4f` | Runtime discovery and reversible registry quarantine research |

### Hermes repo (`BluePaladinLLC/hermes-agent`)

| Branch | Head / key commit | Purpose |
|---|---|---|
| `feat/buzz-native-typing-current` | `12540de408` | Current committed native typing + owner-encrypted activity stack |
| `feat/buzz-room-policy-canary` | `466041b22d` | Subscribe to newly joined rooms |
| `fix/buzz-dynamic-room-subscription` | `b76ddf8015` | Earlier equivalent dynamic-room lane |

Key Hermes history in the typing/activity stack:

- `83478cd754` — emit native typing activity;
- `952aa00c48` — preserve thread scope for typing;
- `b82dbe6c9c` — harden thread typing lifecycle;
- `e7d36b0244` — publish owner-encrypted activity telemetry;
- `d0c65a7d5e` — preserve activity terminal ordering;
- `25c47fbf55` — allow serialized typing egress;
- `12540de408` — bound activity telemetry publication.

Earlier adapter foundation:

- `66fc2e2a92` — initial Buzz platform adapter;
- `ffb38f0c03` — `require_mention` support;
- `7e1e3b92f8` — DM classification from `p` tags;
- `1b9377b1fd` — scoped identity lock, name caching, docs-sidebar registration;
- `07e931fcb4` — authenticated WebSocket inbound transport, live DM discovery, polling fallback;
- `27cebc1954` — first native typing implementation lineage.

### Attached review artifacts in this branch

- `docs/handoffs/buzz-fleet-operability-scorecard-2026-08.md`
- `docs/handoffs/buzz-fleet-calibration-plan-2026-08.md`
- `docs/handoffs/buzz-typing-and-cli-activity-plan-2026-08-16.md`
- `docs/handoffs/buzz-hermes-agent-onboarding-2026-08.md`
- `docs/handoffs/hermes-buzz-typing-current-uncommitted-2026-08-28.patch.b64`

The base64 artifact decodes to an exact patch snapshot of the remaining uncommitted Hermes refinement (SHA-256 `e49efa792bd67c744f8ee0f6105a080964210b98c8c0173b9ff1129fa8eb8891`) and includes:

- DM typing metadata separated from durable reply metadata;
- terminal activity emission/cancellation cleanup;
- dynamic conversation subscription from the invitation boundary;
- focused regressions for DM/group typing scope and membership discovery.

It is preserved for review rather than silently committed from a dirty worktree. Decode it with `base64 -d hermes-buzz-typing-current-uncommitted-2026-08-28.patch.b64 > /tmp/hermes-buzz.patch`, then review or apply `/tmp/hermes-buzz.patch` against commit `12540de408`.

## 15. What Opus should inspect first

1. **Apply/review the attached Hermes patch** against `feat/buzz-native-typing-current`; focus on cancellation, terminal ordering, stale-turn behavior, and whether the membership-event timestamp is the correct first-message boundary.
2. **Review `feat/room-response-coordination-cas`** for transaction isolation, lease ownership, expiry/reclaim behavior, authorization, and replay/idempotency.
3. **Compare Desktop DM activity branches with current fork main** and decide whether the DM rendering fixes should be rebased as a narrow current-main PR.
4. **Run the multi-surface acceptance matrix**, not isolated pings: canonical identity, New DM, autocomplete, Add people, presence, Working, sidebar activity, room wake, exactly-once response, and restart durability.

## 16. Guardrails

- Never publish or commit private keys, auth tokens, Keychain contents, or private infrastructure locators.
- Do not replace a durable Hermes identity with a Desktop-created duplicate merely to gain a card.
- Do not call relay ACK/backend receipt end-to-end acceptance.
- Do not roll the fleet until Sigma passes the complete installed-client matrix.
- Preserve rollback before a new macOS build or meaningful managed-agent registry change.
- Unexpected Keychain prompts: deny/cancel, enter no password, stop automation.
- Do not execute the registry quarantine branch until the manifest-trust and concurrency review findings are fixed and independently verified.

---

**Bottom line:** the work is no longer scattered only across sessions and local worktrees. This document is the canonical map; the committed source branches are on GitHub; the two previously local-only branches were pushed; and the final dirty Hermes refinement is preserved byte-for-byte as a reviewable patch rather than being lost or misrepresented as tested/merged.
