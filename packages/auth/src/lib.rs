mod invite;

pub use invite::{
    INVITE_BLOB_PREFIX, InviteParseError, InvitePayload, REDEEM_INVITE_PATH, RedeemInviteRequest,
    RedeemInviteResponse,
};

use std::time::{SystemTime, UNIX_EPOCH};

pub const SIGNATURE_VERSION: &str = "v1";

pub const HEADER_SIGNATURE_VERSION: &str = "x-railyard-signature-version";
pub const HEADER_KEY_ID: &str = "x-railyard-key-id";
pub const HEADER_TIMESTAMP: &str = "x-railyard-timestamp";
pub const HEADER_NONCE: &str = "x-railyard-nonce";
pub const HEADER_CONTENT_SHA256: &str = "x-railyard-content-sha256";
pub const HEADER_SIGNATURE: &str = "x-railyard-signature";
pub fn canonical_request(
    key_id: &str,
    timestamp: u64,
    nonce: &str,
    method: &str,
    path_and_query: &str,
    host: &str,
    content_sha256: &str,
) -> String {
    format!(
        "RAILYARD-REQUEST-V1\nkey_id:{key_id}\ntimestamp:{timestamp}\nnonce:{nonce}\nmethod:{method}\npath:{path_and_query}\nhost:{host}\ncontent_sha256:{content_sha256}"
    )
}

pub fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs()
}
