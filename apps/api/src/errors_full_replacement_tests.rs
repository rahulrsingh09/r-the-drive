use super::*;

#[tokio::test]
async fn every_error_keeps_its_status_complete_json_and_headers() {
    let cases = vec![
        (AppError::NotFound, 404, "NOT_FOUND", "Not found"),
        (AppError::Unauthorized, 401, "UNAUTHORIZED", "Unauthorized"),
        (AppError::Forbidden, 403, "FORBIDDEN", "Forbidden"),
        (AppError::ResourceLimited("metrics"), 429, "RATE_LIMITED", "Server capacity reached. Please retry shortly."),
        (AppError::SetupProofRequired, 403, "SETUP_PROOF_REQUIRED", "First-owner setup requires the configured setup token"),
        (AppError::SetupProofInvalid, 403, "SETUP_PROOF_INVALID", "The first-owner setup token is invalid"),
        (AppError::Conflict("detail".into()), 409, "CONFLICT", "detail"),
        (AppError::RecoveryConflict("detail".into()), 409, "RECOVERY_CONFLICT", "detail"),
        (AppError::RecoveryTooLarge("detail".into()), 413, "RECOVERY_TOO_LARGE", "detail"),
        (AppError::RecoveryInvalid("detail".into()), 422, "RECOVERY_INVALID", "detail"),
        (AppError::RecoveryInsufficientStorage("detail".into()), 507, "RECOVERY_INSUFFICIENT_STORAGE", "detail"),
        (AppError::RecoveryDeadline("detail".into()), 504, "RECOVERY_DEADLINE", "detail"),
        (AppError::Database(sqlx::Error::RowNotFound), 500, "INTERNAL", "Database error"),
        (AppError::Io(std::io::Error::other("private failure")), 500, "INTERNAL", "Filesystem error"),
        (AppError::RivianApi("detail".into()), 502, "RIVIAN_API", "detail"),
        (AppError::RivianCredentialsRejected, 422, "RIVIAN_CREDENTIALS_REJECTED", "Rivian did not accept that email or password."),
        (AppError::RivianOtpRejected, 422, "RIVIAN_OTP_REJECTED", "Rivian did not accept that verification code."),
        (AppError::RivianConnectSessionExpired, 409, "RIVIAN_CONNECT_SESSION_EXPIRED", "Your secure Rivian sign-in session expired. Start again from the account step."),
        (AppError::Validation("detail".into()), 422, "VALIDATION", "detail"),
        (AppError::DependencyUnavailable("detail".into()), 503, "DEPENDENCY_UNAVAILABLE", "detail"),
        (AppError::ExternalConnectionDisabled("detail".into()), 409, "EXTERNAL_CONNECTION_DISABLED", "detail"),
        (AppError::Internal(anyhow::anyhow!("private failure")), 500, "INTERNAL", "Internal server error"),
        (AppError::Redis(redis::RedisError::from((redis::ErrorKind::Io, "private failure"))), 503, "DEPENDENCY_UNAVAILABLE", "Temporary session storage is unavailable. Please try again."),
    ];
    for (error, status, code, message) in cases {
        let response = error.into_response();
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(response.headers().get("content-type").unwrap(), "application/json");
        if status == 429 {
            assert_eq!(response.headers().get("retry-after").unwrap(), "5");
            assert_eq!(response.headers().get("x-ratelimit-source").unwrap(), "application");
            assert_eq!(response.headers().get("x-ratelimit-class").unwrap(), "metrics");
        } else {
            assert!(!response.headers().contains_key("retry-after"));
            assert!(!response.headers().contains_key("x-ratelimit-source"));
            assert!(!response.headers().contains_key("x-ratelimit-class"));
        }
        let body = axum::body::to_bytes(response.into_body(), 16_384).await.unwrap();
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&body).unwrap(), json!({"error": {"code": code, "message": message}}));
    }
}
