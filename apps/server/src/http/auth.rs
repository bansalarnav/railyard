use axum::Json;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use railyard_auth::{
    HEADER_CONTENT_SHA256, HEADER_KEY_ID, HEADER_NONCE, HEADER_SIGNATURE, HEADER_SIGNATURE_VERSION,
    HEADER_TIMESTAMP, RedeemInviteRequest, RedeemInviteResponse, SIGNATURE_VERSION,
    canonical_request, unix_timestamp,
};
use sha2::{Digest, Sha256};

use super::state::ApiState;
use crate::db::token_hash;
const TIMESTAMP_WINDOW_SECONDS: u64 = 300;

const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

pub(crate) async fn redeem_invite(
    State(state): State<ApiState>,
    Json(request): Json<RedeemInviteRequest>,
) -> Response {
    if parse_public_key(&request.public_key).is_none() {
        return (
            StatusCode::BAD_REQUEST,
            "public_key must be a base64 ed25519 public key",
        )
            .into_response();
    }

    let redeemed = state
        .db
        .redeem_invite(
            &token_hash(&request.invite_token),
            &request.public_key,
            unix_timestamp(),
        )
        .await;

    match redeemed {
        Ok(Some(key_id)) => Json(RedeemInviteResponse { key_id }).into_response(),
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            "invite is invalid, expired, or already redeemed",
        )
            .into_response(),
        Err(error) => {
            log::error!("invite redemption failed: {error}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub(crate) async fn verify_signature(
    State(state): State<ApiState>,
    request: Request,
    next: Next,
) -> Response {
    match checked_request(&state, request).await {
        Ok(request) => next.run(request).await,
        Err(reason) => {
            (StatusCode::UNAUTHORIZED, format!("unauthorized: {reason}")).into_response()
        }
    }
}

async fn checked_request(state: &ApiState, request: Request) -> Result<Request, String> {
    let (mut parts, body) = request.into_parts();

    let header = |name: &str| -> Result<&str, String> {
        parts
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| format!("missing or malformed {name} header"))
    };

    if header(HEADER_SIGNATURE_VERSION)? != SIGNATURE_VERSION {
        return Err("unsupported signature version".to_string());
    }

    let key_id = header(HEADER_KEY_ID)?;
    let nonce = header(HEADER_NONCE)?;
    let content_sha256 = header(HEADER_CONTENT_SHA256)?;
    let signature = header(HEADER_SIGNATURE)?;
    let timestamp: u64 = header(HEADER_TIMESTAMP)?
        .parse()
        .map_err(|_| "timestamp is not unix seconds".to_string())?;

    let now = unix_timestamp();
    if now.abs_diff(timestamp) > TIMESTAMP_WINDOW_SECONDS {
        return Err("timestamp outside the allowed window".to_string());
    }

    let body_bytes = to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|_| "failed to read request body".to_string())?;
    if hex::encode(Sha256::digest(&body_bytes)) != content_sha256.to_ascii_lowercase() {
        return Err("body does not match content hash".to_string());
    }

    {
        let mut seen = state.seen_nonces.lock().expect("nonce lock poisoned");
        seen.retain(|_, seen_at| now.abs_diff(*seen_at) <= TIMESTAMP_WINDOW_SECONDS);
        if seen.insert(nonce.to_string(), now).is_some() {
            return Err("nonce already used".to_string());
        }
    }

    let (public_key, user) = state
        .db
        .key_owner(key_id)
        .await
        .map_err(|error| {
            log::error!("key lookup failed: {error}");
            "key lookup failed".to_string()
        })?
        .ok_or_else(|| "unknown or revoked key".to_string())?;
    let public_key =
        parse_public_key(&public_key).ok_or_else(|| "stored public key is corrupt".to_string())?;

    let signature: [u8; 64] = BASE64_STANDARD
        .decode(signature)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| "signature is not base64 ed25519".to_string())?;

    let host = match parts.headers.get("host").and_then(|v| v.to_str().ok()) {
        Some(host) => host.to_string(),
        None => parts
            .uri
            .authority()
            .map(|authority| authority.to_string())
            .ok_or_else(|| "request has no host".to_string())?,
    };
    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");

    let canonical = canonical_request(
        key_id,
        timestamp,
        nonce,
        parts.method.as_str(),
        path_and_query,
        &host,
        content_sha256,
    );

    public_key
        .verify(canonical.as_bytes(), &Signature::from_bytes(&signature))
        .map_err(|_| "signature verification failed".to_string())?;

    parts.extensions.insert(user);
    Ok(Request::from_parts(parts, Body::from(body_bytes)))
}

fn parse_public_key(encoded: &str) -> Option<VerifyingKey> {
    let bytes: [u8; 32] = BASE64_STANDARD.decode(encoded).ok()?.try_into().ok()?;
    VerifyingKey::from_bytes(&bytes).ok()
}
