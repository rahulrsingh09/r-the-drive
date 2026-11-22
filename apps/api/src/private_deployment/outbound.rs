//! Deployment policy for telemetry-only Rivian access and optional egress.
//! Saved settings cannot relax this policy. Provider tokens are NOT read-only.

#[cfg(test)]
#[path = "full_replacement_key_custody_outbound_tests.rs"]
mod full_replacement_key_custody_tests;

use crate::{
    errors::AppError,
    services::external_connections::{OPEN_METEO, RIVIAN_ACCOUNT, S3_BACKUP},
};
use std::{sync::OnceLock, time::Duration};

pub const GATEWAY_URL: &str = "https://rivian.com/api/gql/gateway/graphql";
pub const CHARGING_URL: &str = "https://rivian.com/api/gql/chrg/user/graphql";
pub const WEBSOCKET_URL: &str = "wss://api.rivian.com/gql-consumer-subscriptions/graphql";

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("Rivian destination rejected by telemetry-only policy")]
    Destination,
    #[error("Vehicle changes are disabled by telemetry-only policy")]
    VehicleWrite,
    #[error("Optional external traffic is disabled by telemetry-only policy")]
    OptionalTraffic,
    #[error("Vehicle artwork destination rejected by telemetry-only policy")]
    ArtworkDestination,
    #[error("Outbound redirects are disabled")]
    Redirect,
    #[error("Upstream response exceeds the size limit")]
    ResponseTooLarge,
    #[error("Invalid upstream JSON")]
    Json(#[from] serde_json::Error),
    #[error("Upstream HTTP request failed")]
    Http(#[from] reqwest::Error),
}

pub fn connection_allowed(id: &str) -> bool {
    [RIVIAN_ACCOUNT, S3_BACKUP, OPEN_METEO].contains(&id)
}

pub fn require_connection_allowed(id: &str) -> Result<(), AppError> {
    connection_allowed(id)
        .then_some(())
        .ok_or_else(|| AppError::ExternalConnectionDisabled(id.to_owned()))
}

pub fn optional_traffic_allowed() -> bool {
    false
}

pub fn require_optional_traffic() -> Result<(), PolicyError> {
    Err(PolicyError::OptionalTraffic)
}

/// The one optional Rivian exchange the owner approved for this deployment:
/// the vehicle-artwork manifest query (`getVehicleOrderMobileImages` and
/// `getVehicleMobileImages`) over the existing Rivian account connection, then
/// a download of the listed images so the server mirrors them and the browser
/// loads them only from this origin. It does not relax
/// `optional_traffic_allowed()`: every other optional provider stays denied.
pub fn vehicle_artwork_allowed() -> bool {
    true
}

pub const VEHICLE_IMAGES_OPERATION: &str = "getVehicleImages";

/// The exact approved query. It selects only the two artwork root fields.
pub const VEHICLE_IMAGES_QUERY: &str = r#"
      query getVehicleImages($extension: String, $resolution: String, $versionForVehicle: String, $versionForPreOrder: String) {
        getVehicleOrderMobileImages(resolution: $resolution, extension: $extension, version: $versionForPreOrder) {
          ...image
        }
        getVehicleMobileImages(resolution: $resolution, extension: $extension, version: $versionForVehicle) {
          ...image
        }
      }

      fragment image on VehicleMobileImage {
          orderId
          vehicleId
          url
          extension
          resolution
          size
          design
          placement
          overlays {
            url
            overlay
            zIndex
          }
      }
    "#;

/// Builds the approved artwork request body. Only the art version changes.
pub fn vehicle_images_request(version: &str) -> Result<serde_json::Value, PolicyError> {
    require_vehicle_artwork()?;
    require_telemetry_query(VEHICLE_IMAGES_QUERY)?;
    if !matches!(version, "2" | "3") {
        return Err(PolicyError::OptionalTraffic);
    }
    Ok(serde_json::json!({
        "operationName": VEHICLE_IMAGES_OPERATION,
        "query": VEHICLE_IMAGES_QUERY,
        "variables": {
            "extension": serde_json::Value::Null,
            "resolution": "@3x",
            "versionForVehicle": version,
            "versionForPreOrder": version
        }
    }))
}

fn require_vehicle_artwork() -> Result<(), PolicyError> {
    if vehicle_artwork_allowed() {
        Ok(())
    } else {
        Err(PolicyError::OptionalTraffic)
    }
}

/// Artwork files listed by the manifest may come only from Rivian's own HTTPS
/// hosts on the default port. IP literals, credentials, alternate ports,
/// fragments and lookalike hosts are rejected before any DNS lookup.
pub fn validate_vehicle_artwork_url(value: &str) -> Result<url::Url, PolicyError> {
    require_vehicle_artwork()?;
    let url = url::Url::parse(value).map_err(|_| PolicyError::ArtworkDestination)?;
    let host_allowed = match url.host() {
        Some(url::Host::Domain(host)) => host == "rivian.com" || host.ends_with(".rivian.com"),
        _ => false,
    };
    if url.scheme() != "https"
        || !host_allowed
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return Err(PolicyError::ArtworkDestination);
    }
    Ok(url)
}

/// A DNS-pinned client for one artwork file: public addresses only, no proxy,
/// no redirects, bounded timeouts.
pub async fn vehicle_artwork_client(
    value: &str,
) -> Result<(url::Url, reqwest::Client), PolicyError> {
    let url = validate_vehicle_artwork_url(value)?;
    let client = crate::services::outbound::outbound_client_for_url(&url, &[])
        .await
        .map_err(|_| PolicyError::ArtworkDestination)?;
    Ok((url, client))
}

/// Mirrored files are served back from this origin, so only raster image
/// types are kept. SVG and HTML are rejected.
pub fn vehicle_artwork_content_type(value: Option<&str>) -> Result<&'static str, PolicyError> {
    let essence = value
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match essence.as_str() {
        "image/webp" => Ok("image/webp"),
        "image/png" => Ok("image/png"),
        "image/jpeg" | "image/jpg" => Ok("image/jpeg"),
        "image/gif" => Ok("image/gif"),
        _ => Err(PolicyError::ArtworkDestination),
    }
}

pub async fn optional_client_for_url(
    url: &url::Url,
    allowlist: &[ipnet::IpNet],
) -> Result<reqwest::Client, crate::errors::AppError> {
    let _ = (url, allowlist);
    Err(crate::errors::AppError::ExternalConnectionDisabled(
        "optional providers".into(),
    ))
}

/// No operation-name exception: a mutation disguised as a telemetry operation
/// is still denied. Auth mutations use the private, fixed auth flow.
pub fn require_telemetry_query(query: &str) -> Result<(), PolicyError> {
    let mut words = query
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|word| !word.is_empty());
    match words.next() {
        Some("query") if !words.any(|word| matches!(word, "mutation" | "subscription")) => Ok(()),
        _ => Err(PolicyError::VehicleWrite),
    }
}

pub fn validate_http_destination(url: &str) -> Result<(), PolicyError> {
    // Exact strings also exclude userinfo, alternate ports, fragments, query
    // parameters, encoded paths and lookalike hosts before adding credentials.
    if matches!(url, GATEWAY_URL | CHARGING_URL) {
        Ok(())
    } else {
        Err(PolicyError::Destination)
    }
}

pub fn validate_ws_destination(url: &str) -> Result<(), PolicyError> {
    if url == WEBSOCKET_URL {
        Ok(())
    } else {
        Err(PolicyError::Destination)
    }
}

pub fn gateway_url() -> Result<String, PolicyError> {
    #[cfg(test)]
    if let Ok(url) = MOCK_GATEWAY.try_with(Clone::clone) {
        return Ok(url);
    }
    let configured = std::env::var("RIVIAN_GRAPHQL_GATEWAY_URL").ok();
    validated_gateway_url(configured.unwrap_or_else(|| GATEWAY_URL.to_owned()))
}

fn validated_gateway_url(url: String) -> Result<String, PolicyError> {
    if url != GATEWAY_URL {
        return Err(PolicyError::Destination);
    }
    Ok(url)
}

/// Keep client ownership here: caller-supplied redirect/proxy policies must
/// never affect requests containing Rivian passwords or session headers.
pub fn rivian_post(url: &str) -> Result<reqwest::RequestBuilder, PolicyError> {
    let is_mock = {
        #[cfg(test)]
        {
            MOCK_GATEWAY.try_with(|mock| mock == url).unwrap_or(false)
        }
        #[cfg(not(test))]
        {
            false
        }
    };
    if !is_mock {
        validate_http_destination(url)?;
        // Unit tests never contact Rivian, including background tasks (such
        // as artwork repair) that run outside a mock-gateway scope.
        #[cfg(test)]
        return Err(PolicyError::Destination);
    }
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    Ok(CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("fixed Rivian HTTP client")
        })
        .post(url))
}

pub async fn read_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, PolicyError> {
    let bytes = read_limited(response, 8 * 1024 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub async fn read_limited(
    response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, PolicyError> {
    use futures::TryStreamExt;
    if response.status().is_redirection() {
        return Err(PolicyError::Redirect);
    }
    response.error_for_status_ref()?;
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(PolicyError::ResponseTooLarge);
    }
    futures::stream::try_unfold((response, limit), |(mut response, remaining)| async move {
        match response.chunk().await? {
            None => Ok(None),
            Some(chunk) => {
                let remaining = remaining
                    .checked_sub(chunk.len())
                    .ok_or(PolicyError::ResponseTooLarge)?;
                Ok::<_, PolicyError>(Some((chunk, (response, remaining))))
            }
        }
    })
    .try_fold(Vec::new(), |mut body, chunk| async move {
        body.extend_from_slice(&chunk);
        Ok(body)
    })
    .await
}

// Compiled only into unit tests; task scoping prevents concurrent tests from
// changing one another's destinations. No environment switch enables mocks.
#[cfg(test)]
tokio::task_local! {
    static MOCK_GATEWAY: String;
}

#[cfg(test)]
pub async fn with_mock_gateway<F: std::future::Future>(url: String, future: F) -> F::Output {
    let parsed = url::Url::parse(&url).expect("mock URL");
    assert_eq!(parsed.scheme(), "http");
    assert_eq!(parsed.username(), "");
    assert!(parsed.password().is_none());
    assert!(parsed
        .host_str()
        .and_then(|host| host.parse::<std::net::IpAddr>().ok())
        .is_some_and(|ip| ip.is_loopback()));
    MOCK_GATEWAY.scope(url, future).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::StatusCode, response::Response, routing::post, Router};
    use tokio::net::TcpListener;

    #[test]
    fn exact_rivian_destinations_exclude_url_confusion() {
        for url in [GATEWAY_URL, CHARGING_URL] {
            assert!(validate_http_destination(url).is_ok());
        }
        assert!(validate_ws_destination(WEBSOCKET_URL).is_ok());
        assert!(validated_gateway_url(GATEWAY_URL.into()).is_ok());
        assert!(validated_gateway_url(CHARGING_URL.into()).is_err());
        assert!(validated_gateway_url("http://127.0.0.1:8080/graphql".into()).is_err());
        for url in [
            "http://rivian.com/api/gql/gateway/graphql",
            "https://rivian.com.evil.test/api/gql/gateway/graphql",
            "https://rivian.com@evil.test/api/gql/gateway/graphql",
            "https://user:password@rivian.com/api/gql/gateway/graphql",
            "https://rivian.com:444/api/gql/gateway/graphql",
            "https://rivian.com/api/gql/gateway/graphql?redirect=evil",
            "https://rivian.com/api/gql/gateway/graphql#fragment",
            "https://rivian.com/api/gql/gateway/%67raphql",
            "https://127.0.0.1/api/gql/gateway/graphql",
            "https://[::ffff:127.0.0.1]/graphql",
            "https://169.254.169.254/latest/meta-data",
            "https://rivian.com\\@evil.test/graphql",
        ] {
            assert!(validate_http_destination(url).is_err(), "{url}");
            assert!(rivian_post(url).is_err(), "{url}");
            assert!(validated_gateway_url(url.into()).is_err(), "{url}");
        }
        for url in [
            "ws://api.rivian.com/gql-consumer-subscriptions/graphql",
            "wss://api.rivian.com@evil.test/gql-consumer-subscriptions/graphql",
            "wss://user:password@api.rivian.com/gql-consumer-subscriptions/graphql",
            "wss://api.rivian.com.evil.test/gql-consumer-subscriptions/graphql",
            "wss://api.rivian.com/gql-consumer-subscriptions/graphql?token=x",
        ] {
            assert!(validate_ws_destination(url).is_err());
        }
    }

    #[test]
    fn optional_traffic_stays_denied_outside_the_artwork_exception() {
        use crate::services::external_connections::{
            BASEMAP, ICONIFY, NOMINATIM, OPTIONAL_CONNECTIONS,
        };
        assert!(!optional_traffic_allowed());
        assert!(matches!(
            require_optional_traffic(),
            Err(PolicyError::OptionalTraffic)
        ));
        for id in [NOMINATIM, BASEMAP, ICONIFY] {
            assert!(!connection_allowed(id), "{id}");
            assert!(require_connection_allowed(id).is_err(), "{id}");
        }
        assert!(OPTIONAL_CONNECTIONS
            .iter()
            .filter(|id| **id != OPEN_METEO)
            .all(|id| !connection_allowed(id)));
        assert!(vehicle_artwork_allowed());
    }

    #[tokio::test]
    async fn optional_provider_clients_stay_denied() {
        let url = url::Url::parse("https://tiles.openfreemap.org/styles/liberty").unwrap();
        assert!(optional_client_for_url(&url, &[]).await.is_err());
    }

    #[test]
    fn artwork_request_is_the_fixed_image_query_only() {
        for version in ["2", "3"] {
            let body = vehicle_images_request(version).unwrap();
            assert_eq!(body["operationName"], VEHICLE_IMAGES_OPERATION);
            assert_eq!(body["query"], VEHICLE_IMAGES_QUERY);
            assert_eq!(body["variables"]["versionForVehicle"], version);
        }
        assert!(require_telemetry_query(VEHICLE_IMAGES_QUERY).is_ok());
        let roots: Vec<_> = VEHICLE_IMAGES_QUERY
            .lines()
            .map(str::trim)
            .filter(|line| line.ends_with('{') && line.contains('('))
            .filter_map(|line| line.split('(').next())
            .collect();
        assert_eq!(
            roots,
            [
                "query getVehicleImages",
                "getVehicleOrderMobileImages",
                "getVehicleMobileImages"
            ]
        );
        for version in ["", "1", "4", "2 } mutation x {", "3 "] {
            assert!(vehicle_images_request(version).is_err(), "{version}");
        }
    }

    #[test]
    fn artwork_downloads_accept_only_rivian_https_hosts() {
        for url in [
            "https://rivian.com/mobile/static/img/v1/vehicle/r2/side_light.webp",
            "https://assets.rivian.com/mobile/r2.png",
            "https://RIVIAN.com:443/mobile/static/img/example.webp",
        ] {
            assert!(validate_vehicle_artwork_url(url).is_ok(), "{url}");
        }
        for url in [
            "http://rivian.com/mobile/static/img/example.webp",
            "https://rivian.com.evil.test/example.webp",
            "https://evilrivian.com/example.webp",
            "https://rivian.com./example.webp",
            "https://rivian.com@evil.test/example.webp",
            "https://user:password@rivian.com/example.webp",
            "https://rivian.com:444/example.webp",
            "https://rivian.com/example.webp#fragment",
            "https://127.0.0.1/example.webp",
            "https://[::1]/example.webp",
            "https://169.254.169.254/latest/meta-data",
            "https://tiles.openfreemap.org/styles/liberty",
            "https://images.example.test/vehicle.webp",
            "file:///etc/passwd",
            "not a url",
        ] {
            assert!(
                matches!(
                    validate_vehicle_artwork_url(url),
                    Err(PolicyError::ArtworkDestination)
                ),
                "{url}"
            );
        }
    }

    #[test]
    fn artwork_files_keep_only_raster_image_types() {
        assert_eq!(
            vehicle_artwork_content_type(Some("image/webp")).unwrap(),
            "image/webp"
        );
        assert_eq!(
            vehicle_artwork_content_type(Some("Image/PNG; charset=binary")).unwrap(),
            "image/png"
        );
        for value in [
            None,
            Some("image/svg+xml"),
            Some("text/html"),
            Some("application/octet-stream"),
        ] {
            assert!(vehicle_artwork_content_type(value).is_err(), "{value:?}");
        }
    }

    #[test]
    fn vehicle_mutations_cannot_hide_behind_query_operation_names() {
        assert!(
            require_telemetry_query("query getUserInfo { currentUser { vehicles { id } } }")
                .is_ok()
        );
        for query in [
            "mutation getUserInfo { updateVehicleChargingSettings { __typename } }",
            "query safe { currentUser { id } } mutation getUserInfo { deleteDepartureSchedule }",
            "# query\nmutation getUserInfo { createDepartureSchedule }",
            "mutation Login { updateDepartureSchedule }",
            "subscription getUserInfo { vehicleState }",
            "{ updateVehicleChargingSettings }",
        ] {
            assert!(matches!(
                require_telemetry_query(query),
                Err(PolicyError::VehicleWrite)
            ));
        }
    }

    #[tokio::test]
    async fn credential_requests_never_follow_redirects() {
        let sink = TcpListener::bind("127.0.0.1:0").await.unwrap();
        for code in [301, 302, 303, 307, 308] {
            let destination = format!("http://{}/stolen", sink.local_addr().unwrap());
            let app = Router::new().route(
                "/graphql",
                post(move || async move {
                    Response::builder()
                        .status(code)
                        .header("location", destination)
                        .body(Body::empty())
                        .unwrap()
                }),
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/graphql", listener.local_addr().unwrap());
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            with_mock_gateway(url.clone(), async {
                let response = rivian_post(&url)
                    .unwrap()
                    .header("A-Sess", "synthetic-app")
                    .header("U-Sess", "synthetic-user")
                    .header("Csrf-Token", "synthetic-csrf")
                    .bearer_auth("synthetic-access")
                    .json(&serde_json::json!({"password": "synthetic-password"}))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response.status().as_u16(), code);
                assert!(matches!(
                    read_limited(response, 1024).await,
                    Err(PolicyError::Redirect)
                ));
            })
            .await;
            server.abort();
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), sink.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn response_limits_cover_content_length_and_chunked_bodies() {
        for chunked in [false, true] {
            let app = Router::new().route(
                "/graphql",
                post(move || async move {
                    if chunked {
                        let stream =
                            futures::stream::iter([Ok::<_, std::io::Error>("12345678"), Ok("9")]);
                        Response::new(Body::from_stream(stream))
                    } else {
                        Response::new(Body::from("123456789"))
                    }
                }),
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/graphql", listener.local_addr().unwrap());
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            with_mock_gateway(url.clone(), async {
                let response = rivian_post(&url).unwrap().send().await.unwrap();
                assert!(matches!(
                    read_limited(response, 8).await,
                    Err(PolicyError::ResponseTooLarge)
                ));
                let response = rivian_post(&url).unwrap().send().await.unwrap();
                assert_eq!(read_limited(response, 9).await.unwrap(), b"123456789");
            })
            .await;
            server.abort();
        }
    }

    #[tokio::test]
    async fn websocket_handshake_rejects_redirect_without_contacting_target() {
        let sink = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let destination = format!("ws://{}/stolen", sink.local_addr().unwrap());
        let app = Router::new().route(
            "/graphql",
            axum::routing::get(move || async move {
                Response::builder()
                    .status(StatusCode::TEMPORARY_REDIRECT)
                    .header("location", destination)
                    .body(Body::empty())
                    .unwrap()
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/graphql", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = url.into_client_request().unwrap();
        request
            .headers_mut()
            .insert("A-Sess", "synthetic-app".parse().unwrap());
        let error = tokio_tungstenite::connect_async(request).await.unwrap_err();
        assert!(
            matches!(error, tokio_tungstenite::tungstenite::Error::Http(response)
            if response.status() == StatusCode::TEMPORARY_REDIRECT)
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(100), sink.accept())
                .await
                .is_err()
        );
        server.abort();
    }
}
