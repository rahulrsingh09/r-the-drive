use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
#[cfg(test)]
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Not found")]
    NotFound,
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Forbidden")]
    Forbidden,
    #[error("Resource capacity reached: {0}")]
    ResourceLimited(&'static str),
    #[error("First-owner setup requires the configured setup token")]
    SetupProofRequired,
    #[error("The first-owner setup token is invalid")]
    SetupProofInvalid,
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Recovery operation conflict: {0}")]
    RecoveryConflict(String),
    #[error("Recovery package too large: {0}")]
    RecoveryTooLarge(String),
    #[error("Invalid recovery package: {0}")]
    RecoveryInvalid(String),
    #[error("Insufficient recovery storage: {0}")]
    RecoveryInsufficientStorage(String),
    #[error("Recovery operation deadline exceeded: {0}")]
    RecoveryDeadline(String),
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Rivian API error: {0}")]
    RivianApi(String),
    #[error("Rivian rejected the account credentials")]
    RivianCredentialsRejected,
    #[error("Rivian rejected the verification code")]
    RivianOtpRejected,
    #[error("Rivian connection session expired")]
    RivianConnectSessionExpired,
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Dependency unavailable: {0}")]
    DependencyUnavailable(String),
    #[error("External connection disabled: {0}")]
    ExternalConnectionDisabled(String),
    #[error("Internal error")]
    Internal(#[from] anyhow::Error),
    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),
}

#[derive(serde::Serialize)]
struct ErrorEnvelope<'a> {
    error: PublicError<'a>,
}

#[derive(serde::Serialize)]
struct PublicError<'a> {
    code: &'static str,
    message: std::borrow::Cow<'a, str>,
}

impl AppError {
    fn response_identity(&self) -> (StatusCode, &'static str) {
        use AppError::*;
        match self {
            NotFound => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            Unauthorized => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED"),
            Forbidden => (StatusCode::FORBIDDEN, "FORBIDDEN"),
            ResourceLimited(_) => (StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED"),
            SetupProofRequired => (StatusCode::FORBIDDEN, "SETUP_PROOF_REQUIRED"),
            SetupProofInvalid => (StatusCode::FORBIDDEN, "SETUP_PROOF_INVALID"),
            Conflict(_) => (StatusCode::CONFLICT, "CONFLICT"),
            RecoveryConflict(_) => (StatusCode::CONFLICT, "RECOVERY_CONFLICT"),
            RecoveryTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, "RECOVERY_TOO_LARGE"),
            RecoveryInvalid(_) => (StatusCode::UNPROCESSABLE_ENTITY, "RECOVERY_INVALID"),
            RecoveryInsufficientStorage(_) => (
                StatusCode::INSUFFICIENT_STORAGE,
                "RECOVERY_INSUFFICIENT_STORAGE",
            ),
            RecoveryDeadline(_) => (StatusCode::GATEWAY_TIMEOUT, "RECOVERY_DEADLINE"),
            Validation(_) => (StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION"),
            DependencyUnavailable(_) | Redis(_) => {
                (StatusCode::SERVICE_UNAVAILABLE, "DEPENDENCY_UNAVAILABLE")
            }
            ExternalConnectionDisabled(_) => (StatusCode::CONFLICT, "EXTERNAL_CONNECTION_DISABLED"),
            RivianApi(_) => (StatusCode::BAD_GATEWAY, "RIVIAN_API"),
            RivianCredentialsRejected => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "RIVIAN_CREDENTIALS_REJECTED",
            ),
            RivianOtpRejected => (StatusCode::UNPROCESSABLE_ENTITY, "RIVIAN_OTP_REJECTED"),
            RivianConnectSessionExpired => (StatusCode::CONFLICT, "RIVIAN_CONNECT_SESSION_EXPIRED"),
            Database(_) | Io(_) | Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL"),
        }
    }

    fn public_message(self) -> std::borrow::Cow<'static, str> {
        use std::borrow::Cow::{Borrowed, Owned};
        use AppError::*;
        let message = match self {
            Conflict(message)
            | RecoveryConflict(message)
            | RecoveryTooLarge(message)
            | RecoveryInvalid(message)
            | RecoveryInsufficientStorage(message)
            | RecoveryDeadline(message)
            | Validation(message)
            | DependencyUnavailable(message)
            | ExternalConnectionDisabled(message)
            | RivianApi(message) => return Owned(message),
            NotFound => "Not found",
            Unauthorized => "Unauthorized",
            Forbidden => "Forbidden",
            SetupProofRequired => "First-owner setup requires the configured setup token",
            SetupProofInvalid => "The first-owner setup token is invalid",
            ResourceLimited(_) => "Server capacity reached. Please retry shortly.",
            RivianCredentialsRejected => "Rivian did not accept that email or password.",
            RivianOtpRejected => "Rivian did not accept that verification code.",
            RivianConnectSessionExpired => {
                "Your secure Rivian sign-in session expired. Start again from the account step."
            }
            Io(error) => {
                tracing::error!(err = %error, "io error");
                "Filesystem error"
            }
            Redis(error) => {
                tracing::error!(err = %error, "redis error");
                "Temporary session storage is unavailable. Please try again."
            }
            Database(error) => {
                tracing::error!(err = %error, "database error");
                "Database error"
            }
            Internal(error) => {
                tracing::error!(err = %error, "internal error");
                "Internal server error"
            }
        };
        Borrowed(message)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = self.response_identity();
        let capacity_class = match &self {
            Self::ResourceLimited(class) => Some(*class),
            _ => None,
        };
        let payload = ErrorEnvelope {
            error: PublicError {
                code,
                message: self.public_message(),
            },
        };
        let mut response = (status, Json(payload)).into_response();
        if let Some(class) = capacity_class {
            for (name, value) in [
                ("retry-after", "5"),
                ("x-ratelimit-source", "application"),
                ("x-ratelimit-class", class),
            ] {
                response.headers_mut().insert(name, value.parse().unwrap());
            }
        }
        response
    }
}

#[cfg(test)]
#[path = "errors_full_replacement_tests.rs"]
mod full_replacement_tests;
