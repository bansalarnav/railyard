//! Invite blobs (`ryd-invite-v1.<base64url JSON>`) and the redeem exchange,
//! shared so the server mints exactly what the client parses.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const INVITE_BLOB_PREFIX: &str = "ryd-invite-v1.";

pub const REDEEM_INVITE_PATH: &str = "/auth/redeem-invite";

/// The JSON payload inside an invite blob.
#[derive(Debug, Serialize, Deserialize)]
pub struct InvitePayload {
    pub server_url: String,
    pub invite_token: String,
    /// Unix seconds after which the server refuses to redeem the invite.
    pub expires_at: u64,
}

impl InvitePayload {
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("invite payload serializes to JSON");
        format!("{INVITE_BLOB_PREFIX}{}", URL_SAFE_NO_PAD.encode(json))
    }

    pub fn parse(blob: &str) -> Result<Self, InviteParseError> {
        let encoded = blob
            .trim()
            .strip_prefix(INVITE_BLOB_PREFIX)
            .ok_or(InviteParseError)?;
        let json = URL_SAFE_NO_PAD
            .decode(encoded.as_bytes())
            .map_err(|_| InviteParseError)?;
        serde_json::from_slice(&json).map_err(|_| InviteParseError)
    }
}

/// Body of `POST /auth/redeem-invite`, the one unauthenticated endpoint.
#[derive(Debug, Serialize, Deserialize)]
pub struct RedeemInviteRequest {
    pub invite_token: String,
    /// Base64 (standard) ed25519 public key generated on the client.
    pub public_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RedeemInviteResponse {
    pub key_id: String,
}

#[derive(Debug)]
pub struct InviteParseError;

impl fmt::Display for InviteParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not a valid {INVITE_BLOB_PREFIX}* invite blob")
    }
}

impl std::error::Error for InviteParseError {}
