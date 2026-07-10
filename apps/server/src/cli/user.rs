use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use railyard_auth::{CreateUserRequest, CreateUserResponse, USERS_PATH, unix_timestamp};
use std::{future::Future, io};
use tokio::net::UnixStream;

use crate::db::Db;
use crate::paths;

pub(crate) fn add(name: &str) -> io::Result<()> {
    block_on(async move {
        let created = create_user_via_daemon(name).await?;

        println!("Created user {name}.");
        println!("Single-use invite, expires in 24h. Redeem with `railyard login <blob>`:");
        println!();
        println!("{}", created.invite_blob);
        Ok(())
    })
}

/// Invites go through the daemon's local admin socket rather than opening
/// the database directly, so client and server CLI share one minting path
/// and blobs always describe the server with the daemon's environment.
async fn create_user_via_daemon(name: &str) -> io::Result<CreateUserResponse> {
    let stream = UnixStream::connect(paths::admin_sock_path())
        .await
        .map_err(|error| {
            io::Error::other(format!(
                "could not reach the railyard-server daemon ({error}); start it with `railyard-server up`"
            ))
        })?;

    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(io::Error::other)?;
    tokio::spawn(connection);

    let body = serde_json::to_vec(&CreateUserRequest {
        name: name.to_string(),
        project_id: None,
    })?;
    let request = hyper::Request::post(USERS_PATH)
        .header("host", "railyard")
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .map_err(io::Error::other)?;

    let response = sender
        .send_request(request)
        .await
        .map_err(io::Error::other)?;
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .map_err(io::Error::other)?
        .to_bytes();

    if !status.is_success() {
        return Err(io::Error::other(format!(
            "user creation failed ({status}): {}",
            String::from_utf8_lossy(&bytes)
        )));
    }

    serde_json::from_slice(&bytes).map_err(io::Error::other)
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
            let scope = user.project_id.as_deref().unwrap_or("admin");
            println!(
                "{}\t{}\t{}\t{}\tcreated {} ago",
                user.name,
                user.id,
                scope,
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
