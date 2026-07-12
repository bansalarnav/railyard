use railyard_auth::{
    CreateProjectRequest, CreateUserRequest, CreateUserResponse, HEADER_CONTENT_SHA256,
    HEADER_KEY_ID, HEADER_NONCE, HEADER_SIGNATURE, HEADER_SIGNATURE_VERSION, HEADER_TIMESTAMP,
    InvitePayload, ListProjectsResponse, ListUsersResponse, PROJECTS_PATH, ProjectSummary,
    REDEEM_INVITE_PATH, RedeemInviteRequest, RedeemInviteResponse, SIGNATURE_VERSION, USERS_PATH,
    UserSummary, WHOAMI_PATH, WhoamiResponse,
};
use reqwest::blocking::{Client, Response};
use reqwest::{Method, StatusCode, Url};
use std::error::Error;

use crate::auth::sign_request;
use crate::config::{ServerConfig, read_signing_key};

pub(crate) fn redeem_invite(
    invite: &InvitePayload,
    public_key: &str,
) -> Result<RedeemInviteResponse, Box<dyn Error>> {
    let base_url = Url::parse(&invite.server_url)?;
    let redeem_url = control_plane_api_url(base_url, REDEEM_INVITE_PATH)?;

    let response = Client::new()
        .post(redeem_url)
        .json(&RedeemInviteRequest {
            invite_token: invite.invite_token.clone(),
            public_key: public_key.to_string(),
        })
        .send()?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(format!("invite redemption failed ({status}): {body}").into());
    }

    Ok(response.json()?)
}

pub(crate) fn list_projects(server: &ServerConfig) -> Result<Vec<ProjectSummary>, Box<dyn Error>> {
    let response =
        signed_request(server, Method::GET, PROJECTS_PATH, Vec::new())?.error_for_status()?;
    let listed: ListProjectsResponse = response.json()?;
    Ok(listed.projects)
}

pub(crate) fn create_project(
    server: &ServerConfig,
    name: &str,
    id: Option<&str>,
) -> Result<ProjectSummary, Box<dyn Error>> {
    let body = serde_json::to_vec(&CreateProjectRequest {
        name: name.to_string(),
        id: id.map(ToOwned::to_owned),
    })?;
    let response = signed_request(server, Method::POST, PROJECTS_PATH, body)?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(format!("project creation failed ({status}): {body}").into());
    }

    Ok(response.json()?)
}

pub(crate) fn create_user(
    server: &ServerConfig,
    name: &str,
    project_id: Option<&str>,
) -> Result<CreateUserResponse, Box<dyn Error>> {
    let body = serde_json::to_vec(&CreateUserRequest {
        name: name.to_string(),
        project_id: project_id.map(ToOwned::to_owned),
    })?;
    let response = signed_request(server, Method::POST, USERS_PATH, body)?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(format!("user creation failed ({status}): {body}").into());
    }

    Ok(response.json()?)
}

pub(crate) fn list_users(server: &ServerConfig) -> Result<Vec<UserSummary>, Box<dyn Error>> {
    let response = signed_request(server, Method::GET, USERS_PATH, Vec::new())?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(format!("user listing failed ({status}): {body}").into());
    }

    let listed: ListUsersResponse = response.json()?;
    Ok(listed.users)
}

/// Ok(false) means the server knows no such user.
pub(crate) fn remove_user(server: &ServerConfig, name: &str) -> Result<bool, Box<dyn Error>> {
    let path = format!("{USERS_PATH}/{name}");
    let response = signed_request(server, Method::DELETE, &path, Vec::new())?;

    match response.status() {
        StatusCode::NO_CONTENT => Ok(true),
        StatusCode::NOT_FOUND => Ok(false),
        status => {
            let body = response.text().unwrap_or_default();
            Err(format!("user removal failed ({status}): {body}").into())
        }
    }
}

pub(crate) enum WhoamiOutcome {
    Identity(WhoamiResponse),
    /// The server answered but rejected the key (revoked, unknown, …).
    Rejected(String),
    Unreachable,
}

pub(crate) fn whoami(server: &ServerConfig) -> Result<WhoamiOutcome, Box<dyn Error>> {
    let response = match signed_request(server, Method::GET, WHOAMI_PATH, Vec::new()) {
        Ok(response) => response,
        Err(error) => {
            let network = error
                .downcast_ref::<reqwest::Error>()
                .is_some_and(|error| error.is_connect() || error.is_timeout());
            if network {
                return Ok(WhoamiOutcome::Unreachable);
            }
            return Err(error);
        }
    };

    let status = response.status();
    if status.is_success() {
        return Ok(WhoamiOutcome::Identity(response.json()?));
    }
    Ok(WhoamiOutcome::Rejected(format!(
        "({status}) {}",
        response.text().unwrap_or_default().trim()
    )))
}

fn signed_request(
    server: &ServerConfig,
    method: Method,
    suffix: &str,
    body: Vec<u8>,
) -> Result<Response, Box<dyn Error>> {
    let signing_key = read_signing_key(&server.private_key_path)?;
    let server_url = Url::parse(&server.server_url)?;
    let request_url = control_plane_api_url(server_url, suffix)?;
    let path_and_query = request_url[url::Position::BeforePath..].to_string();
    let host = request_url
        .host_str()
        .ok_or("server URL is missing a host")?
        .to_string();
    let host = match request_url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    };
    let signed = sign_request(
        &signing_key,
        &server.key_id,
        method.as_str(),
        &path_and_query,
        &host,
        &body,
    );

    let mut request = Client::new()
        .request(method, request_url)
        .header("host", host)
        .header(HEADER_KEY_ID, signed.key_id)
        .header(HEADER_TIMESTAMP, signed.timestamp.to_string())
        .header(HEADER_NONCE, signed.nonce)
        .header(HEADER_CONTENT_SHA256, signed.content_sha256)
        .header(HEADER_SIGNATURE, signed.signature)
        .header(HEADER_SIGNATURE_VERSION, SIGNATURE_VERSION);

    if !body.is_empty() {
        request = request
            .header("content-type", "application/json")
            .body(body);
    }

    Ok(request.send()?)
}

fn control_plane_api_url(mut base_url: Url, suffix: &str) -> Result<Url, Box<dyn Error>> {
    let existing_segments: Vec<String> = base_url
        .path_segments()
        .map(|segments| {
            segments
                .filter(|segment| !segment.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();

    {
        let mut segments = base_url
            .path_segments_mut()
            .map_err(|_| "server URL cannot be a cannot-be-a-base URL")?;
        segments.clear();

        for segment in &existing_segments {
            segments.push(segment);
        }

        for segment in suffix.split('/') {
            if !segment.is_empty() {
                segments.push(segment);
            }
        }
    }

    Ok(base_url)
}
