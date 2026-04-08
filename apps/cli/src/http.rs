use reqwest::Url;
use reqwest::blocking::Client;
use serde_json::Value;
use std::error::Error;

use crate::auth::sign_request;
use crate::config::{read_profile, read_signing_key};

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
        .header("x-aethon-key-id", signed.key_id)
        .header("x-aethon-timestamp", signed.timestamp.to_string())
        .header("x-aethon-nonce", signed.nonce)
        .header("x-aethon-content-sha256", signed.content_sha256)
        .header("x-aethon-signature", signed.signature)
        .header("x-aethon-signature-version", "v1")
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
