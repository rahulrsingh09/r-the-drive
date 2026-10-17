//! SDK transport enforcement, including each retry and literal-IP endpoints.
use crate::{config::Config, errors::AppError, services::outbound};
use aws_smithy_runtime_api::client::{
    dns::{DnsFuture, ResolveDns, ResolveDnsError},
    http::{
        HttpClient, HttpConnector, HttpConnectorFuture, HttpConnectorSettings, SharedHttpConnector,
    },
    orchestrator::HttpRequest,
    result::ConnectorError,
    runtime_components::RuntimeComponents,
};
use ipnet::IpNet;
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use url::Url;

#[derive(Debug, Clone, Default)]
pub struct S3Policy {
    pub allowlist: Vec<IpNet>,
    pub allow_private_http: bool,
    /// Operator-owned development origin only; never restored from a package.
    pub development_origin: Option<Url>,
}
impl S3Policy {
    pub async fn validate_endpoint(&self, endpoint: &str) -> Result<(), AppError> {
        match endpoint.trim() {
            "" => Ok(()),
            endpoint => {
                let url = Url::parse(endpoint)
                    .map_err(|_| AppError::Validation("Invalid S3 endpoint".into()))?;
                self.resolve(&url, false).await.map(|_| ())
            }
        }
    }
    pub fn from_config(config: &Config) -> Result<Self, AppError> {
        let allowlist =
            outbound::operator_allowlist(&config.security.s3_private_network_allowlist)?;
        let development_origin =
            match (
                config.is_production(),
                config.security.s3_allow_development_garage,
                config.s3_endpoint.as_deref(),
            ) {
                (false, true, Some(endpoint)) => Some(Url::parse(endpoint).map_err(|_| {
                    AppError::Validation("Invalid development Garage endpoint".into())
                })?),
                _ => None,
            };
        Ok(Self {
            allowlist,
            development_origin,
            allow_private_http: config.security.s3_allow_insecure_private_http,
        })
    }
    pub fn validate(&self, url: &Url, addresses: &[SocketAddr]) -> Result<(), AppError> {
        let valid_shape = matches!(url.scheme(), "https" | "http")
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none();
        if !valid_shape {
            return Err(AppError::Validation("Invalid S3 endpoint".into()));
        }
        let development = self
            .development_origin
            .as_ref()
            .is_some_and(|base| base.origin() == url.origin());
        if development {
            return match addresses.split_first() {
                Some(_)
                    if addresses.iter().all(|address| {
                        address.ip().is_loopback()
                            || (outbound::is_private_ip(address.ip())
                                && !outbound::is_forbidden_ip(address.ip()))
                    }) =>
                {
                    Ok(())
                }
                _ => Err(AppError::Validation(
                    "Development Garage must resolve to a local/private address".into(),
                )),
            };
        }
        let private = outbound::validate_addresses(addresses, &self.allowlist)?;
        match (url.scheme(), private, self.allow_private_http, self.allowlist.is_empty()) {
            ("https", _, _, _) | ("http", true, true, false) => Ok(()),
            _ => Err(AppError::Validation("S3 requires HTTPS; trusted-LAN HTTP requires an operator exception and CIDR allowlist".into())),
        }
    }
}

#[derive(Debug, Clone)]
struct PinnedDns {
    host: String,
    addresses: Vec<IpAddr>,
}
impl ResolveDns for PinnedDns {
    fn resolve_dns<'a>(&'a self, name: &'a str) -> DnsFuture<'a> {
        DnsFuture::new(async move {
            if name.eq_ignore_ascii_case(&self.host) {
                Ok(self.addresses.clone())
            } else {
                Err(ResolveDnsError::new(std::io::Error::other(
                    "Unexpected S3 transport hostname",
                )))
            }
        })
    }
}

#[derive(Debug, Clone)]
pub struct S3HttpClient(pub S3Policy);
impl HttpClient for S3HttpClient {
    fn http_connector(
        &self,
        settings: &HttpConnectorSettings,
        components: &RuntimeComponents,
    ) -> SharedHttpConnector {
        SharedHttpConnector::new(S3Connector {
            policy: self.0.clone(),
            settings: settings.clone(),
            components: components.clone(),
        })
    }
}
#[derive(Debug, Clone)]
struct S3Connector {
    policy: S3Policy,
    settings: HttpConnectorSettings,
    components: RuntimeComponents,
}
impl HttpConnector for S3Connector {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        let connector = self.clone();
        HttpConnectorFuture::new(async move {
            let target = async {
                let url = Url::parse(request.uri())
                    .map_err(|_| AppError::Validation("Invalid S3 request URL".into()))?;
                connector.policy.resolve(&url, true).await
            }
            .await
            .map_err(|error| ConnectorError::other(Box::new(error), None))?;
            let transport = aws_smithy_http_client::Builder::new()
                .tls_provider(aws_smithy_http_client::tls::Provider::Rustls(
                    aws_smithy_http_client::tls::rustls_provider::CryptoMode::AwsLc,
                ))
                .build_with_resolver(target);
            transport
                .http_connector(&connector.settings, &connector.components)
                .call(request)
                .await
        })
    }
}

impl S3Policy {
    async fn resolve(&self, url: &Url, request: bool) -> Result<PinnedDns, AppError> {
        let host = url
            .host_str()
            .ok_or_else(|| {
                AppError::Validation(
                    if request {
                        "S3 host required"
                    } else {
                        "S3 endpoint needs a host"
                    }
                    .into(),
                )
            })?
            .trim_start_matches('[')
            .trim_end_matches(']');
        let port = url.port_or_known_default().ok_or_else(|| {
            AppError::Validation(
                if request {
                    "S3 port required"
                } else {
                    "Invalid S3 endpoint scheme"
                }
                .into(),
            )
        })?;
        let lookup = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::lookup_host((host, port)),
        )
        .await;
        let resolved = match lookup {
            Ok(Ok(addresses)) => addresses.collect::<Vec<_>>(),
            failure => {
                let timed_out = failure.is_err();
                return Err(match (request, timed_out) {
                    (true, true) => AppError::DependencyUnavailable("S3 DNS timed out".into()),
                    (true, false) => AppError::DependencyUnavailable("S3 DNS failed".into()),
                    (false, true) => {
                        AppError::Validation("S3 endpoint DNS lookup timed out".into())
                    }
                    (false, false) => AppError::Validation("S3 endpoint DNS lookup failed".into()),
                });
            }
        };
        self.validate(url, &resolved)?;
        Ok(PinnedDns {
            host: host.to_owned(),
            addresses: resolved.into_iter().map(|address| address.ip()).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn address(ip: &str) -> SocketAddr {
        SocketAddr::new(ip.parse().unwrap(), 443)
    }
    #[test]
    fn defaults_require_public_https_and_reject_mixed_dns_or_metadata() {
        let policy = S3Policy::default();
        assert!(policy
            .validate(
                &Url::parse("https://s3.example.test").unwrap(),
                &[address("1.1.1.1")]
            )
            .is_ok());
        for (url, ips) in [
            ("http://s3.example.test", vec![address("1.1.1.1")]),
            ("https://s3.example.test", vec![address("10.0.0.1")]),
            (
                "https://s3.example.test",
                vec![address("1.1.1.1"), address("10.0.0.1")],
            ),
            (
                "https://s3.example.test",
                vec![address("::ffff:169.254.169.254")],
            ),
        ] {
            assert!(policy.validate(&Url::parse(url).unwrap(), &ips).is_err());
        }
    }
    #[test]
    fn private_http_requires_both_exception_and_cidr_and_garage_is_exact_origin() {
        let url = Url::parse("http://s3.lan.test:3900").unwrap();
        let mut policy = S3Policy {
            allowlist: vec!["10.0.0.0/24".parse().unwrap()],
            ..Default::default()
        };
        assert!(policy.validate(&url, &[address("10.0.0.2")]).is_err());
        policy.allow_private_http = true;
        assert!(policy.validate(&url, &[address("10.0.0.2")]).is_ok());
        assert!(policy.validate(&url, &[address("10.0.1.2")]).is_err());
        policy = S3Policy {
            development_origin: Some(Url::parse("http://127.0.0.1:3900").unwrap()),
            ..Default::default()
        };
        assert!(policy
            .validate(
                &Url::parse("http://127.0.0.1:3900/bucket").unwrap(),
                &[address("127.0.0.1")]
            )
            .is_ok());
        assert!(policy
            .validate(
                &Url::parse("http://127.0.0.1:3901/bucket").unwrap(),
                &[address("127.0.0.1")]
            )
            .is_err());
        assert!(policy
            .validate(
                &Url::parse("http://127.0.0.1:3900").unwrap(),
                &[address("1.1.1.1")]
            )
            .is_err());
    }
}
