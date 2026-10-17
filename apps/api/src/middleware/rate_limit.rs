use std::net::{IpAddr, SocketAddr};

use http::{HeaderMap, Request};
use jsonwebtoken::DecodingKey;
use sha2::{Digest, Sha256};
use tower_governor::{key_extractor::KeyExtractor, GovernorError};

use crate::middleware::auth::{bearer_token, decode_access_claims};

#[derive(Clone, Debug)]
pub struct TrustedProxyIpKeyExtractor;

#[derive(Clone)]
pub struct AuthIdentityKeyExtractor {
    decoding_key: DecodingKey,
}

#[derive(Clone, Copy, Debug)]
pub enum RateLimitClass {
    AuthPublic,
    AuthMetadata,
    AuthRead,
    AuthWrite,
    HeavyRead,
}

impl RateLimitClass {
    pub fn as_header_value(self) -> &'static str {
        match self {
            Self::AuthPublic => "auth_public",
            Self::AuthMetadata => "auth_metadata",
            Self::AuthRead => "auth_read",
            Self::AuthWrite => "auth_write",
            Self::HeavyRead => "heavy_read",
        }
    }
}

impl AuthIdentityKeyExtractor {
    pub fn new(decoding_key: DecodingKey) -> Self {
        Self { decoding_key }
    }
}

enum LimitIdentity<'a> {
    Integration(&'a str),
    User(uuid::Uuid),
    Peer,
}

impl<'a> LimitIdentity<'a> {
    fn select(headers: &'a HeaderMap, key: &DecodingKey) -> Self {
        match bearer_token(headers) {
            Some(token) if token.starts_with("panda_") => Self::Integration(token),
            Some(token) => {
                decode_access_claims(token, key).map_or(Self::Peer, |claims| Self::User(claims.sub))
            }
            None => Self::Peer,
        }
    }
}

impl KeyExtractor for TrustedProxyIpKeyExtractor {
    type Key = String;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        trusted_client_ip(req)
            .map(|ip| format!("ip:{ip}"))
            .ok_or(GovernorError::UnableToExtractKey)
    }
}

impl KeyExtractor for AuthIdentityKeyExtractor {
    type Key = String;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        match LimitIdentity::select(req.headers(), &self.decoding_key) {
            LimitIdentity::Integration(token) => Ok(format!("api:{}", hash_token(token))),
            LimitIdentity::User(user) => Ok(format!("user:{user}")),
            LimitIdentity::Peer => TrustedProxyIpKeyExtractor.extract(req),
        }
    }
}

pub fn infer_key_type(headers: &HeaderMap, decoding_key: &DecodingKey) -> &'static str {
    match LimitIdentity::select(headers, decoding_key) {
        LimitIdentity::Integration(_) => "api_key",
        LimitIdentity::User(_) => "jwt_user",
        LimitIdentity::Peer => "ip_fallback",
    }
}

fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn trusted_client_ip<T>(req: &Request<T>) -> Option<IpAddr> {
    match req
        .extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
    {
        None => Some(IpAddr::from([127, 0, 0, 1])),
        Some(peer) if peer.ip().is_loopback() => {
            parse_forwarded_ip(req.headers()).or_else(|| Some(peer.ip()))
        }
        Some(peer) => Some(peer.ip()),
    }
}

fn parse_forwarded_ip(headers: &HeaderMap) -> Option<IpAddr> {
    for name in ["x-real-ip", "x-forwarded-for"] {
        if let Some(ip) = headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse().ok())
        {
            return Some(ip);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;

    #[test]
    fn trusted_proxy_key_extractor_falls_back_without_connect_info() {
        let extractor = TrustedProxyIpKeyExtractor;
        let req = Request::builder()
            .uri("/v1/charging/chart-series")
            .body(())
            .unwrap();

        let key = extractor.extract(&req).expect("key should be extractable");

        assert_eq!(key, "ip:127.0.0.1");
    }

    #[test]
    fn trusted_client_ip_falls_back_without_connect_info() {
        let req = Request::builder()
            .uri("/v1/charging/chart-series")
            .body(())
            .unwrap();

        assert_eq!(trusted_client_ip(&req), Some(IpAddr::from([127, 0, 0, 1])));
    }
}
