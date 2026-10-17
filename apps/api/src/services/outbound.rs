//! Resolve and pin approved socket addresses for every outbound request.
use crate::{errors::AppError, services::external_connections::ConnectionSettingsRow};
use ipnet::IpNet;
use std::{
    net::{IpAddr, SocketAddr},
    str::FromStr,
    time::Duration,
};
use url::Url;
pub async fn read_response_limited(
    response: reqwest::Response,
    limit: usize,
    label: &str,
) -> Result<Vec<u8>, AppError> {
    let exceeded =
        || AppError::DependencyUnavailable(format!("{label} response exceeded the limit"));
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(exceeded());
    }
    let mut response = response;
    let mut remaining = limit;
    let mut bytes = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                remaining = remaining.checked_sub(chunk.len()).ok_or_else(exceeded)?;
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => return Ok(bytes),
            Err(_) => {
                return Err(AppError::DependencyUnavailable(format!(
                    "{label} response failed"
                )))
            }
        }
    }
}

pub fn parse_private_network_allowlist(values: &[String]) -> Result<Vec<IpNet>, AppError> {
    let mut networks = std::collections::BTreeMap::new();
    for value in values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        let network: IpNet = value
            .parse()
            .map_err(|_| AppError::Validation(format!("invalid private-network CIDR `{value}`")))?;
        networks.insert(network.to_string(), network);
    }
    networks.into_values().map(|network| {
        if [network.network(), network.broadcast()].into_iter().all(is_private_ip) {
            Ok(network)
        } else {
            Err(AppError::Validation(format!(
                "private-network CIDR `{network}` must be contained entirely in RFC1918 or IPv6 ULA space"
            )))
        }
    }).collect()
}

pub fn configured_private_network_allowlist(
    settings: &ConnectionSettingsRow,
) -> Result<Vec<IpNet>, AppError> {
    if settings.private_network_policy_state == "migration_required" {
        return Err(AppError::Validation(
            "private-network access is disabled until an administrator confirms explicit CIDR allowlists".into(),
        ));
    }
    parse_private_network_allowlist(&settings.private_network_allowlist)
}

pub fn endpoint_is_private(value: &str) -> bool {
    Url::parse(value).ok().is_some_and(|url| {
        url.host_str().is_some_and(|host| {
            if host.eq_ignore_ascii_case("localhost") {
                return true;
            }
            match host.parse::<IpAddr>() {
                Ok(IpAddr::V4(ip)) => ip.is_private() || ip.is_loopback(),
                Ok(IpAddr::V6(ip)) => ip.is_loopback() || is_ipv6_unique_local(ip),
                Err(_) => false,
            }
        })
    })
}

pub fn is_link_local_or_metadata(host: &str) -> bool {
    if host.eq_ignore_ascii_case("metadata.google.internal") {
        return true;
    }
    IpAddr::from_str(host).map(is_forbidden_ip).unwrap_or(false)
}

pub fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let address = u32::from(ip);
            let ranges = [
                (0x0000_0000, 8),
                (0x6440_0000, 10),
                (0x7f00_0000, 8),
                (0xa9fe_0000, 16),
                (0xc000_0000, 24),
                (0xc000_0200, 24),
                (0xc058_6300, 24),
                (0xc0af_3000, 24),
                (0xc612_0000, 15),
                (0xc633_6400, 24),
                (0xcb00_7100, 24),
                (0xe000_0000, 3),
            ];
            ranges
                .into_iter()
                .any(|(network, bits)| address >> (32 - bits) == network >> (32 - bits))
        }
        IpAddr::V6(ip) => {
            let words = ip.segments();
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_forbidden_ip(IpAddr::V4(mapped));
            }
            if is_ipv6_unique_local(ip) {
                return false;
            }
            if words[0] & 0xe000 != 0x2000 {
                return true;
            }
            match words[0] {
                0x2001 => words[1] == 0x0db8 || words[1] & 0xfe00 == 0,
                0x2002 | 0x3fff => true,
                _ => false,
            }
        }
    }
}

pub fn is_private_ip(ip: IpAddr) -> bool {
    let ip = canonical_ip(ip);
    match ip {
        IpAddr::V4(ip) => ip.is_private(),
        IpAddr::V6(ip) => is_ipv6_unique_local(ip),
    }
}

pub fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(ip)),
        other => other,
    }
}

pub fn operator_security() -> Result<crate::config::SecurityConfig, AppError> {
    envy::from_env()
        .map_err(|_| AppError::Validation("Invalid operator outbound configuration".into()))
}

pub fn operator_allowlist(value: &str) -> Result<Vec<IpNet>, AppError> {
    parse_private_network_allowlist(&value.split(',').map(str::to_owned).collect::<Vec<_>>())
}

pub async fn read_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    limit: usize,
    label: &str,
) -> Result<T, AppError> {
    let bytes = read_response_limited(response, limit, label).await?;
    serde_json::from_slice(&bytes)
        .map_err(|_| AppError::DependencyUnavailable(format!("{label} returned invalid JSON")))
}

pub fn is_ipv6_unique_local(ip: std::net::Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xfe00) == 0xfc00
}

pub fn is_ipv6_unicast_link_local(ip: std::net::Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xffc0) == 0xfe80
}

pub async fn outbound_client_for_url(
    url: &Url,
    allowlist: &[IpNet],
) -> Result<reqwest::Client, AppError> {
    let destination = ApprovedDestination::resolve(url, allowlist).await?;
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .resolve_to_addrs(destination.host, &destination.addresses)
        .build()
        .map_err(|error| AppError::Internal(error.into()))
}

pub fn validate_addresses(addresses: &[SocketAddr], allowlist: &[IpNet]) -> Result<bool, AppError> {
    let Some(_) = addresses.first() else {
        return Err(AppError::DependencyUnavailable(
            "No outbound addresses resolved".into(),
        ));
    };
    let mut family = None;
    let mut mixed = false;
    for address in addresses {
        let ip = address.ip();
        if is_forbidden_ip(ip) {
            return Err(AppError::Validation(
                "connection endpoint resolved to a forbidden address".into(),
            ));
        }
        let private = is_private_ip(ip);
        if private
            && !allowlist
                .iter()
                .any(|network| network.contains(&canonical_ip(ip)))
        {
            return Err(AppError::Validation(
                "connection endpoint resolved outside its private CIDR allowlist".into(),
            ));
        }
        mixed |= family.is_some_and(|prior| prior != private);
        family = Some(private);
    }
    if mixed {
        Err(AppError::Validation(
            "connection endpoint DNS returned mixed public and private addresses".into(),
        ))
    } else {
        Ok(family.unwrap_or(false))
    }
}

struct ApprovedDestination<'a> {
    host: &'a str,
    addresses: Vec<SocketAddr>,
}

impl<'a> ApprovedDestination<'a> {
    async fn resolve(url: &'a Url, allowlist: &[IpNet]) -> Result<Self, AppError> {
        let clean =
            url.username().is_empty() && url.password().is_none() && url.fragment().is_none();
        if !clean || !["http", "https"].contains(&url.scheme()) {
            return Err(AppError::Validation(
                "Outbound endpoint must use HTTP(S) without credentials or fragments".into(),
            ));
        }
        let host = url
            .host_str()
            .ok_or_else(|| AppError::Validation("connection endpoint host is required".into()))?;
        if is_link_local_or_metadata(host) {
            return Err(AppError::Validation(
                "link-local and cloud metadata endpoints are not allowed".into(),
            ));
        }
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let port = url
            .port_or_known_default()
            .ok_or_else(|| AppError::Validation("connection endpoint port is required".into()))?;
        let unresolved =
            || AppError::DependencyUnavailable("connection endpoint could not be resolved".into());
        let addresses: Vec<_> = match tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::lookup_host((host, port)),
        )
        .await
        {
            Err(_) => {
                return Err(AppError::DependencyUnavailable(
                    "connection endpoint DNS timed out".into(),
                ))
            }
            Ok(Err(_)) => return Err(unresolved()),
            Ok(Ok(addresses)) => addresses.collect(),
        };
        if addresses.is_empty() {
            return Err(unresolved());
        }
        match (url.scheme(), validate_addresses(&addresses, allowlist)?) {
            ("http", false) => Err(AppError::Validation(
                "HTTP is permitted only for an explicitly allowed private destination".into(),
            )),
            _ => Ok(Self { host, addresses }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::IntoFuture;
    #[test]
    fn classifies_every_resolved_address_and_mapped_ipv4() {
        let allowlist = vec!["192.168.1.0/24".parse().unwrap()];
        let private = "192.168.1.10:443".parse().unwrap();
        let public = "8.8.8.8:443".parse().unwrap();
        assert!(validate_addresses(&[private], &allowlist).unwrap());
        assert!(validate_addresses(&[private], &[]).is_err());
        assert!(validate_addresses(&[private, public], &allowlist).is_err());
        assert!(
            validate_addresses(&["[::ffff:192.168.1.10]:443".parse().unwrap()], &allowlist)
                .unwrap()
        );
        for address in [
            "127.0.0.1:443",
            "169.254.169.254:443",
            "[::ffff:127.0.0.1]:443",
            "[64:ff9b::a9fe:a9fe]:443",
            "[2002:7f00:1::]:443",
        ] {
            assert!(
                validate_addresses(&[address.parse().unwrap()], &allowlist).is_err(),
                "{address}"
            );
        }
    }
    #[tokio::test]
    async fn counts_chunked_response_bytes_without_content_length() {
        use axum::{
            body::{Body, Bytes},
            routing::get,
            Router,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(
            axum::serve(
                listener,
                Router::new().route(
                    "/",
                    get(|| async {
                        Body::from_stream(futures::stream::iter([
                            Ok::<_, std::io::Error>(Bytes::from_static(b"12345")),
                            Ok(Bytes::from_static(b"67890")),
                        ]))
                    }),
                ),
            )
            .into_future(),
        );
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{addr}/"))
            .send()
            .await
            .unwrap();
        assert!(read_response_limited(response, 8, "fixture").await.is_err());
        server.abort();
    }
}
