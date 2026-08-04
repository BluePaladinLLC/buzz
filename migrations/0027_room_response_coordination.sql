-- Relay-authoritative room response coordination.
--
-- This is intentionally durable transactional state rather than replaceable
-- Nostr events: claim/takeover and contribution admission require a synchronous
-- compare-and-swap answer. Generic event ingest cannot truthfully tell two
-- concurrent writers that exactly one acquired an expired lease.
--
-- A finalized turn is immutable. Reopening means using a new turn_id; the old
-- turn and its audit trail remain closed.
CREATE TABLE room_coordination_turns (
    community_id UUID NOT NULL REFERENCES communities(id),
    channel_id UUID NOT NULL,
    thread_id TEXT NOT NULL CHECK (length(thread_id) BETWEEN 1 AND 255),
    turn_id TEXT NOT NULL CHECK (length(turn_id) BETWEEN 1 AND 255),
    version BIGINT NOT NULL CHECK (version > 0),
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'finalized')),
    holder_pubkey BYTEA NOT NULL CHECK (length(holder_pubkey) = 32),
    lease_expires_at TIMESTAMPTZ NOT NULL,
    contribution_budget INTEGER NOT NULL CHECK (contribution_budget BETWEEN 0 AND 64),
    contribution_count INTEGER NOT NULL DEFAULT 0 CHECK (contribution_count >= 0),
    final_event_id TEXT CHECK (
        final_event_id IS NULL OR length(final_event_id) BETWEEN 1 AND 255
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finalized_at TIMESTAMPTZ,
    last_request_id UUID NOT NULL,
    last_nip98_event_id BYTEA NOT NULL CHECK (length(last_nip98_event_id) = 32),
    CHECK ((status = 'open' AND finalized_at IS NULL AND final_event_id IS NULL)
        OR (status = 'finalized' AND finalized_at IS NOT NULL)),
    PRIMARY KEY (community_id, channel_id, thread_id, turn_id),
    FOREIGN KEY (community_id, channel_id)
        REFERENCES channels (community_id, id) ON DELETE CASCADE
);

CREATE TABLE room_coordination_contributions (
    community_id UUID NOT NULL,
    channel_id UUID NOT NULL,
    thread_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL CHECK (length(fingerprint) BETWEEN 1 AND 255),
    claimant_pubkey BYTEA NOT NULL CHECK (length(claimant_pubkey) = 32),
    request_id UUID NOT NULL,
    nip98_event_id BYTEA NOT NULL CHECK (length(nip98_event_id) = 32),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, channel_id, thread_id, turn_id, fingerprint),
    FOREIGN KEY (community_id, channel_id, thread_id, turn_id)
        REFERENCES room_coordination_turns
        (community_id, channel_id, thread_id, turn_id) ON DELETE CASCADE
);

-- Durable idempotency ledger. The digest covers the canonical request payload;
-- reusing a request_id with different bytes or identity is a conflict.
CREATE TABLE room_coordination_requests (
    community_id UUID NOT NULL REFERENCES communities(id),
    request_id UUID NOT NULL,
    claimant_pubkey BYTEA NOT NULL CHECK (length(claimant_pubkey) = 32),
    request_digest BYTEA NOT NULL CHECK (length(request_digest) = 32),
    nip98_event_id BYTEA NOT NULL CHECK (length(nip98_event_id) = 32),
    response JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, request_id)
);

CREATE INDEX room_coordination_expiry_idx
    ON room_coordination_turns (lease_expires_at) WHERE status = 'open';
