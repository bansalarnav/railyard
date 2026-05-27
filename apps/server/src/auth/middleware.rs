use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use ed25519_dalek::{Signature, Verifier};
use sha2::{Digest, Sha256};

use crate::auth::NonceCache;
use crate::daemon::AppState;

const MAX_SIGNED_BODY_SIZE: usize = 1024 * 1024;
const ALLOWED_CLOCK_SKEW_SECS: u64 = 300;

pub(crate) async fn require_signed_request(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, String)> {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, MAX_SIGNED_BODY_SIZE)
        .await
        .map_err(|error| bad_request(format!("failed to read request body: {error}")))?;

    verify_request(
        &state,
        &parts.headers,
        parts.method.as_str(),
        &parts.uri,
        &body,
    )?;

    let request = Request::from_parts(parts, Body::from(body));
    Ok(next.run(request).await)
}

fn verify_request(
    state: &AppState,
    headers: &HeaderMap,
    method: &str,
    uri: &http::Uri,
    body: &[u8],
) -> Result<(), (StatusCode, String)> {
    let version = required_header(headers, "x-railyard-signature-version")?;
    if version != "v1" {
        return Err(unauthorized("unsupported signature version"));
    }

    let key_id = required_header(headers, "x-railyard-key-id")?;
    let nonce = required_header(headers, "x-railyard-nonce")?;
    let timestamp = required_header(headers, "x-railyard-timestamp")?
        .parse::<u64>()
        .map_err(|_| bad_request("invalid x-railyard-timestamp"))?;
    let body_hash = required_header(headers, "x-railyard-content-sha256")?;
    let signature_base64 = required_header(headers, "x-railyard-signature")?;
    let host = required_host(headers)?;

    let computed_hash = hex::encode(Sha256::digest(body));
    if computed_hash != body_hash {
        return Err(bad_request("request body hash mismatch"));
    }

    let now = NonceCache::now_unix_secs();
    if now.abs_diff(timestamp) > ALLOWED_CLOCK_SKEW_SECS {
        return Err(unauthorized(
            "request timestamp is outside the allowed window",
        ));
    }

    if !state.auth_nonce_cache.check_and_store(key_id, nonce, now) {
        return Err(unauthorized("nonce has already been used"));
    }

    let verifying_key = state
        .auth_store
        .verifying_key_for(key_id)
        .map_err(internal_error)?
        .ok_or_else(|| unauthorized("unknown or revoked key id"))?;

    let canonical = canonical_request(method, uri, key_id, nonce, timestamp, &body_hash, &host);
    let signature_bytes = BASE64_STANDARD
        .decode(signature_base64.as_bytes())
        .map_err(|_| bad_request("invalid signature encoding"))?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|_| bad_request("invalid signature length"))?;

    verifying_key
        .verify(canonical.as_bytes(), &signature)
        .map_err(|_| unauthorized("signature verification failed"))?;

    Ok(())
}

fn canonical_request(
    method: &str,
    uri: &http::Uri,
    key_id: &str,
    nonce: &str,
    timestamp: u64,
    body_hash: &str,
    host: &str,
) -> String {
    format!(
        "RAILYARD-REQUEST-V1\nkey_id:{key_id}\ntimestamp:{timestamp}\nnonce:{nonce}\nmethod:{method}\npath:{}\nhost:{host}\ncontent_sha256:{body_hash}",
        uri.path_and_query()
            .map(|value| value.as_str())
            .unwrap_or(uri.path())
    )
}

fn required_header<'a>(
    headers: &'a HeaderMap,
    name: &'static str,
) -> Result<&'a str, (StatusCode, String)> {
    headers
        .get(name)
        .ok_or_else(|| bad_request(format!("missing {name}")))?
        .to_str()
        .map_err(|_| bad_request(format!("invalid {name}")))
}

fn required_host(headers: &HeaderMap) -> Result<String, (StatusCode, String)> {
    Ok(headers
        .get(header::HOST)
        .ok_or_else(|| bad_request("missing host header"))?
        .to_str()
        .map_err(|_| bad_request("invalid host header"))?
        .to_string())
}

fn bad_request(message: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, message.into())
}

fn unauthorized(message: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::UNAUTHORIZED, message.into())
}

fn internal_error(error: std::io::Error) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("auth store failure: {error}"),
    )
}
