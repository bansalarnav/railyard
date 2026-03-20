use axum::{Router, routing::get};
use std::{
    env,
    net::{IpAddr, SocketAddr},
};

#[tokio::main]
async fn main() {
    let config = config::AppConfig::default();
    let app = Router::new().route("/", get(root));
    let addr = SocketAddr::from((server_host(), server_port()));

    println!("Starting {} server on http://{}", config.app_name, addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind TCP listener");

    axum::serve(listener, app)
        .await
        .expect("server exited with error");
}

async fn root() -> &'static str {
    "Hello from aethon server"
}

fn server_host() -> IpAddr {
    match env::var("SERVER_HOST") {
        Ok(value) => value.parse().unwrap_or_else(|_| {
            panic!("SERVER_HOST must be a valid IP address, got {value:?}");
        }),
        Err(_) => IpAddr::from([127, 0, 0, 1]),
    }
}

fn server_port() -> u16 {
    match env::var("SERVER_PORT") {
        Ok(value) => value.parse().unwrap_or_else(|_| {
            panic!("SERVER_PORT must be a valid port number, got {value:?}");
        }),
        Err(_) => 3000,
    }
}
