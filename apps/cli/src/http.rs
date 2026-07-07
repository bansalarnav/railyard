use railyard_auth::{
    HEADER_CONTENT_SHA256, HEADER_KEY_ID, HEADER_NONCE, HEADER_SIGNATURE, HEADER_SIGNATURE_VERSION,
    HEADER_TIMESTAMP, InvitePayload, REDEEM_INVITE_PATH, RedeemInviteRequest, RedeemInviteResponse,
    SIGNATURE_VERSION,
};
use reqwest::Url;
use reqwest::blocking::Client;
use serde_json::Value;
use std::error::Error;

use crate::auth::sign_request;
use crate::config::{read_profile, read_signing_key};

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

pub(crate) fn list_services(profile_name: &str) -> Result<Value, Box<dyn Error>> {
    let profile = read_profile(profile_name)?;
    let signing_key = read_signing_key(&profile.private_key_path)?;
    let server_url = Url::parse(&profile.server_url)?;
    let services_url = control_plane_api_url(server_url, "api/services")?;
    let path_and_query = services_url[url::Position::BeforePath..].to_string();
    let host = services_url
        .host_str()
        .ok_or("server URL is missing a host")?
        .to_string();
    let host = match services_url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    };
    let signed = sign_request(
        &signing_key,
        &profile.key_id,
        "GET",
        &path_and_query,
        &host,
        b"",
    );

    let response = Client::new()
        .get(services_url)
        .header("host", host)
        .header(HEADER_KEY_ID, signed.key_id)
        .header(HEADER_TIMESTAMP, signed.timestamp.to_string())
        .header(HEADER_NONCE, signed.nonce)
        .header(HEADER_CONTENT_SHA256, signed.content_sha256)
        .header(HEADER_SIGNATURE, signed.signature)
        .header(HEADER_SIGNATURE_VERSION, SIGNATURE_VERSION)
        .send()?
        .error_for_status()?;

    Ok(response.json()?)
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
