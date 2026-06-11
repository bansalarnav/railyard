use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use ed25519_dalek::{Signer, SigningKey};
use railyard_auth::{canonical_request, unix_timestamp};
use rand::RngCore;
use rand::rngs::OsRng;
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize)]
pub(crate) struct BootstrapResponse {
    pub(crate) key_id: String,
    pub(crate) device_name: String,
    pub(crate) server_url: String,
}

pub(crate) struct SignedRequestHeaders {
    pub(crate) key_id: String,
    pub(crate) timestamp: u64,
    pub(crate) nonce: String,
    pub(crate) content_sha256: String,
    pub(crate) signature: String,
}

pub(crate) fn generate_signing_key() -> SigningKey {
    SigningKey::generate(&mut OsRng)
}

pub(crate) fn public_key_base64(signing_key: &SigningKey) -> String {
    BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes())
}

pub(crate) fn sign_request(
    signing_key: &SigningKey,
    key_id: &str,
    method: &str,
    path_and_query: &str,
    host: &str,
    body: &[u8],
) -> SignedRequestHeaders {
    let timestamp = unix_timestamp();
    let nonce = random_nonce();
    let content_sha256 = hex::encode(Sha256::digest(body));
    let canonical = canonical_request(
        key_id,
        timestamp,
        &nonce,
        method,
        path_and_query,
        host,
        &content_sha256,
    );
    let signature = signing_key.sign(canonical.as_bytes());

    SignedRequestHeaders {
        key_id: key_id.to_string(),
        timestamp,
        nonce,
        content_sha256,
        signature: BASE64_STANDARD.encode(signature.to_bytes()),
    }
}

fn random_nonce() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
