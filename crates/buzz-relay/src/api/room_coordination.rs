//! Authenticated HTTP RPC for transactional room-response coordination.
//!
//! This is a narrow exception to Buzz's Nostr-first rule. A lease CAS must
//! synchronously report one winner under concurrent takeover; generic signed
//! event ingest cannot provide that response semantic honestly. Authentication
//! remains Nostr-native through NIP-98 and tenant selection remains Host-bound.

use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Json,
};
use buzz_db::room_coordination::{
    CoordinationConflict, CoordinationMutation, CoordinationOperation, CoordinationResult,
    RoomScope,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::state::AppState;

use super::{api_error, bridge, internal_error};

/// Request envelope. Claimant identity is deliberately absent: it is derived
/// exclusively from the verified NIP-98 event.
#[derive(Debug, Deserialize)]
pub struct CoordinationRequest {
    /// Channel scope.
    pub channel_id: Uuid,
    /// Thread scope.
    pub thread_id: String,
    /// Human-trigger/turn scope. Use a new ID instead of reopening finalized state.
    pub turn_id: String,
    /// Operation (`read`, `claim`, `renew`, `contribute`, or `finalize`).
    #[serde(flatten)]
    pub operation: RequestOperation,
}

/// Wire operations accepted by the endpoint.
#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum RequestOperation {
    /// Read current state (no CAS/idempotency key).
    Read,
    /// Acquire an absent/expired lease.
    Claim {
        /// Initial state expects zero; takeover expects the current version.
        expected_version: i64,
        /// Durable idempotency key.
        request_id: Uuid,
        /// Lease duration, bounded by buzz-db constants.
        lease_seconds: u32,
        /// One-turn deputy contribution budget.
        #[serde(default = "default_contribution_budget")]
        contribution_budget: u32,
    },
    /// Renew the holder lease.
    Renew {
        /// Current version.
        expected_version: i64,
        /// Durable idempotency key.
        request_id: Uuid,
        /// Lease duration.
        lease_seconds: u32,
    },
    /// Admit one unique contribution fingerprint without changing the holder.
    Contribute {
        /// Current version.
        expected_version: i64,
        /// Durable idempotency key.
        request_id: Uuid,
        /// Duplicate-suppression fingerprint.
        fingerprint: String,
    },
    /// Permanently finalize this turn (holder only).
    Finalize {
        /// Current version.
        expected_version: i64,
        /// Durable idempotency key.
        request_id: Uuid,
        /// Optional final response event identifier.
        #[serde(default)]
        final_event_id: Option<String>,
    },
}

fn default_contribution_budget() -> u32 {
    1
}

/// `POST /api/room-coordination`.
pub async fn coordinate(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| {
            api_error(
                StatusCode::NOT_FOUND,
                "relay: no community is configured for this host",
            )
        })?;
    let url =
        bridge::nip98_expected_url(&state.config.relay_url, &tenant, "/api/room-coordination");
    let (pubkey, nip98_event_id) =
        bridge::verify_bridge_auth_with_options(&headers, "POST", &url, Some(&body), true, true)?;
    bridge::check_nip98_replay(&state, &tenant, nip98_event_id).await?;
    bridge::enforce_http_admission(&state, &tenant, &pubkey).await?;

    let auth_tag = headers
        .get("x-auth-tag")
        .and_then(|value| value.to_str().ok());
    let relay_authorizer = super::relay_members::enforce_relay_membership(
        &state,
        tenant.community(),
        pubkey.as_bytes(),
        auth_tag,
    )
    .await?;

    let request: CoordinationRequest = serde_json::from_slice(&body).map_err(|error| {
        api_error(
            StatusCode::BAD_REQUEST,
            &format!("invalid room coordination JSON: {error}"),
        )
    })?;
    // Direct membership is intentional even when relay membership was delegated
    // through NIP-OA: the authenticated claimant itself must belong to the room.
    let channel_member = state
        .db
        .is_member(tenant.community(), request.channel_id, pubkey.as_bytes())
        .await
        .map_err(|error| internal_error(&format!("room membership lookup: {error}")))?;
    if !channel_member {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "active channel membership required",
        ));
    }

    let scope = RoomScope {
        channel_id: request.channel_id,
        thread_id: request.thread_id,
        turn_id: request.turn_id,
    };
    if matches!(request.operation, RequestOperation::Read) {
        let state_value = state
            .db
            .read_room_coordination(tenant.community(), &scope)
            .await
            .map_err(|error| internal_error(&format!("room coordination read: {error}")))?;
        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({ "state": state_value })),
        ));
    }

    let (expected_version, request_id, operation) = match request.operation {
        RequestOperation::Read => unreachable!("read returned above"),
        RequestOperation::Claim {
            expected_version,
            request_id,
            lease_seconds,
            contribution_budget,
        } => (
            expected_version,
            request_id,
            CoordinationOperation::Claim {
                lease_seconds,
                contribution_budget,
            },
        ),
        RequestOperation::Renew {
            expected_version,
            request_id,
            lease_seconds,
        } => (
            expected_version,
            request_id,
            CoordinationOperation::Renew { lease_seconds },
        ),
        RequestOperation::Contribute {
            expected_version,
            request_id,
            fingerprint,
        } => (
            expected_version,
            request_id,
            CoordinationOperation::Contribute { fingerprint },
        ),
        RequestOperation::Finalize {
            expected_version,
            request_id,
            final_event_id,
        } => (
            expected_version,
            request_id,
            CoordinationOperation::Finalize { final_event_id },
        ),
    };
    let mutation = CoordinationMutation {
        scope,
        expected_version,
        request_id,
        claimant_pubkey: pubkey.to_bytes(),
        relay_authorizer_pubkey: relay_authorizer
            .as_ref()
            .map(|owner| owner.to_bytes())
            .unwrap_or_else(|| pubkey.to_bytes()),
        require_relay_membership: state.config.require_relay_membership,
        nip98_event_id,
        operation,
    };
    let result = state
        .db
        .mutate_room_coordination(tenant.community(), &mutation)
        .await
        .map_err(|error| match error {
            buzz_db::DbError::InvalidData(message) => api_error(StatusCode::BAD_REQUEST, &message),
            buzz_db::DbError::AccessDenied(message) => api_error(StatusCode::FORBIDDEN, &message),
            error => internal_error(&format!("room coordination mutation: {error}")),
        })?;
    let status = match &result {
        CoordinationResult::Applied { .. } | CoordinationResult::Idempotent { .. } => {
            StatusCode::OK
        }
        CoordinationResult::Conflict { reason, .. } => match reason {
            CoordinationConflict::NotHolder => StatusCode::FORBIDDEN,
            _ => StatusCode::CONFLICT,
        },
    };
    let value = serde_json::to_value(result)
        .map_err(|error| internal_error(&format!("room coordination response: {error}")))?;
    Ok((status, Json(value)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use axum::{
        body::Body,
        http::{header, Request},
    };
    use base64::Engine;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;

    use crate::{router::build_router, state::AppState};

    struct AlwaysFreshReplayGuard;

    impl buzz_auth::Nip98ReplayGuard for AlwaysFreshReplayGuard {
        fn try_mark_in_scope<'a>(
            &'a self,
            _scope: &'a str,
            _event_id: &'a nostr::EventId,
            _ttl_secs: u64,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<bool, buzz_auth::AuthError>> + Send + 'a>,
        > {
            Box::pin(async { Ok(true) })
        }
    }

    async fn test_state(host: &str) -> Arc<AppState> {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .expect("BUZZ_TEST_DATABASE_URL for relay DB test");
        let redis_url = std::env::var("BUZZ_TEST_REDIS_URL")
            .expect("BUZZ_TEST_REDIS_URL for relay admission test");
        let mut config = crate::config::Config::from_env().expect("test config");
        config.database_url = database_url.clone();
        config.redis_url = redis_url;
        config.relay_url = format!("wss://{host}");
        config.require_relay_membership = false;

        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("connect Postgres");
        buzz_db::migration::run_migrations(&pool)
            .await
            .expect("run migrations");
        let db = buzz_db::Db::from_pool(pool.clone());
        db.ensure_configured_community(host)
            .await
            .expect("configured community");
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (mut state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            Keys::generate(),
            media_storage,
        );
        state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);
        Arc::new(state)
    }

    fn auth_header(keys: &Keys, host: &str, body: &[u8]) -> String {
        let url = format!("https://{host}/api/room-coordination");
        let hash: [u8; 32] = Sha256::digest(body).into();
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags([
                Tag::parse(["u", url.as_str()]).expect("u tag"),
                Tag::parse(["method", "POST"]).expect("method tag"),
                Tag::parse(["payload", hex::encode(hash).as_str()]).expect("payload tag"),
            ])
            .sign_with_keys(keys)
            .expect("sign auth");
        let json = serde_json::to_vec(&event).expect("serialize auth");
        format!(
            "Nostr {}",
            base64::engine::general_purpose::STANDARD.encode(json)
        )
    }

    async fn request(
        state: Arc<AppState>,
        host: &str,
        body: Vec<u8>,
        keys: Option<&Keys>,
    ) -> axum::response::Response {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/api/room-coordination")
            .header(header::HOST, host)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(keys) = keys {
            builder = builder.header(header::AUTHORIZATION, auth_header(keys, host, &body));
        }
        build_router(state)
            .oneshot(builder.body(Body::from(body)).expect("request"))
            .await
            .expect("response")
    }

    #[test]
    fn request_has_no_caller_supplied_claimant_identity() {
        let request: CoordinationRequest = serde_json::from_value(serde_json::json!({
            "channel_id": Uuid::new_v4(),
            "thread_id": "thread",
            "turn_id": "turn",
            "operation": "claim",
            "expected_version": 0,
            "request_id": Uuid::new_v4(),
            "lease_seconds": 30,
            "claimant_pubkey": "attacker"
        }))
        .expect("unknown identity field is ignored and never consumed");
        assert!(matches!(request.operation, RequestOperation::Claim { .. }));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn unauthenticated_and_wrong_channel_requests_fail_closed() {
        let host = format!("room-relay-{}.example", Uuid::new_v4().simple());
        let state = test_state(&host).await;
        let keys = Keys::generate();
        let body = serde_json::to_vec(&serde_json::json!({
            "channel_id": Uuid::new_v4(),
            "thread_id": "thread",
            "turn_id": "turn",
            "operation": "claim",
            "expected_version": 0,
            "request_id": Uuid::new_v4(),
            "lease_seconds": 5
        }))
        .expect("body");

        let unauthenticated = request(Arc::clone(&state), &host, body.clone(), None).await;
        assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

        let wrong_channel = request(state, &host, body, Some(&keys)).await;
        assert_eq!(wrong_channel.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn database_failure_never_admits_coordination_request() {
        let host = format!("room-db-fail-{}.example", Uuid::new_v4().simple());
        let state = test_state(&host).await;
        let mut broken_state = Arc::into_inner(state).expect("exclusive test state");
        let broken_pool =
            sqlx::PgPool::connect_lazy("postgres://invalid:invalid@127.0.0.1:1/unreachable")
                .expect("lazy broken pool");
        broken_state.db = buzz_db::Db::from_pool(broken_pool);
        let keys = Keys::generate();
        let body = serde_json::to_vec(&serde_json::json!({
            "channel_id": Uuid::new_v4(),
            "thread_id": "thread",
            "turn_id": "turn",
            "operation": "claim",
            "expected_version": 0,
            "request_id": Uuid::new_v4(),
            "lease_seconds": 5
        }))
        .expect("body");
        let response = request(Arc::new(broken_state), &host, body, Some(&keys)).await;
        assert!(!response.status().is_success());
    }
}
