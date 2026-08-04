use std::{
    collections::BTreeMap,
    env, io,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    str::FromStr,
    sync::Arc,
    time::Duration,
};

const PUBLIC_IP_URL: &str = "https://api.ipify.org";

/// Process-wide configuration, read from the environment at startup and
/// shared by both halves of the server: the ingress proxy routes with it,
/// the API reports it.
#[derive(Clone)]
pub(crate) struct Config {
    pub(crate) proxy_addr: SocketAddr,
    pub(crate) api_addr: SocketAddr,
    pub(crate) server_url: String,
    pub(crate) api_hostname: String,
    pub(crate) service_upstreams: Arc<BTreeMap<String, SocketAddr>>,
}

impl Config {
    pub(crate) fn load() -> io::Result<Self> {
        let proxy_host: IpAddr =
            parsed_env("RAILYARD_PROXY_HOST", [0, 0, 0, 0].into(), "an IP address")?;
        let proxy_port: u16 = parsed_env("RAILYARD_PROXY_PORT", 3000, "a port number")?;
        let api_host: IpAddr =
            parsed_env("RAILYARD_API_HOST", [127, 0, 0, 1].into(), "an IP address")?;
        let api_port: u16 = parsed_env("RAILYARD_API_PORT", 3001, "a port number")?;
        let ip = public_ip()?;
        let base_hostname = format!("{}.nip.io", ip.to_string().replace('.', "-"));
        let api_hostname = format!("railyard.{base_hostname}");
        let server_url = match proxy_port {
            80 => format!("http://{api_hostname}"),
            _ => format!("http://{api_hostname}:{proxy_port}"),
        };

        Ok(Self {
            proxy_addr: SocketAddr::from((proxy_host, proxy_port)),
            api_addr: SocketAddr::from((api_host, api_port)),
            server_url,
            api_hostname,
            service_upstreams: Arc::new(configured_service_upstreams()?),
        })
    }
}

fn public_ip() -> io::Result<Ipv4Addr> {
    if let Ok(value) = env::var("RAILYARD_PUBLIC_IP") {
        return value
            .parse()
            .map_err(|_| invalid_env("RAILYARD_PUBLIC_IP", &value, "an IPv4 address"));
    }

    if let Ok(ip) = outbound_ip()
        && is_public(ip)
    {
        return Ok(ip);
    }

    discover_public_ip()
}

/// The address selected by the machine's default IPv4 route. Connecting a
/// UDP socket sends no packets; it only asks the kernel which source address
/// it would use.
fn outbound_ip() -> io::Result<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("8.8.8.8:80")?;
    match socket.local_addr()?.ip() {
        IpAddr::V4(ip) => Ok(ip),
        IpAddr::V6(_) => Err(io::Error::other(
            "default IPv4 route returned an IPv6 address",
        )),
    }
}

fn is_public(ip: Ipv4Addr) -> bool {
    let [a, b, c, d] = ip.octets();
    !(a == 0
        || a == 10
        || (a == 100 && (64..=127).contains(&b))
        || a == 127
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && c == 0 && d != 9 && d != 10)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn discover_public_ip() -> io::Result<Ipv4Addr> {
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| io::Error::other(format!("could not prepare public IP lookup: {error}")))?
        .get(PUBLIC_IP_URL)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| {
            io::Error::other(format!(
                "could not discover this machine's public IP ({error}); set RAILYARD_PUBLIC_IP explicitly"
            ))
        })?;
    let value = response.text().map_err(|error| {
        io::Error::other(format!(
            "could not read the public IP response ({error}); set RAILYARD_PUBLIC_IP explicitly"
        ))
    })?;

    let ip: Ipv4Addr = value.trim().parse().map_err(|_| {
        io::Error::other(format!(
            "public IP lookup returned {value:?}; set RAILYARD_PUBLIC_IP explicitly"
        ))
    })?;
    if !is_public(ip) {
        return Err(io::Error::other(format!(
            "public IP lookup returned non-public address {ip}; set RAILYARD_PUBLIC_IP explicitly"
        )));
    }
    Ok(ip)
}

const UPSTREAM_ENV_PREFIX: &str = "RAILYARD_CONTAINER_UPSTREAM_";

fn configured_service_upstreams() -> io::Result<BTreeMap<String, SocketAddr>> {
    env::vars()
        .filter(|(key, _)| key.starts_with(UPSTREAM_ENV_PREFIX))
        .map(|(key, value)| {
            let service = env_key_to_service_name(&key[UPSTREAM_ENV_PREFIX.len()..]);
            let upstream = value
                .parse()
                .map_err(|_| invalid_env(&key, &value, "a socket address"))?;
            Ok((service, upstream))
        })
        .collect()
}

pub(crate) fn parsed_env<T: FromStr>(name: &str, default: T, expected: &str) -> io::Result<T> {
    match env::var(name) {
        Ok(value) => value
            .parse()
            .map_err(|_| invalid_env(name, &value, expected)),
        Err(_) => Ok(default),
    }
}

fn invalid_env(name: &str, value: &str, expected: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{name} must be {expected}, got {value:?}"),
    )
}

fn env_key_to_service_name(key: &str) -> String {
    key.to_ascii_lowercase().replace('_', "-")
}
