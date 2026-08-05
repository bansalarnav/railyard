use anyhow::{Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use railyard_auth::{InvitePayload, InviteProject, unix_timestamp};
use std::env;

use super::db::{Db, token_hash};

const INVITE_TTL_SECONDS: u64 = 24 * 60 * 60;

pub(crate) struct MintedInvite {
    pub(crate) user_id: String,
    pub(crate) blob: String,
    pub(crate) expires_at: u64,
}

/// Create a user — server-wide admin, or scoped to `project` — together with
/// a single-use invite for it, and encode the self-describing blob a client
/// redeems with `railyard login`.
pub(crate) async fn mint_invite(
    db: &Db,
    server_url: &str,
    name: &str,
    project: Option<InviteProject>,
) -> Result<Option<MintedInvite>> {
    let name = validated_name(name)?;
    let server_name = server_name()?;

    let token = random_token();
    let now = unix_timestamp();
    let expires_at = now + INVITE_TTL_SECONDS;
    let Some(user_id) = db
        .create_user(&name, project.as_ref().map(|p| p.id.as_str()), now)
        .await?
    else {
        return Ok(None);
    };
    db.create_invite(&user_id, &token_hash(&token), now, expires_at)
        .await?;

    let blob = InvitePayload {
        server_url: server_url.to_string(),
        server_name,
        user_id: user_id.clone(),
        user_name: name,
        project,
        invite_token: token,
        expires_at,
    }
    .encode();

    Ok(Some(MintedInvite {
        user_id,
        blob,
        expires_at,
    }))
}

/// The server's human name, embedded in invites so clients can derive a
/// profile name independently of its generated URL. `RAILYARD_SERVER_NAME`
/// overrides; the default is the OS hostname's first label.
fn server_name() -> Result<String> {
    let name = match env::var("RAILYARD_SERVER_NAME") {
        Ok(name) => name,
        Err(_) => {
            let hostname = nix::unistd::gethostname()?
                .into_string()
                .unwrap_or_default();
            hostname.split('.').next().unwrap_or_default().to_string()
        }
    };

    let name = name.trim().to_string();
    if name.is_empty() {
        bail!("could not determine a server name from the hostname; set RAILYARD_SERVER_NAME");
    }
    Ok(name)
}

pub(crate) fn validated_name(name: &str) -> Result<String> {
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_'));

    if valid {
        Ok(name.to_string())
    } else {
        bail!("user name {name:?} must be lowercase letters, digits, - or _")
    }
}

fn random_token() -> String {
    use rand::RngCore;

    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
