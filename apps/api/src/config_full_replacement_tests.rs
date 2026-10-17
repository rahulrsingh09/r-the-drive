use super::*;

fn config() -> Config {
    envy::from_iter([
        ("DATABASE_URL".to_owned(), "postgresql://unused".to_owned()),
        ("REDIS_URL".to_owned(), "redis://unused".to_owned()),
        ("PANDA_ENV".to_owned(), "development".to_owned()),
    ]).expect("synthetic configuration")
}

#[test]
fn cookie_and_key_source_truth_tables_are_complete() {
    for environment in [None, Some("development"), Some("DEVELOPMENT"), Some("production"), Some("invalid")] {
        for legacy in [None, Some(false), Some(true)] {
            for lan in [false, true] {
                let mut value = config();
                value.panda_env = environment.map(str::to_owned);
                value.cookie_insecure = legacy;
                value.allow_insecure_lan_http_auth = lan;
                assert_eq!(value.allows_insecure_refresh_cookies(), lan || (environment.is_some_and(|v| v.eq_ignore_ascii_case("development")) && legacy == Some(true)));
                for bits in 0..8 {
                    value.jwt_secret = (bits & 1 != 0).then(|| "private".into());
                    value.jwt_public_key = (bits & 2 != 0).then(|| "public".into());
                    value.age_encryption_key = (bits & 4 != 0).then(|| "age".into());
                    assert_eq!(value.cryptographic_key_source(), if bits == 7 { "external" } else { "database" });
                }
            }
        }
    }
}

#[test]
fn recovery_validation_checks_every_limit_and_error_precedence() {
    let zeroes: [fn(&mut RecoveryConfig); 10] = [
        |v| v.max_upload_bytes = 0, |v| v.max_expanded_bytes = 0,
        |v| v.max_manifest_bytes = 0, |v| v.max_settings_bytes = 0,
        |v| v.max_history_bytes = 0, |v| v.max_member_bytes = 0,
        |v| v.max_members = 0, |v| v.max_compression_ratio = 0,
        |v| v.upload_deadline_seconds = 0, |v| v.restore_deadline_seconds = 0,
    ];
    for alter in zeroes {
        let mut value = config(); alter(&mut value.recovery);
        value.recovery.min_free_bytes = 0;
        assert_eq!(value.validate_recovery_limits().unwrap_err().to_string(), "recovery resource limits and deadlines must be greater than zero");
    }
    let mut value = config();
    assert!(value.validate_recovery_limits().is_ok());
    value.recovery.max_member_bytes = value.recovery.max_expanded_bytes + 1;
    value.recovery.min_free_bytes = 0;
    assert_eq!(value.validate_recovery_limits().unwrap_err().to_string(), "RECOVERY_MAX_MEMBER_BYTES cannot exceed RECOVERY_MAX_EXPANDED_BYTES");
    value.recovery.max_member_bytes = 1;
    assert_eq!(value.validate_recovery_limits().unwrap_err().to_string(), "RECOVERY_MIN_FREE_BYTES cannot be less than 2 GiB");
}

#[test]
fn rate_limit_validation_retains_order_and_messages() {
    type RateLimitCase = (&'static str, fn(&mut RateLimitConfig, bool));
    let fields: [RateLimitCase; 5] = [
        ("RATE_LIMIT_AUTH_PUBLIC_PER_MINUTE", |v, burst| if burst { v.auth_public_burst = 0 } else { v.auth_public_per_minute = 0 }),
        ("RATE_LIMIT_AUTH_METADATA_PER_MINUTE", |v, burst| if burst { v.auth_metadata_burst = 0 } else { v.auth_metadata_per_minute = 0 }),
        ("RATE_LIMIT_AUTH_READ_PER_MINUTE", |v, burst| if burst { v.auth_read_burst = 0 } else { v.auth_read_per_minute = 0 }),
        ("RATE_LIMIT_AUTH_WRITE_PER_MINUTE", |v, burst| if burst { v.auth_write_burst = 0 } else { v.auth_write_per_minute = 0 }),
        ("RATE_LIMIT_HEAVY_READ_PER_MINUTE", |v, burst| if burst { v.heavy_read_burst = 0 } else { v.heavy_read_per_minute = 0 }),
    ];
    for (name, change) in fields {
        for burst in [false, true] {
            let mut value = RateLimitConfig::default(); change(&mut value, burst);
            assert_eq!(value.validate().unwrap_err().to_string(), format!("{name}{} must be greater than 0", if burst { " burst size" } else { "" }));
        }
    }
}

#[test]
fn bind_and_client_policy_classification_is_fixed() {
    for (address, client, bind) in [
        ("0.0.0.0", false, true), ("127.0.0.1", true, false),
        ("10.1.2.3", true, true), ("172.16.0.1", true, true),
        ("192.168.0.1", true, true), ("169.254.1.1", true, true),
        ("100.64.0.1", false, false), ("8.8.8.8", false, false),
        ("::", false, true), ("::1", true, false), ("fc00::1", true, true),
        ("fe80::1", true, true), ("2001:db8::1", false, false),
        ("::ffff:127.0.0.1", false, false),
    ] {
        let address = address.parse().unwrap();
        assert_eq!(is_lan_client_address(address), client);
        assert_eq!(is_lan_accessible_bind_address(address), bind);
    }
}

#[test]
fn key_sources_newlines_and_redis_encoding_preserve_complete_values() {
    for (input, expected) in [("a", "a"), ("a\n", "a"), ("a\r\n", "a"), ("a\n\n", "a\n"), ("a\r", "a\r"), ("\n", "")] {
        assert_eq!(trim_one_trailing_line_ending(input), expected);
    }
    assert_eq!(resolve_key_source("EXAMPLE", None, None).unwrap(), None);
    assert_eq!(resolve_key_source("EXAMPLE", Some("direct\n".into()), None).unwrap().as_deref(), Some("direct\n"));
    assert_eq!(resolve_key_source("EXAMPLE", Some("v".into()), Some("f".into())).unwrap_err().to_string(), "EXAMPLE and EXAMPLE_FILE are mutually exclusive");
    assert_eq!(resolve_key_source("EXAMPLE", None, Some(" ".into())).unwrap_err().to_string(), "EXAMPLE_FILE must name a readable key file");
    for password in ["", "a", " :/@?#%+é\n"] {
        let encoded = compose_redis_url(password).unwrap();
        let details = redis::IntoConnectionInfo::into_connection_info(encoded).unwrap();
        assert_eq!(details.redis_settings().username(), Some("default"));
        assert_eq!(details.redis_settings().password().unwrap_or_default(), password);
    }
}

#[test]
fn textual_field_deserialization_keeps_representations_and_errors() {
    for text in ["1", "8", "128"] {
        let value: SecurityConfig = serde_json::from_value(serde_json::json!({"ws_max_per_user": text})).unwrap();
        assert_eq!(value.ws_max_per_user, text.parse::<usize>().unwrap());
    }
    for (value, expected) in [
        (serde_json::json!("bad"), "invalid value \"bad\": invalid digit found in string"),
        (serde_json::json!("-1"), "invalid value \"-1\": invalid digit found in string"),
        (serde_json::json!(8), "invalid type: integer `8`, expected a string containing a value"),
        (serde_json::json!(true), "invalid type: boolean `true`, expected a string containing a value"),
        (serde_json::Value::Null, "invalid type: null, expected a string containing a value"),
    ] {
        let result = serde_json::from_value::<SecurityConfig>(serde_json::json!({"ws_max_per_user": value}));
        assert_eq!(result.unwrap_err().to_string(), expected);
    }
}
