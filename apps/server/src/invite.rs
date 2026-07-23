use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use railyard_auth::{InvitePayload, InviteProject, unix_timestamp};
use std::{env, io, net::IpAddr, net::UdpSocket};

use crate::db::{Db, token_hash};
use crate::http::parsed_env;

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
    name: &str,
    project: Option<InviteProject>,
) -> io::Result<MintedInvite> {
    let name = validated_name(name)?;
    let server_url = server_url()?;
    let server_name = server_name()?;

    let token = random_token();
    let now = unix_timestamp();
    let expires_at = now + INVITE_TTL_SECONDS;
    let user_id = db
        .create_user(&name, project.as_ref().map(|p| p.id.as_str()), now)
        .await?;
    db.create_invite(&user_id, &token_hash(&token), now, expires_at)
        .await?;

    let blob = InvitePayload {
        server_url,
        server_name,
        user_id: user_id.clone(),
        user_name: name,
        project,
        invite_token: token,
        expires_at,
    }
    .encode();

    Ok(MintedInvite {
        user_id,
        blob,
        expires_at,
    })
}

/// Where redeemed invites will point their API requests. `RAILYARD_SERVER_URL`
/// overrides (needed behind a domain or TLS terminator); otherwise the
/// machine's outbound IP + the proxy port + the `/railyard` API prefix.
fn server_url() -> io::Result<String> {
    if let Ok(url) = env::var("RAILYARD_SERVER_URL") {
        return Ok(url);
    }

    let ip = outbound_ip().map_err(|error| {
        io::Error::other(format!(
            "could not detect this machine's IP ({error}); set RAILYARD_SERVER_URL explicitly"
        ))
    })?;
    let port: u16 = parsed_env("RAILYARD_PROXY_PORT", 3000, "a port number")?;

    Ok(match port {
        80 => format!("http://{ip}/railyard"),
        _ => format!("http://{ip}:{port}/railyard"),
    })
}

/// The IP this machine reaches the internet from — on a VPS, its public
/// address. Connecting a UDP socket sends no packets; it only asks the
/// kernel which local address routes out.
fn outbound_ip() -> io::Result<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("8.8.8.8:80")?;
    Ok(socket.local_addr()?.ip())
}

/// The server's human name, embedded in invites so clients can derive a
/// profile name even when the URL is a bare IP. `RAILYARD_SERVER_NAME`
/// overrides; the default is the OS hostname's first label (dropping
/// `.local` on macOS and the domain of an FQDN).
fn server_name() -> io::Result<String> {
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
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "could not determine a server name from the hostname; set RAILYARD_SERVER_NAME",
        ));
    }
    Ok(name)
}

fn validated_name(name: &str) -> io::Result<String> {
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_'));

    if valid {
        Ok(name.to_string())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("user name {name:?} must be lowercase letters, digits, - or _"),
        ))
    }
}

fn random_token() -> String {
    use rand::RngCore;

    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
