use railyard_auth::{
    CreateProjectRequest, CreateUserRequest, CreateUserResponse, DeploymentSummary,
    HEADER_CONTENT_SHA256, HEADER_KEY_ID, HEADER_NONCE, HEADER_SIGNATURE, HEADER_SIGNATURE_VERSION,
    HEADER_TIMESTAMP, InvitePayload, ListProjectsResponse, ListUsersResponse, PROJECTS_PATH,
    ProjectSummary, REDEEM_INVITE_PATH, RedeemInviteRequest, RedeemInviteResponse,
    SIGNATURE_VERSION, USERS_PATH, UserSummary, WHOAMI_PATH, WhoamiResponse,
    project_deployments_path,
};
use reqwest::{Client, Method, Response, StatusCode, Url};
use std::error::Error;

use crate::auth::sign_request;
use crate::config::{ServerConfig, read_signing_key};

pub(crate) async fn redeem_invite(
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
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("invite redemption failed ({status}): {body}").into());
    }

    Ok(response.json().await?)
}

pub(crate) async fn list_projects(
    server: &ServerConfig,
) -> Result<Vec<ProjectSummary>, Box<dyn Error>> {
    let response = signed_request(server, Method::GET, PROJECTS_PATH, Vec::new())
        .await?
        .error_for_status()?;
    let listed: ListProjectsResponse = response.json().await?;
    Ok(listed.projects)
}

pub(crate) async fn create_project(
    server: &ServerConfig,
    name: &str,
    id: Option<&str>,
) -> Result<ProjectSummary, Box<dyn Error>> {
    let body = serde_json::to_vec(&CreateProjectRequest {
        name: name.to_string(),
        id: id.map(ToOwned::to_owned),
    })?;
    let response = signed_request(server, Method::POST, PROJECTS_PATH, body).await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("project creation failed ({status}): {body}").into());
    }

    Ok(response.json().await?)
}

/// Upload a packed repository archive; the server unpacks it and answers
/// with the deployment it created (or a failure explaining why not).
pub(crate) async fn create_deployment(
    server: &ServerConfig,
    project_id: &str,
    archive: Vec<u8>,
) -> Result<DeploymentSummary, Box<dyn Error>> {
    let path = project_deployments_path(project_id);
    let response =
        signed_request_with_type(server, Method::POST, &path, archive, "application/gzip").await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("deployment upload failed ({status}): {body}").into());
    }

    Ok(response.json().await?)
}

pub(crate) async fn create_user(
    server: &ServerConfig,
    name: &str,
    project_id: Option<&str>,
) -> Result<CreateUserResponse, Box<dyn Error>> {
    let body = serde_json::to_vec(&CreateUserRequest {
        name: name.to_string(),
        project_id: project_id.map(ToOwned::to_owned),
    })?;
    let response = signed_request(server, Method::POST, USERS_PATH, body).await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("user creation failed ({status}): {body}").into());
    }

    Ok(response.json().await?)
}

pub(crate) async fn list_users(server: &ServerConfig) -> Result<Vec<UserSummary>, Box<dyn Error>> {
    let response = signed_request(server, Method::GET, USERS_PATH, Vec::new()).await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("user listing failed ({status}): {body}").into());
    }

    let listed: ListUsersResponse = response.json().await?;
    Ok(listed.users)
}

/// Ok(false) means the server knows no such user.
pub(crate) async fn remove_user(server: &ServerConfig, name: &str) -> Result<bool, Box<dyn Error>> {
    let path = format!("{USERS_PATH}/{name}");
    let response = signed_request(server, Method::DELETE, &path, Vec::new()).await?;

    match response.status() {
        StatusCode::NO_CONTENT => Ok(true),
        StatusCode::NOT_FOUND => Ok(false),
        status => {
            let body = response.text().await.unwrap_or_default();
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

pub(crate) async fn whoami(server: &ServerConfig) -> Result<WhoamiOutcome, Box<dyn Error>> {
    let response = match signed_request(server, Method::GET, WHOAMI_PATH, Vec::new()).await {
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
        return Ok(WhoamiOutcome::Identity(response.json().await?));
    }
    Ok(WhoamiOutcome::Rejected(format!(
        "({status}) {}",
        response.text().await.unwrap_or_default().trim()
    )))
}

async fn signed_request(
    server: &ServerConfig,
    method: Method,
    suffix: &str,
    body: Vec<u8>,
) -> Result<Response, Box<dyn Error>> {
    signed_request_with_type(server, method, suffix, body, "application/json").await
}

async fn signed_request_with_type(
    server: &ServerConfig,
    method: Method,
    suffix: &str,
    body: Vec<u8>,
    content_type: &str,
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
        request = request.header("content-type", content_type).body(body);
    }

    Ok(request.send().await?)
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
