# Room response coordination RPC

`POST /api/room-coordination` is the relay authority for deciding which runtime
may produce a room response. It is intentionally a narrow exception to Buzz's
Nostr-first guidance: claim, renewal, expired-lease takeover, contribution
budgeting, and finalization require a synchronous compare-and-swap result.
Generic event ingest can store competing signed events but cannot honestly tell
two concurrent callers that exactly one acquired the same expired lease.

The RPC remains Nostr-native at the security boundary:

- The community is resolved exclusively from the request `Host`.
- Every body is covered by a NIP-98 `payload` tag and signed by the claimant.
- The claimant pubkey is implicit in that event and is never accepted in JSON.
- The NIP-98 replay guard, HTTP admission, relay membership, and direct active
  channel membership gates all fail closed.
- The signed NIP-98 event ID and authenticated claimant are stored with durable
  request and contribution audit rows.

## Scope and state

The primary key is `(community_id, channel_id, thread_id, turn_id)`. Versions
start at 1 and increase for every successful mutation. Mutations require
`expected_version` (`0` for an initial claim). Lease duration is bounded to
5–300 seconds; contribution budget is bounded to 0–64 and fixed for the
lifetime of the turn, including primary takeover.

Operations are `claim`, `renew`, `contribute`, `finalize`, and `read`.
PostgreSQL transaction-scoped advisory locks serialize each complete scope
before state is read, so concurrent first claims and expired-lease takeovers
have exactly one winner. Contributions claim a unique fingerprint and consume
the one-turn deputy budget without changing the primary holder.

Only the primary holder may renew or finalize. Finalization is permanent and
blocks all later claim, renewal, and contribution attempts for that turn. There
is no `reopen` mutation because current runtime identities do not reliably
prove a fresh human trigger. A fresh human trigger uses a **new `turn_id`**,
which is independent and open while preserving the finalized turn's audit
history.

## Idempotency and replay

NIP-98 transport events are one-use: replaying the exact auth event is rejected.
A transport retry signs a fresh NIP-98 event but sends the same body. Every
mutation body therefore includes a UUID `request_id`. The relay stores the
claimant, a canonical payload digest, NIP-98 event ID, and response. Repeating
the exact `(community, request_id, claimant, payload)` returns the original
state without incrementing the version. Reusing that request ID with different
identity or payload returns HTTP 409.

Other CAS conflicts, live-holder conflicts, duplicate fingerprints, exhausted
budgets, and finalized turns return HTTP 409. A non-holder holder-only mutation
returns HTTP 403. Validation errors return HTTP 400. Database, auth, admission,
and membership failures never fall back to process-local state.

## CLI

```sh
buzz room-coordination claim --channel UUID --thread THREAD --turn TURN \
  --expected-version 0 --lease-seconds 30 --contribution-budget 1
buzz room-coordination renew --channel UUID --thread THREAD --turn TURN \
  --expected-version 1 --lease-seconds 30
buzz room-coordination contribute --channel UUID --thread THREAD --turn TURN \
  --expected-version 2 --fingerprint SHA256
buzz room-coordination finalize --channel UUID --thread THREAD --turn TURN \
  --expected-version 3 --final-event EVENT_ID
buzz room-coordination read --channel UUID --thread THREAD --turn TURN
```

The CLI generates a `request_id` unless `--request-id UUID` is supplied. A
caller retrying after an ambiguous transport outcome must reuse the same ID.
