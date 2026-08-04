//! Transactional room-response coordination authority.
//!
//! Every mutation takes a transaction-scoped advisory lock for the complete
//! `(community, channel, thread, turn)` scope before reading state. This makes
//! first claim and expired-lease takeover atomic across relay processes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use buzz_core::CommunityId;

use crate::{DbError, Result};

/// Minimum primary lease duration accepted by the authority.
pub const MIN_LEASE_SECONDS: u32 = 5;
/// Maximum primary lease duration accepted by the authority.
pub const MAX_LEASE_SECONDS: u32 = 300;
/// Maximum deputy contribution budget for one turn.
pub const MAX_CONTRIBUTION_BUDGET: u32 = 64;

/// Community-local room response scope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoomScope {
    /// Channel containing the conversation.
    pub channel_id: Uuid,
    /// Stable thread identifier supplied by the room runtime.
    pub thread_id: String,
    /// Human-trigger/turn identifier. A new value opens an independent turn.
    pub turn_id: String,
}

/// Mutation requested against a room turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum CoordinationOperation {
    /// Acquire an absent or expired primary lease.
    Claim {
        /// Requested lease duration.
        lease_seconds: u32,
        /// Maximum accepted deputy contributions for this turn.
        contribution_budget: u32,
    },
    /// Extend the current holder's lease.
    Renew {
        /// Requested lease duration.
        lease_seconds: u32,
    },
    /// Atomically admit one deputy contribution fingerprint.
    Contribute {
        /// Caller-defined duplicate-suppression fingerprint.
        fingerprint: String,
    },
    /// Permanently close the turn.
    Finalize {
        /// Optional durable event identifier for the final response.
        final_event_id: Option<String>,
    },
}

/// Complete mutation input. Claimant identity is supplied by verified auth,
/// never deserialized from the caller's JSON body by the relay.
#[derive(Debug, Clone)]
pub struct CoordinationMutation {
    /// Target scope.
    pub scope: RoomScope,
    /// Required compare-and-swap version (`0` for first claim).
    pub expected_version: i64,
    /// Durable client idempotency key.
    pub request_id: Uuid,
    /// NIP-98-authenticated claimant public key bytes.
    pub claimant_pubkey: [u8; 32],
    /// Relay member whose membership authorizes this claimant. This is the
    /// claimant for direct membership or the verified NIP-OA owner.
    pub relay_authorizer_pubkey: [u8; 32],
    /// Whether this deployment requires relay membership for the mutation.
    pub require_relay_membership: bool,
    /// Signed NIP-98 request event ID.
    pub nip98_event_id: [u8; 32],
    /// Requested operation.
    pub operation: CoordinationOperation,
}

/// Current authoritative turn state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoordinationState {
    /// Target scope.
    pub scope: RoomScope,
    /// Monotonically increasing CAS version.
    pub version: i64,
    /// `open` or `finalized`.
    pub status: String,
    /// Current/last primary holder as lowercase hex.
    pub holder_pubkey: String,
    /// Lease expiry for an open turn.
    pub lease_expires_at: DateTime<Utc>,
    /// Configured deputy contribution budget.
    pub contribution_budget: u32,
    /// Number of admitted unique contribution fingerprints.
    pub contribution_count: u32,
    /// Final response event identifier, when supplied.
    pub final_event_id: Option<String>,
    /// Finalization timestamp.
    pub finalized_at: Option<DateTime<Utc>>,
    /// Last mutation timestamp.
    pub updated_at: DateTime<Utc>,
}

/// Stable conflict categories returned by the authority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoordinationConflict {
    /// The expected version is not current.
    VersionMismatch,
    /// Another live primary lease exists.
    LeaseHeld,
    /// The holder's primary lease has already expired.
    LeaseExpired,
    /// Only the primary holder may perform the operation.
    NotHolder,
    /// The turn is permanently finalized.
    Finalized,
    /// The fingerprint was already admitted.
    DuplicateFingerprint,
    /// The turn's contribution budget is exhausted.
    ContributionBudgetExhausted,
    /// The request id was reused with another identity or payload.
    RequestIdReused,
    /// A mutation requiring existing turn state found none.
    TurnNotFound,
}

/// Result of a coordination mutation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum CoordinationResult {
    /// Mutation committed.
    Applied {
        /// State after the committed mutation.
        state: CoordinationState,
    },
    /// Exact request-id replay; the original response is returned unchanged.
    Idempotent {
        /// State returned by the original request.
        state: CoordinationState,
    },
    /// Mutation was rejected without changing state.
    Conflict {
        /// Machine-readable reason.
        reason: CoordinationConflict,
        /// Current state, if the turn exists.
        state: Option<CoordinationState>,
    },
}

fn validate(mutation: &CoordinationMutation) -> Result<()> {
    for (name, value) in [
        ("thread_id", mutation.scope.thread_id.as_str()),
        ("turn_id", mutation.scope.turn_id.as_str()),
    ] {
        if value.is_empty() || value.len() > 255 {
            return Err(DbError::InvalidData(format!(
                "{name} must contain 1..=255 bytes"
            )));
        }
    }
    if mutation.expected_version < 0 {
        return Err(DbError::InvalidData(
            "expected_version must be non-negative".into(),
        ));
    }
    match &mutation.operation {
        CoordinationOperation::Claim {
            lease_seconds,
            contribution_budget,
        } => {
            validate_lease(*lease_seconds)?;
            if *contribution_budget > MAX_CONTRIBUTION_BUDGET {
                return Err(DbError::InvalidData(format!(
                    "contribution_budget must be at most {MAX_CONTRIBUTION_BUDGET}"
                )));
            }
        }
        CoordinationOperation::Renew { lease_seconds } => validate_lease(*lease_seconds)?,
        CoordinationOperation::Contribute { fingerprint } => {
            if fingerprint.is_empty() || fingerprint.len() > 255 {
                return Err(DbError::InvalidData(
                    "fingerprint must contain 1..=255 bytes".into(),
                ));
            }
        }
        CoordinationOperation::Finalize { final_event_id } => {
            if final_event_id
                .as_ref()
                .is_some_and(|id| id.is_empty() || id.len() > 255)
            {
                return Err(DbError::InvalidData(
                    "final_event_id must contain 1..=255 bytes when supplied".into(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_lease(seconds: u32) -> Result<()> {
    if !(MIN_LEASE_SECONDS..=MAX_LEASE_SECONDS).contains(&seconds) {
        return Err(DbError::InvalidData(format!(
            "lease_seconds must be between {MIN_LEASE_SECONDS} and {MAX_LEASE_SECONDS}"
        )));
    }
    Ok(())
}

fn request_digest(mutation: &CoordinationMutation) -> Result<[u8; 32]> {
    #[derive(Serialize)]
    struct DigestBody<'a> {
        scope: &'a RoomScope,
        expected_version: i64,
        operation: &'a CoordinationOperation,
    }
    let bytes = serde_json::to_vec(&DigestBody {
        scope: &mutation.scope,
        expected_version: mutation.expected_version,
        operation: &mutation.operation,
    })?;
    Ok(Sha256::digest(bytes).into())
}

async fn lock_scope(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    scope: &RoomScope,
) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!(
            "buzz_room_coordination:{}:{}:{}:{}",
            community_id.as_uuid(),
            scope.channel_id,
            scope.thread_id,
            scope.turn_id
        ))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn row_to_state(row: sqlx::postgres::PgRow) -> Result<CoordinationState> {
    let budget: i32 = row.try_get("contribution_budget")?;
    let count: i32 = row.try_get("contribution_count")?;
    Ok(CoordinationState {
        scope: RoomScope {
            channel_id: row.try_get("channel_id")?,
            thread_id: row.try_get("thread_id")?,
            turn_id: row.try_get("turn_id")?,
        },
        version: row.try_get("version")?,
        status: row.try_get("status")?,
        holder_pubkey: hex::encode(row.try_get::<Vec<u8>, _>("holder_pubkey")?),
        lease_expires_at: row.try_get("lease_expires_at")?,
        contribution_budget: u32::try_from(budget)
            .map_err(|_| DbError::InvalidData("negative contribution budget in database".into()))?,
        contribution_count: u32::try_from(count)
            .map_err(|_| DbError::InvalidData("negative contribution count in database".into()))?,
        final_event_id: row.try_get("final_event_id")?,
        finalized_at: row.try_get("finalized_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

async fn read_locked(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    scope: &RoomScope,
) -> Result<Option<CoordinationState>> {
    sqlx::query(
        "SELECT channel_id, thread_id, turn_id, version, status, holder_pubkey, \
         lease_expires_at, contribution_budget, contribution_count, final_event_id, \
         finalized_at, updated_at FROM room_coordination_turns \
         WHERE community_id=$1 AND channel_id=$2 AND thread_id=$3 AND turn_id=$4 FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(scope.channel_id)
    .bind(&scope.thread_id)
    .bind(&scope.turn_id)
    .fetch_optional(&mut **tx)
    .await?
    .map(row_to_state)
    .transpose()
}

async fn updated_state(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    scope: &RoomScope,
) -> Result<CoordinationState> {
    read_locked(tx, community_id, scope)
        .await?
        .ok_or_else(|| DbError::InvalidData("coordination state disappeared in transaction".into()))
}

fn conflict(reason: CoordinationConflict, state: Option<CoordinationState>) -> CoordinationResult {
    CoordinationResult::Conflict { reason, state }
}

/// Atomically apply one room coordination mutation.
pub async fn mutate(
    pool: &PgPool,
    community_id: CommunityId,
    mutation: &CoordinationMutation,
) -> Result<CoordinationResult> {
    validate(mutation)?;
    let digest = request_digest(mutation)?;
    let mut tx = pool.begin().await?;

    // Membership writers use this same per-channel lock. Taking it as the
    // transaction's first statement means a revocation either commits before
    // these checks (and denies the mutation) or waits until this mutation has
    // committed. A relay-side precheck alone cannot provide that ordering.
    crate::channel::acquire_channel_membership_lock(
        &mut tx,
        community_id,
        mutation.scope.channel_id,
    )
    .await?;
    lock_scope(&mut tx, community_id, &mutation.scope).await?;

    let channel_member = sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM channel_members cm \
         JOIN channels c ON c.community_id=cm.community_id AND c.id=cm.channel_id \
         WHERE cm.community_id=$1 AND cm.channel_id=$2 AND cm.pubkey=$3 \
         AND cm.removed_at IS NULL AND c.deleted_at IS NULL \
         FOR SHARE OF cm, c",
    )
    .bind(community_id.as_uuid())
    .bind(mutation.scope.channel_id)
    .bind(mutation.claimant_pubkey.as_slice())
    .fetch_optional(&mut *tx)
    .await?
    .is_some();
    if !channel_member {
        return Err(DbError::AccessDenied(
            "active channel membership required".into(),
        ));
    }

    if mutation.require_relay_membership {
        let relay_member = sqlx::query_scalar::<_, i32>(
            "SELECT 1 FROM relay_members WHERE community_id=$1 AND pubkey=$2 FOR KEY SHARE",
        )
        .bind(community_id.as_uuid())
        .bind(hex::encode(mutation.relay_authorizer_pubkey))
        .fetch_optional(&mut *tx)
        .await?
        .is_some();
        if !relay_member {
            return Err(DbError::AccessDenied(
                "active relay membership required".into(),
            ));
        }
    }

    if let Some(row) = sqlx::query(
        "SELECT claimant_pubkey, request_digest, response FROM room_coordination_requests \
         WHERE community_id=$1 AND request_id=$2 FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(mutation.request_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        let same_claimant: Vec<u8> = row.try_get("claimant_pubkey")?;
        let same_digest: Vec<u8> = row.try_get("request_digest")?;
        let result = if same_claimant == mutation.claimant_pubkey && same_digest == digest {
            match serde_json::from_value::<CoordinationResult>(row.try_get("response")?)? {
                CoordinationResult::Applied { state }
                | CoordinationResult::Idempotent { state } => {
                    CoordinationResult::Idempotent { state }
                }
                other => other,
            }
        } else {
            conflict(
                CoordinationConflict::RequestIdReused,
                read_locked(&mut tx, community_id, &mutation.scope).await?,
            )
        };
        tx.commit().await?;
        return Ok(result);
    }

    // Read the PostgreSQL wall clock only after every potentially blocking
    // lock, including the community-scoped request-ledger row lock above.
    // Transaction time is fixed at BEGIN and can become stale while waiting
    // behind another mutation, membership writer, or same request id used by
    // another turn. Existing request rows return before a fresh time is needed.
    let db_now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut *tx)
        .await?;

    let current = read_locked(&mut tx, community_id, &mutation.scope).await?;
    let result = match (&mutation.operation, current) {
        (
            CoordinationOperation::Claim {
                lease_seconds,
                contribution_budget,
            },
            None,
        ) => {
            if mutation.expected_version != 0 {
                conflict(CoordinationConflict::VersionMismatch, None)
            } else {
                sqlx::query(
                    "INSERT INTO room_coordination_turns \
                     (community_id, channel_id, thread_id, turn_id, version, holder_pubkey, \
                      lease_expires_at, contribution_budget, last_request_id, last_nip98_event_id, \
                      created_at, updated_at) \
                     VALUES ($1,$2,$3,$4,1,$5,$10+make_interval(secs => $6),$7,$8,$9,$10,$10)",
                )
                .bind(community_id.as_uuid())
                .bind(mutation.scope.channel_id)
                .bind(&mutation.scope.thread_id)
                .bind(&mutation.scope.turn_id)
                .bind(mutation.claimant_pubkey.as_slice())
                .bind(i32::try_from(*lease_seconds).map_err(|_| {
                    DbError::InvalidData("lease_seconds does not fit database integer".into())
                })?)
                .bind(i32::try_from(*contribution_budget).map_err(|_| {
                    DbError::InvalidData("contribution_budget does not fit database integer".into())
                })?)
                .bind(mutation.request_id)
                .bind(mutation.nip98_event_id.as_slice())
                .bind(db_now)
                .execute(&mut *tx)
                .await?;
                CoordinationResult::Applied {
                    state: updated_state(&mut tx, community_id, &mutation.scope).await?,
                }
            }
        }
        (CoordinationOperation::Claim { .. }, Some(state)) if state.status == "finalized" => {
            conflict(CoordinationConflict::Finalized, Some(state))
        }
        (CoordinationOperation::Claim { .. }, Some(state))
            if mutation.expected_version != state.version =>
        {
            conflict(CoordinationConflict::VersionMismatch, Some(state))
        }
        (CoordinationOperation::Claim { .. }, Some(state)) if state.lease_expires_at > db_now => {
            conflict(CoordinationConflict::LeaseHeld, Some(state))
        }
        (
            CoordinationOperation::Claim {
                lease_seconds,
                contribution_budget: _,
            },
            Some(_),
        ) => {
            sqlx::query(
                "UPDATE room_coordination_turns SET version=version+1, holder_pubkey=$5, \
                 lease_expires_at=$9+make_interval(secs => $6), \
                 updated_at=$9, last_request_id=$7, last_nip98_event_id=$8 \
                 WHERE community_id=$1 AND channel_id=$2 AND thread_id=$3 AND turn_id=$4",
            )
            .bind(community_id.as_uuid())
            .bind(mutation.scope.channel_id)
            .bind(&mutation.scope.thread_id)
            .bind(&mutation.scope.turn_id)
            .bind(mutation.claimant_pubkey.as_slice())
            .bind(i32::try_from(*lease_seconds).map_err(|_| {
                DbError::InvalidData("lease_seconds does not fit database integer".into())
            })?)
            .bind(mutation.request_id)
            .bind(mutation.nip98_event_id.as_slice())
            .bind(db_now)
            .execute(&mut *tx)
            .await?;
            CoordinationResult::Applied {
                state: updated_state(&mut tx, community_id, &mutation.scope).await?,
            }
        }
        (_, None) => conflict(CoordinationConflict::TurnNotFound, None),
        (_, Some(state)) if state.status == "finalized" => {
            conflict(CoordinationConflict::Finalized, Some(state))
        }
        (_, Some(state)) if mutation.expected_version != state.version => {
            conflict(CoordinationConflict::VersionMismatch, Some(state))
        }
        (
            CoordinationOperation::Renew { .. } | CoordinationOperation::Finalize { .. },
            Some(state),
        ) if state.holder_pubkey != hex::encode(mutation.claimant_pubkey) => {
            conflict(CoordinationConflict::NotHolder, Some(state))
        }
        (
            CoordinationOperation::Renew { .. } | CoordinationOperation::Finalize { .. },
            Some(state),
        ) if state.lease_expires_at <= db_now => {
            conflict(CoordinationConflict::LeaseExpired, Some(state))
        }
        (CoordinationOperation::Renew { lease_seconds }, Some(_)) => {
            sqlx::query(
                "UPDATE room_coordination_turns SET version=version+1, \
                 lease_expires_at=$8+make_interval(secs => $5), \
                 updated_at=$8, \
                 last_request_id=$6, last_nip98_event_id=$7 \
                 WHERE community_id=$1 AND channel_id=$2 AND thread_id=$3 AND turn_id=$4",
            )
            .bind(community_id.as_uuid())
            .bind(mutation.scope.channel_id)
            .bind(&mutation.scope.thread_id)
            .bind(&mutation.scope.turn_id)
            .bind(i32::try_from(*lease_seconds).map_err(|_| {
                DbError::InvalidData("lease_seconds does not fit database integer".into())
            })?)
            .bind(mutation.request_id)
            .bind(mutation.nip98_event_id.as_slice())
            .bind(db_now)
            .execute(&mut *tx)
            .await?;
            CoordinationResult::Applied {
                state: updated_state(&mut tx, community_id, &mutation.scope).await?,
            }
        }
        (CoordinationOperation::Contribute { fingerprint }, Some(state)) => {
            let duplicate: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM room_coordination_contributions \
                 WHERE community_id=$1 AND channel_id=$2 AND thread_id=$3 AND turn_id=$4 \
                 AND fingerprint=$5)",
            )
            .bind(community_id.as_uuid())
            .bind(mutation.scope.channel_id)
            .bind(&mutation.scope.thread_id)
            .bind(&mutation.scope.turn_id)
            .bind(fingerprint)
            .fetch_one(&mut *tx)
            .await?;
            if duplicate {
                conflict(CoordinationConflict::DuplicateFingerprint, Some(state))
            } else if state.contribution_count >= state.contribution_budget {
                conflict(
                    CoordinationConflict::ContributionBudgetExhausted,
                    Some(state),
                )
            } else {
                sqlx::query(
                    "INSERT INTO room_coordination_contributions \
                     (community_id,channel_id,thread_id,turn_id,fingerprint,claimant_pubkey,request_id,nip98_event_id,created_at) \
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
                )
                .bind(community_id.as_uuid())
                .bind(mutation.scope.channel_id)
                .bind(&mutation.scope.thread_id)
                .bind(&mutation.scope.turn_id)
                .bind(fingerprint)
                .bind(mutation.claimant_pubkey.as_slice())
                .bind(mutation.request_id)
                .bind(mutation.nip98_event_id.as_slice())
                .bind(db_now)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE room_coordination_turns SET version=version+1, \
                     contribution_count=contribution_count+1, updated_at=$7, \
                     last_request_id=$5,last_nip98_event_id=$6 \
                     WHERE community_id=$1 AND channel_id=$2 AND thread_id=$3 AND turn_id=$4",
                )
                .bind(community_id.as_uuid())
                .bind(mutation.scope.channel_id)
                .bind(&mutation.scope.thread_id)
                .bind(&mutation.scope.turn_id)
                .bind(mutation.request_id)
                .bind(mutation.nip98_event_id.as_slice())
                .bind(db_now)
                .execute(&mut *tx)
                .await?;
                CoordinationResult::Applied {
                    state: updated_state(&mut tx, community_id, &mutation.scope).await?,
                }
            }
        }
        (CoordinationOperation::Finalize { final_event_id }, Some(_)) => {
            sqlx::query(
                "UPDATE room_coordination_turns SET version=version+1,status='finalized', \
                 final_event_id=$5,finalized_at=$8,updated_at=$8,last_request_id=$6, \
                 last_nip98_event_id=$7 \
                 WHERE community_id=$1 AND channel_id=$2 AND thread_id=$3 AND turn_id=$4",
            )
            .bind(community_id.as_uuid())
            .bind(mutation.scope.channel_id)
            .bind(&mutation.scope.thread_id)
            .bind(&mutation.scope.turn_id)
            .bind(final_event_id)
            .bind(mutation.request_id)
            .bind(mutation.nip98_event_id.as_slice())
            .bind(db_now)
            .execute(&mut *tx)
            .await?;
            CoordinationResult::Applied {
                state: updated_state(&mut tx, community_id, &mutation.scope).await?,
            }
        }
    };

    let response = serde_json::to_value(&result)?;
    sqlx::query(
        "INSERT INTO room_coordination_requests \
         (community_id,request_id,claimant_pubkey,request_digest,nip98_event_id,response,created_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(community_id.as_uuid())
    .bind(mutation.request_id)
    .bind(mutation.claimant_pubkey.as_slice())
    .bind(digest.as_slice())
    .bind(mutation.nip98_event_id.as_slice())
    .bind(response)
    .bind(db_now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(result)
}

/// Read one authoritative turn state from the writer pool.
pub async fn read(
    pool: &PgPool,
    community_id: CommunityId,
    scope: &RoomScope,
) -> Result<Option<CoordinationState>> {
    sqlx::query(
        "SELECT channel_id, thread_id, turn_id, version, status, holder_pubkey, \
         lease_expires_at, contribution_budget, contribution_count, final_event_id, \
         finalized_at, updated_at FROM room_coordination_turns \
         WHERE community_id=$1 AND channel_id=$2 AND thread_id=$3 AND turn_id=$4",
    )
    .bind(community_id.as_uuid())
    .bind(scope.channel_id)
    .bind(&scope.thread_id)
    .bind(&scope.turn_id)
    .fetch_optional(pool)
    .await?
    .map(row_to_state)
    .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Barrier;

    fn mutation(operation: CoordinationOperation) -> CoordinationMutation {
        CoordinationMutation {
            scope: RoomScope {
                channel_id: Uuid::new_v4(),
                thread_id: "thread".into(),
                turn_id: "turn".into(),
            },
            expected_version: 0,
            request_id: Uuid::new_v4(),
            claimant_pubkey: [1; 32],
            relay_authorizer_pubkey: [1; 32],
            require_relay_membership: true,
            nip98_event_id: [2; 32],
            operation,
        }
    }

    #[test]
    fn validation_bounds_leases_budgets_and_fingerprints() {
        assert!(validate(&mutation(CoordinationOperation::Claim {
            lease_seconds: MIN_LEASE_SECONDS,
            contribution_budget: MAX_CONTRIBUTION_BUDGET,
        }))
        .is_ok());
        assert!(validate(&mutation(CoordinationOperation::Claim {
            lease_seconds: MIN_LEASE_SECONDS - 1,
            contribution_budget: 1,
        }))
        .is_err());
        assert!(validate(&mutation(CoordinationOperation::Contribute {
            fingerprint: String::new(),
        }))
        .is_err());
    }

    async fn setup_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".into());
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect Postgres");
        crate::migration::run_migrations(&pool)
            .await
            .expect("run migrations");
        pool
    }

    async fn wait_for_membership_lock_wait(
        pool: &PgPool,
        community: CommunityId,
        channel_id: Uuid,
        blocker_pid: i32,
    ) {
        let lock_key = format!(
            "{}{}:{}",
            crate::channel::CHANNEL_MEMBERSHIP_LOCK_NAMESPACE,
            community.as_uuid(),
            channel_id
        );
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let waiting: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM pg_locks waiter \
                     WHERE waiter.locktype='advisory' AND NOT waiter.granted \
                     AND waiter.classid=((hashtextextended($1,0)::bigint >> 32) & 4294967295)::oid \
                     AND waiter.objid=(hashtextextended($1,0)::bigint & 4294967295)::oid \
                     AND $2=ANY(pg_blocking_pids(waiter.pid)))",
                )
                .bind(&lock_key)
                .bind(blocker_pid)
                .fetch_one(pool)
                .await
                .expect("observe scoped PostgreSQL membership-lock wait");
                if waiting {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("mutation must visibly wait on the expected membership lock holder");
    }

    async fn wait_for_request_ledger_lock_wait(pool: &PgPool) {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let waiting: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                     WHERE wait_event_type='Lock' \
                     AND query LIKE '%FROM room_coordination_requests%FOR UPDATE%')",
                )
                .fetch_one(pool)
                .await
                .expect("observe PostgreSQL request-ledger lock wait");
                if waiting {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("mutation must visibly wait on the request-ledger row lock");
    }

    async fn fixture(pool: &PgPool, channel_id: Uuid) -> (CommunityId, [u8; 32], [u8; 32]) {
        let community_uuid = Uuid::new_v4();
        let community = CommunityId::from_uuid(community_uuid);
        let holder = [7; 32];
        let deputy = [8; 32];
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(community_uuid)
            .bind(format!("room-{}.example", community_uuid.simple()))
            .execute(pool)
            .await
            .expect("insert community");
        sqlx::query("INSERT INTO channels (community_id,id,name,created_by) VALUES ($1,$2,$3,$4)")
            .bind(community_uuid)
            .bind(channel_id)
            .bind(format!("room-{}", community_uuid.simple()))
            .bind(holder.as_slice())
            .execute(pool)
            .await
            .expect("insert channel");
        for pubkey in [holder, deputy] {
            sqlx::query(
                "INSERT INTO channel_members (community_id,channel_id,pubkey,role) \
                 VALUES ($1,$2,$3,$4::member_role)",
            )
            .bind(community_uuid)
            .bind(channel_id)
            .bind(pubkey.as_slice())
            .bind(if pubkey == holder { "owner" } else { "member" })
            .execute(pool)
            .await
            .expect("insert channel member");
            sqlx::query(
                "INSERT INTO relay_members (community_id,pubkey,role) VALUES ($1,$2,'member')",
            )
            .bind(community_uuid)
            .bind(hex::encode(pubkey))
            .execute(pool)
            .await
            .expect("insert relay member");
        }
        (community, holder, deputy)
    }

    fn claim(
        scope: RoomScope,
        claimant: [u8; 32],
        expected_version: i64,
        request_id: Uuid,
        budget: u32,
    ) -> CoordinationMutation {
        CoordinationMutation {
            scope,
            expected_version,
            request_id,
            claimant_pubkey: claimant,
            relay_authorizer_pubkey: claimant,
            require_relay_membership: true,
            nip98_event_id: Sha256::digest(Uuid::new_v4().as_bytes()).into(),
            operation: CoordinationOperation::Claim {
                lease_seconds: MIN_LEASE_SECONDS,
                contribution_budget: budget,
            },
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn claim_cas_idempotency_expiry_and_concurrent_takeover() {
        let pool = setup_pool().await;
        let channel = Uuid::new_v4();
        let (community, holder, deputy) = fixture(&pool, channel).await;
        let scope = RoomScope {
            channel_id: channel,
            thread_id: "thread-a".into(),
            turn_id: "turn-a".into(),
        };
        let request_id = Uuid::new_v4();
        let first = claim(scope.clone(), holder, 0, request_id, 1);
        let CoordinationResult::Applied { state } =
            mutate(&pool, community, &first).await.expect("first claim")
        else {
            panic!("first claim must apply");
        };
        assert_eq!(state.version, 1);
        assert!(matches!(
            mutate(&pool, community, &first).await.expect("retry"),
            CoordinationResult::Idempotent { ref state } if state.version == 1
        ));

        let other = claim(scope.clone(), deputy, 1, Uuid::new_v4(), 1);
        assert!(matches!(
            mutate(&pool, community, &other)
                .await
                .expect("holder conflict"),
            CoordinationResult::Conflict {
                reason: CoordinationConflict::LeaseHeld,
                ..
            }
        ));
        let wrong_version = claim(scope.clone(), deputy, 99, Uuid::new_v4(), 1);
        assert!(matches!(
            mutate(&pool, community, &wrong_version)
                .await
                .expect("version conflict"),
            CoordinationResult::Conflict {
                reason: CoordinationConflict::VersionMismatch,
                ..
            }
        ));

        sqlx::query(
            "UPDATE room_coordination_turns SET lease_expires_at=now()-interval '1 second' \
             WHERE community_id=$1 AND channel_id=$2 AND thread_id=$3 AND turn_id=$4",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .bind(&scope.thread_id)
        .bind(&scope.turn_id)
        .execute(&pool)
        .await
        .expect("expire lease");

        let contender = [9; 32];
        sqlx::query(
            "INSERT INTO channel_members (community_id,channel_id,pubkey,role) \
             VALUES ($1,$2,$3,'member')",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .bind(contender.as_slice())
        .execute(&pool)
        .await
        .expect("insert contender channel membership");
        sqlx::query("INSERT INTO relay_members (community_id,pubkey,role) VALUES ($1,$2,'member')")
            .bind(community.as_uuid())
            .bind(hex::encode(contender))
            .execute(&pool)
            .await
            .expect("insert contender relay membership");

        let barrier = Arc::new(Barrier::new(3));
        let mut tasks = Vec::new();
        for claimant in [deputy, contender] {
            let pool = pool.clone();
            let barrier = Arc::clone(&barrier);
            let scope = scope.clone();
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                mutate(
                    &pool,
                    community,
                    &claim(scope, claimant, 1, Uuid::new_v4(), MAX_CONTRIBUTION_BUDGET),
                )
                .await
                .expect("concurrent takeover")
            }));
        }
        barrier.wait().await;
        let mut winners = 0;
        for task in tasks {
            if let CoordinationResult::Applied { state } = task.await.expect("join") {
                winners += 1;
                assert_eq!(
                    state.contribution_budget, 1,
                    "takeover must preserve the turn-wide contribution budget"
                );
            }
        }
        assert_eq!(winners, 1, "DB boundary must admit exactly one takeover");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn contribution_budget_finalize_new_turn_and_community_isolation() {
        let pool = setup_pool().await;
        let channel = Uuid::new_v4();
        let (community_a, holder, deputy) = fixture(&pool, channel).await;
        let (community_b, _, _) = fixture(&pool, channel).await;
        let scope = RoomScope {
            channel_id: channel,
            thread_id: "thread-b".into(),
            turn_id: "turn-b".into(),
        };
        assert!(matches!(
            mutate(
                &pool,
                community_a,
                &claim(scope.clone(), holder, 0, Uuid::new_v4(), 1)
            )
            .await
            .expect("claim A"),
            CoordinationResult::Applied { .. }
        ));
        assert!(matches!(
            mutate(
                &pool,
                community_b,
                &claim(scope.clone(), holder, 0, Uuid::new_v4(), 1)
            )
            .await
            .expect("claim B"),
            CoordinationResult::Applied { .. }
        ));

        let contribution = CoordinationMutation {
            scope: scope.clone(),
            expected_version: 1,
            request_id: Uuid::new_v4(),
            claimant_pubkey: deputy,
            relay_authorizer_pubkey: deputy,
            require_relay_membership: true,
            nip98_event_id: [11; 32],
            operation: CoordinationOperation::Contribute {
                fingerprint: "critical-risk:f1".into(),
            },
        };
        assert!(matches!(
            mutate(&pool, community_a, &contribution).await.expect("contribute"),
            CoordinationResult::Applied { ref state }
                if state.version == 2 && state.holder_pubkey == hex::encode(holder)
        ));
        let duplicate = CoordinationMutation {
            request_id: Uuid::new_v4(),
            expected_version: 2,
            ..contribution.clone()
        };
        assert!(matches!(
            mutate(&pool, community_a, &duplicate).await.expect("dedup"),
            CoordinationResult::Conflict {
                reason: CoordinationConflict::DuplicateFingerprint,
                ..
            }
        ));
        let over_budget = CoordinationMutation {
            request_id: Uuid::new_v4(),
            expected_version: 2,
            operation: CoordinationOperation::Contribute {
                fingerprint: "critical-risk:f2".into(),
            },
            ..contribution.clone()
        };
        assert!(matches!(
            mutate(&pool, community_a, &over_budget)
                .await
                .expect("budget"),
            CoordinationResult::Conflict {
                reason: CoordinationConflict::ContributionBudgetExhausted,
                ..
            }
        ));

        let finalize = CoordinationMutation {
            scope: scope.clone(),
            expected_version: 2,
            request_id: Uuid::new_v4(),
            claimant_pubkey: holder,
            relay_authorizer_pubkey: holder,
            require_relay_membership: true,
            nip98_event_id: [12; 32],
            operation: CoordinationOperation::Finalize {
                final_event_id: Some("event-final".into()),
            },
        };
        assert!(matches!(
            mutate(&pool, community_a, &finalize).await.expect("finalize"),
            CoordinationResult::Applied { ref state } if state.status == "finalized"
        ));
        let open_with_terminal_state = sqlx::query(
            "UPDATE room_coordination_turns SET status='open', finalized_at=clock_timestamp() \
             WHERE community_id=$1 AND channel_id=$2 AND thread_id=$3 AND turn_id=$4",
        )
        .bind(community_a.as_uuid())
        .bind(channel)
        .bind(&scope.thread_id)
        .bind(&scope.turn_id)
        .execute(&pool)
        .await;
        assert!(
            open_with_terminal_state.is_err(),
            "SQL must reject open turns carrying terminal state"
        );
        let finalized_without_timestamp = sqlx::query(
            "UPDATE room_coordination_turns SET finalized_at=NULL \
             WHERE community_id=$1 AND channel_id=$2 AND thread_id=$3 AND turn_id=$4",
        )
        .bind(community_a.as_uuid())
        .bind(channel)
        .bind(&scope.thread_id)
        .bind(&scope.turn_id)
        .execute(&pool)
        .await;
        assert!(
            finalized_without_timestamp.is_err(),
            "SQL must reject finalized turns without finalized_at"
        );
        let renew = CoordinationMutation {
            request_id: Uuid::new_v4(),
            expected_version: 3,
            operation: CoordinationOperation::Renew {
                lease_seconds: MIN_LEASE_SECONDS,
            },
            ..finalize.clone()
        };
        assert!(matches!(
            mutate(&pool, community_a, &renew)
                .await
                .expect("blocked renew"),
            CoordinationResult::Conflict {
                reason: CoordinationConflict::Finalized,
                ..
            }
        ));
        let blocked_contribution = CoordinationMutation {
            request_id: Uuid::new_v4(),
            expected_version: 3,
            claimant_pubkey: deputy,
            operation: CoordinationOperation::Contribute {
                fingerprint: "critical-risk:f3".into(),
            },
            ..finalize
        };
        assert!(matches!(
            mutate(&pool, community_a, &blocked_contribution)
                .await
                .expect("blocked contribution"),
            CoordinationResult::Conflict {
                reason: CoordinationConflict::Finalized,
                ..
            }
        ));

        let new_turn = RoomScope {
            turn_id: "turn-c".into(),
            ..scope
        };
        assert!(matches!(
            mutate(
                &pool,
                community_a,
                &claim(new_turn, deputy, 0, Uuid::new_v4(), 1)
            )
            .await
            .expect("new turn"),
            CoordinationResult::Applied { .. }
        ));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn expired_renew_and_revoked_memberships_are_rejected() {
        let pool = setup_pool().await;
        let channel = Uuid::new_v4();
        let (community, holder, deputy) = fixture(&pool, channel).await;
        let scope = RoomScope {
            channel_id: channel,
            thread_id: "negative-thread".into(),
            turn_id: "expired-renew".into(),
        };
        let CoordinationResult::Applied { state } = mutate(
            &pool,
            community,
            &claim(scope.clone(), holder, 0, Uuid::new_v4(), 1),
        )
        .await
        .expect("claim") else {
            panic!("claim must apply");
        };
        sqlx::query(
            "UPDATE room_coordination_turns SET lease_expires_at=clock_timestamp()-interval '1 second' \
             WHERE community_id=$1 AND channel_id=$2 AND thread_id=$3 AND turn_id=$4",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .bind(&scope.thread_id)
        .bind(&scope.turn_id)
        .execute(&pool)
        .await
        .expect("expire lease");
        let expired_renew = CoordinationMutation {
            scope: scope.clone(),
            expected_version: state.version,
            request_id: Uuid::new_v4(),
            claimant_pubkey: holder,
            relay_authorizer_pubkey: holder,
            require_relay_membership: true,
            nip98_event_id: [31; 32],
            operation: CoordinationOperation::Renew {
                lease_seconds: MIN_LEASE_SECONDS,
            },
        };
        assert!(matches!(
            mutate(&pool, community, &expired_renew)
                .await
                .expect("expired renew result"),
            CoordinationResult::Conflict {
                reason: CoordinationConflict::LeaseExpired,
                ..
            }
        ));
        let expired_finalize = CoordinationMutation {
            request_id: Uuid::new_v4(),
            nip98_event_id: [32; 32],
            operation: CoordinationOperation::Finalize {
                final_event_id: Some("must-not-finalize-after-expiry".into()),
            },
            ..expired_renew
        };
        assert!(matches!(
            mutate(&pool, community, &expired_finalize)
                .await
                .expect("expired finalize result"),
            CoordinationResult::Conflict {
                reason: CoordinationConflict::LeaseExpired,
                ..
            }
        ));

        for (turn_id, operation) in [
            (
                "expires-during-finalize-lock-wait",
                CoordinationOperation::Finalize {
                    final_event_id: Some("must-not-finalize-after-lock-wait".into()),
                },
            ),
            (
                "expires-during-renew-lock-wait",
                CoordinationOperation::Renew {
                    lease_seconds: MIN_LEASE_SECONDS,
                },
            ),
        ] {
            let waiting_scope = RoomScope {
                turn_id: turn_id.into(),
                ..scope.clone()
            };
            let CoordinationResult::Applied { state } = mutate(
                &pool,
                community,
                &claim(waiting_scope.clone(), holder, 0, Uuid::new_v4(), 1),
            )
            .await
            .expect("claim before lock wait") else {
                panic!("claim before lock wait must apply");
            };
            sqlx::query(
                "UPDATE room_coordination_turns \
                 SET lease_expires_at=clock_timestamp()+interval '400 milliseconds' \
                 WHERE community_id=$1 AND channel_id=$2 AND thread_id=$3 AND turn_id=$4",
            )
            .bind(community.as_uuid())
            .bind(channel)
            .bind(&waiting_scope.thread_id)
            .bind(&waiting_scope.turn_id)
            .execute(&pool)
            .await
            .expect("shorten lease before lock wait");

            let mut blocker = pool.begin().await.expect("begin membership lock blocker");
            crate::channel::acquire_channel_membership_lock(&mut blocker, community, channel)
                .await
                .expect("hold membership lock");
            let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *blocker)
                .await
                .expect("read membership lock blocker pid");
            let waiting_pool = pool.clone();
            let pending = tokio::spawn(async move {
                mutate(
                    &waiting_pool,
                    community,
                    &CoordinationMutation {
                        scope: waiting_scope,
                        expected_version: state.version,
                        request_id: Uuid::new_v4(),
                        claimant_pubkey: holder,
                        relay_authorizer_pubkey: holder,
                        require_relay_membership: true,
                        nip98_event_id: Sha256::digest(Uuid::new_v4().as_bytes()).into(),
                        operation,
                    },
                )
                .await
            });
            wait_for_membership_lock_wait(&pool, community, channel, blocker_pid).await;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            blocker.commit().await.expect("release membership lock");
            assert!(matches!(
                pending
                    .await
                    .expect("join lock-wait mutation")
                    .expect("lock-wait result"),
                CoordinationResult::Conflict {
                    reason: CoordinationConflict::LeaseExpired,
                    ..
                }
            ));
        }

        // A request id is community-scoped rather than turn-scoped. Therefore a
        // mutation can clear its turn lock and still block on a request-ledger
        // row owned by another turn. Delete the existing claim's ledger row in
        // a transaction, hold that row lock across lease expiry, then let both
        // holder-only operations continue down the new-mutation path. Their
        // lease decision must use a DB instant sampled after the row-lock wait.
        for (turn_id, operation) in [
            (
                "expires-during-request-lock-finalize",
                CoordinationOperation::Finalize {
                    final_event_id: Some("must-not-finalize-after-request-lock".into()),
                },
            ),
            (
                "expires-during-request-lock-renew",
                CoordinationOperation::Renew {
                    lease_seconds: MIN_LEASE_SECONDS,
                },
            ),
        ] {
            let waiting_scope = RoomScope {
                turn_id: turn_id.into(),
                ..scope.clone()
            };
            let claim_request_id = Uuid::new_v4();
            let initial_claim = claim(waiting_scope.clone(), holder, 0, claim_request_id, 1);
            let CoordinationResult::Applied { state } = mutate(&pool, community, &initial_claim)
                .await
                .expect("claim before request-ledger lock wait")
            else {
                panic!("claim before request-ledger lock wait must apply");
            };
            sqlx::query(
                "UPDATE room_coordination_turns \
                 SET lease_expires_at=clock_timestamp()+interval '400 milliseconds' \
                 WHERE community_id=$1 AND channel_id=$2 AND thread_id=$3 AND turn_id=$4",
            )
            .bind(community.as_uuid())
            .bind(channel)
            .bind(&waiting_scope.thread_id)
            .bind(&waiting_scope.turn_id)
            .execute(&pool)
            .await
            .expect("shorten lease before request-ledger lock wait");

            let mut blocker = pool.begin().await.expect("begin request-ledger blocker");
            sqlx::query(
                "DELETE FROM room_coordination_requests \
                 WHERE community_id=$1 AND request_id=$2",
            )
            .bind(community.as_uuid())
            .bind(claim_request_id)
            .execute(&mut *blocker)
            .await
            .expect("lock existing request-ledger row by deleting it");

            let waiting_pool = pool.clone();
            let pending = tokio::spawn(async move {
                mutate(
                    &waiting_pool,
                    community,
                    &CoordinationMutation {
                        scope: waiting_scope,
                        expected_version: state.version,
                        request_id: claim_request_id,
                        claimant_pubkey: holder,
                        relay_authorizer_pubkey: holder,
                        require_relay_membership: true,
                        nip98_event_id: Sha256::digest(Uuid::new_v4().as_bytes()).into(),
                        operation,
                    },
                )
                .await
            });
            wait_for_request_ledger_lock_wait(&pool).await;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            blocker.commit().await.expect("release deleted request row");
            assert!(matches!(
                pending
                    .await
                    .expect("join request-lock mutation")
                    .expect("request-lock result"),
                CoordinationResult::Conflict {
                    reason: CoordinationConflict::LeaseExpired,
                    ..
                }
            ));
        }

        // An existing exact request remains an early return even if its ledger
        // row was locked while the represented lease expired. Do not re-evaluate
        // the original mutation against current lease state on an exact replay.
        let replay_scope = RoomScope {
            turn_id: "idempotent-replay-across-request-lock".into(),
            ..scope.clone()
        };
        let replay_request = claim(replay_scope.clone(), holder, 0, Uuid::new_v4(), 1);
        let CoordinationResult::Applied {
            state: replay_state,
        } = mutate(&pool, community, &replay_request)
            .await
            .expect("claim before locked exact replay")
        else {
            panic!("claim before locked exact replay must apply");
        };
        sqlx::query(
            "UPDATE room_coordination_turns \
             SET lease_expires_at=clock_timestamp()+interval '400 milliseconds' \
             WHERE community_id=$1 AND channel_id=$2 AND thread_id=$3 AND turn_id=$4",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .bind(&replay_scope.thread_id)
        .bind(&replay_scope.turn_id)
        .execute(&pool)
        .await
        .expect("shorten lease before exact replay lock wait");
        let mut replay_blocker = pool.begin().await.expect("begin exact replay blocker");
        sqlx::query(
            "SELECT 1 FROM room_coordination_requests \
             WHERE community_id=$1 AND request_id=$2 FOR UPDATE",
        )
        .bind(community.as_uuid())
        .bind(replay_request.request_id)
        .execute(&mut *replay_blocker)
        .await
        .expect("lock exact replay request row");
        let replay_pool = pool.clone();
        let pending_replay =
            tokio::spawn(async move { mutate(&replay_pool, community, &replay_request).await });
        wait_for_request_ledger_lock_wait(&pool).await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        replay_blocker
            .commit()
            .await
            .expect("release exact replay request row");
        assert!(matches!(
            pending_replay
                .await
                .expect("join exact replay")
                .expect("exact replay result"),
            CoordinationResult::Idempotent { state }
                if state == replay_state
        ));

        let timestamp_scope = RoomScope {
            turn_id: "post-lock-authoritative-timestamps".into(),
            ..scope.clone()
        };
        let mut blocker = pool.begin().await.expect("begin timestamp blocker");
        crate::channel::acquire_channel_membership_lock(&mut blocker, community, channel)
            .await
            .expect("hold timestamp membership lock");
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await
            .expect("read timestamp lock blocker pid");
        let waiting_pool = pool.clone();
        let waiting_scope = timestamp_scope.clone();
        let pending = tokio::spawn(async move {
            mutate(
                &waiting_pool,
                community,
                &claim(waiting_scope, holder, 0, Uuid::new_v4(), 1),
            )
            .await
        });
        wait_for_membership_lock_wait(&pool, community, channel, blocker_pid).await;
        let claim_floor: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *blocker)
            .await
            .expect("capture claim release floor");
        blocker.commit().await.expect("release timestamp lock");
        assert!(matches!(
            pending
                .await
                .expect("join timestamp claim")
                .expect("timestamp claim result"),
            CoordinationResult::Applied { .. }
        ));
        let (created_at, updated_at): (DateTime<Utc>, DateTime<Utc>) = sqlx::query_as(
            "SELECT created_at,updated_at FROM room_coordination_turns \
             WHERE community_id=$1 AND channel_id=$2 AND thread_id=$3 AND turn_id=$4",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .bind(&timestamp_scope.thread_id)
        .bind(&timestamp_scope.turn_id)
        .fetch_one(&pool)
        .await
        .expect("read post-lock claim timestamps");
        assert!(
            created_at >= claim_floor && updated_at >= claim_floor,
            "initial claim timestamps must come from the post-lock DB instant"
        );

        let revoked_scope = RoomScope {
            turn_id: "revoked-channel-member".into(),
            ..scope.clone()
        };
        sqlx::query(
            "UPDATE channel_members SET removed_at=clock_timestamp() \
             WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .bind(deputy.as_slice())
        .execute(&pool)
        .await
        .expect("revoke channel membership");
        assert!(matches!(
            mutate(
                &pool,
                community,
                &claim(revoked_scope, deputy, 0, Uuid::new_v4(), 1)
            )
            .await,
            Err(DbError::AccessDenied(_))
        ));

        let relay_scope = RoomScope {
            turn_id: "revoked-relay-member".into(),
            ..scope
        };
        sqlx::query("DELETE FROM relay_members WHERE community_id=$1 AND pubkey=$2")
            .bind(community.as_uuid())
            .bind(hex::encode(holder))
            .execute(&pool)
            .await
            .expect("revoke relay membership");
        assert!(matches!(
            mutate(
                &pool,
                community,
                &claim(relay_scope, holder, 0, Uuid::new_v4(), 1)
            )
            .await,
            Err(DbError::AccessDenied(_))
        ));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn committed_channel_membership_revocation_fences_pending_mutation() {
        let pool = setup_pool().await;
        let channel = Uuid::new_v4();
        let (community, _holder, deputy) = fixture(&pool, channel).await;
        let scope = RoomScope {
            channel_id: channel,
            thread_id: "revocation-race".into(),
            turn_id: "turn".into(),
        };

        let mut revoke = pool.begin().await.expect("begin revocation");
        crate::channel::acquire_channel_membership_lock(&mut revoke, community, channel)
            .await
            .expect("lock membership");
        sqlx::query(
            "UPDATE channel_members SET removed_at=clock_timestamp() \
             WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .bind(deputy.as_slice())
        .execute(&mut *revoke)
        .await
        .expect("stage revocation");

        let contender_pool = pool.clone();
        let pending = tokio::spawn(async move {
            mutate(
                &contender_pool,
                community,
                &claim(scope, deputy, 0, Uuid::new_v4(), 1),
            )
            .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            !pending.is_finished(),
            "mutation must wait behind revocation"
        );
        revoke.commit().await.expect("commit revocation");

        assert!(matches!(
            pending.await.expect("join mutation"),
            Err(DbError::AccessDenied(_))
        ));
        let turn_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM room_coordination_turns \
             WHERE community_id=$1 AND channel_id=$2 AND thread_id='revocation-race'",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .fetch_one(&pool)
        .await
        .expect("count turns");
        assert_eq!(turn_count, 0, "revoked member must not mutate state");
    }
}
