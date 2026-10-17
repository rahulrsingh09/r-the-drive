use super::*;

#[tokio::test]
async fn full_replacement_key_custody_optional_traffic_always_denied() {
    assert!(!optional_traffic_allowed());
    assert!(matches!(
        require_optional_traffic(),
        Err(PolicyError::OptionalTraffic)
    ));
    for id in [RIVIAN_ACCOUNT, S3_BACKUP, OPEN_METEO] {
        assert!(connection_allowed(id));
        assert!(require_connection_allowed(id).is_ok());
    }
    for id in ["", "unknown", "nominatim", "RIVIAN_ACCOUNT"] {
        assert!(!connection_allowed(id));
        assert!(matches!(
            require_connection_allowed(id),
            Err(AppError::ExternalConnectionDisabled(_))
        ));
    }
    for target in [
        "http://127.0.0.1/",
        "http://10.0.0.1/",
        "https://example.invalid/",
    ] {
        let url = url::Url::parse(target).unwrap();
        assert!(matches!(
            optional_client_for_url(&url, &["10.0.0.0/8".parse().unwrap()]).await,
            Err(AppError::ExternalConnectionDisabled(_))
        ));
    }
}
