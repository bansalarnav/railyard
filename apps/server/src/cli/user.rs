use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use railyard_auth::{InvitePayload, unix_timestamp};
use std::{env, future::Future, io, net::IpAddr, net::UdpSocket};

use crate::db::{Db, token_hash};
use crate::http::parsed_env;

const INVITE_TTL_SECONDS: u64 = 24 * 60 * 60;

pub(crate) fn add(name: &str) -> io::Result<()> {
    let name = validated_name(name)?;
    let server_url = server_url()?;
    let server_name = server_name()?;

    block_on(async move {
        let db = Db::open().await?;
        let token = random_token();
        let now = unix_timestamp();
        let expires_at = now + INVITE_TTL_SECONDS;
        let user_id = db.create_user(&name, now).await?;
        db.create_invite(&user_id, &token_hash(&token), now, expires_at)
            .await?;

        let blob = InvitePayload {
            server_url,
            server_name,
            invite_token: token,
            expires_at,
        }
        .encode();

        println!("Created user {name}.");
        println!("Single-use invite, expires in 24h. Redeem with `railyard login <blob>`:");
        println!();
        println!("{blob}");
        Ok(())
    })
}

pub(crate) fn list() -> io::Result<()> {
    block_on(async {
        let db = Db::open().await?;
        let users = db.list_users().await?;
        if users.is_empty() {
            println!("No users. Create one with `railyard-server user add <name>`.");
            return Ok(());
        }

        let now = unix_timestamp();
        for user in users {
            let status = if user.has_key { "active" } else { "invited" };
            println!(
                "{}\t{}\t{}\tcreated {} ago",
                user.name,
                user.id,
                status,
                format_age(now.saturating_sub(user.created_at))
            );
        }
        Ok(())
    })
}

pub(crate) fn remove(name: &str) -> io::Result<()> {
    block_on(async move {
        let db = Db::open().await?;
        if db.remove_user(name).await? {
            println!("Removed user {name} and revoked its key.");
        } else {
            println!("No user named {name}.");
        }
        Ok(())
    })
}

/// Where redeemed invites will point their API requests. `SERVER_URL`
/// overrides (needed behind a domain or TLS terminator); otherwise the
/// machine's outbound IP + the proxy port + the `/railyard` API prefix.
fn server_url() -> io::Result<String> {
    if let Ok(url) = env::var("SERVER_URL") {
        return Ok(url);
    }

    let ip = outbound_ip().map_err(|error| {
        io::Error::other(format!(
            "could not detect this machine's IP ({error}); set SERVER_URL explicitly"
        ))
    })?;
    let port: u16 = parsed_env("PROXY_PORT", 3000, "a port number")?;

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
/// profile name even when the URL is a bare IP. `SERVER_NAME` overrides;
/// the default is the OS hostname's first label (dropping `.local` on
/// macOS and the domain of an FQDN).
fn server_name() -> io::Result<String> {
    let name = match env::var("SERVER_NAME") {
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
            "could not determine a server name from the hostname; set SERVER_NAME",
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

fn format_age(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m", seconds / 60),
        3600..86400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86400),
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to start tokio runtime")
        .block_on(future)
}
