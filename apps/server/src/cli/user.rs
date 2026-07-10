use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Method, StatusCode};
use hyper_util::rt::TokioIo;
use railyard_auth::{
    CreateUserRequest, CreateUserResponse, ListUsersResponse, USERS_PATH, unix_timestamp,
};
use std::{future::Future, io};
use tokio::net::UnixStream;

use crate::paths;

pub(crate) fn add(name: &str) -> io::Result<()> {
    block_on(async move {
        let body = serde_json::to_vec(&CreateUserRequest {
            name: name.to_string(),
            project_id: None,
        })?;
        let (status, bytes) = admin_request(Method::POST, USERS_PATH, Some(body)).await?;
        if !status.is_success() {
            return Err(request_failed("user creation", status, &bytes));
        }
        let created: CreateUserResponse =
            serde_json::from_slice(&bytes).map_err(io::Error::other)?;

        println!("Created user {name}.");
        println!("Single-use invite, expires in 24h. Redeem with `railyard login <blob>`:");
        println!();
        println!("{}", created.invite_blob);
        Ok(())
    })
}

pub(crate) fn list() -> io::Result<()> {
    block_on(async {
        let (status, bytes) = admin_request(Method::GET, USERS_PATH, None).await?;
        if !status.is_success() {
            return Err(request_failed("user listing", status, &bytes));
        }
        let listed: ListUsersResponse = serde_json::from_slice(&bytes).map_err(io::Error::other)?;

        if listed.users.is_empty() {
            println!("No users. Create one with `railyard-server user add <name>`.");
            return Ok(());
        }

        let now = unix_timestamp();
        for user in listed.users {
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
        let path = format!("{USERS_PATH}/{name}");
        let (status, bytes) = admin_request(Method::DELETE, &path, None).await?;

        match status {
            StatusCode::NO_CONTENT => {
                println!("Removed user {name} and revoked its key.");
                Ok(())
            }
            StatusCode::NOT_FOUND => {
                println!("No user named {name}.");
                Ok(())
            }
            _ => Err(request_failed("user removal", status, &bytes)),
        }
    })
}

/// User commands go through the daemon's local admin socket rather than
/// opening the database directly, so client and server CLI share one API
/// path and only the daemon's process ever touches the database.
async fn admin_request(
    method: Method,
    path: &str,
    body: Option<Vec<u8>>,
) -> io::Result<(StatusCode, Bytes)> {
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

    let mut request = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("host", "railyard");
    if body.is_some() {
        request = request.header("content-type", "application/json");
    }
    let request = request
        .body(Full::new(Bytes::from(body.unwrap_or_default())))
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

    Ok((status, bytes))
}

fn request_failed(action: &str, status: StatusCode, body: &[u8]) -> io::Error {
    io::Error::other(format!(
        "{action} failed ({status}): {}",
        String::from_utf8_lossy(body)
    ))
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
