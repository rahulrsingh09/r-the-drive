//! Encrypts/decrypts Rivian credential bundles using the `age` crate.

#[cfg(test)]
#[path = "full_replacement_key_custody_tests.rs"]
mod full_replacement_key_custody_tests;

use age::x25519;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::io::Read;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RivianTokenBundle {
    pub access_token: String,
    pub refresh_token: String,
    pub app_session_token: String,
    pub user_session_token: String,
    pub csrf_token: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl RivianTokenBundle {
    /// Returns an error if any token field that is strictly required for API
    /// calls is empty.
    ///
    /// Only `access_token` and `refresh_token` are required:
    /// - `access_token`: needed as the Bearer token for every Rivian GQL call.
    /// - `refresh_token`: retained for future use; token renewal today works by
    ///   refreshing the CSRF session via `createCsrfToken` (see `rivian_refresh_csrf`).
    ///
    /// `user_session_token`, `app_session_token`, and `csrf_token` are NOT
    /// required here because they are refreshed on demand via `createCsrfToken`
    /// when the session expires.
    ///
    /// Call this immediately after decrypting a bundle to catch genuinely
    /// corrupted blobs early, without rejecting valid bundles that happen to
    /// have an empty `user_session_token`.
    pub fn validate(&self) -> anyhow::Result<()> {
        for (name, value) in [
            ("access_token", &self.access_token),
            ("refresh_token", &self.refresh_token),
        ] {
            anyhow::ensure!(!value.is_empty(), "RivianTokenBundle: {name} is empty");
        }
        Ok(())
    }

    pub(super) fn has_same_session_snapshot(&self, other: &Self) -> bool {
        self.created_at == other.created_at
            && [
                (&self.app_session_token, &other.app_session_token),
                (&self.user_session_token, &other.user_session_token),
                (&self.csrf_token, &other.csrf_token),
            ]
            .into_iter()
            .all(|(left, right)| left == right)
    }
}

pub fn encrypt_json<T: Serialize>(
    value: &T,
    identity: &x25519::Identity,
) -> anyhow::Result<Vec<u8>> {
    let public_key = identity.to_public();
    let mut ciphertext = Vec::new();
    {
        let recipients = std::iter::once(&public_key as &dyn age::Recipient);
        let mut output =
            age::Encryptor::with_recipients(recipients)?.wrap_output(&mut ciphertext)?;
        serde_json::to_writer(&mut output, value)?;
        output.finish()?;
    }
    Ok(ciphertext)
}

pub fn decrypt_json<T: DeserializeOwned>(
    ciphertext: &[u8],
    identity: &x25519::Identity,
) -> anyhow::Result<T> {
    let identities = std::iter::once(identity as &dyn age::Identity);
    let input = age::Decryptor::new(ciphertext)?.decrypt(identities)?;
    // from_reader checks EOF, including the final authenticated AGE chunk.
    Ok(serde_json::from_reader(input)?)
}

pub fn encrypt_tokens(
    bundle: &RivianTokenBundle,
    identity: &x25519::Identity,
) -> anyhow::Result<Vec<u8>> {
    encrypt_json(bundle, identity)
}

pub fn decrypt_tokens(
    ciphertext: &[u8],
    identity: &x25519::Identity,
) -> anyhow::Result<RivianTokenBundle> {
    decrypt_json(ciphertext, identity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_contract_tokens_keep_optional_sessions_and_validation_precedence() {
        let mut tokens = RivianTokenBundle {
            access_token: String::new(),
            refresh_token: String::new(),
            app_session_token: String::new(),
            user_session_token: String::new(),
            csrf_token: String::new(),
            created_at: chrono::Utc::now(),
        };
        assert_eq!(
            tokens.validate().unwrap_err().to_string(),
            "RivianTokenBundle: access_token is empty"
        );
        tokens.access_token = "synthetic".into();
        assert_eq!(
            tokens.validate().unwrap_err().to_string(),
            "RivianTokenBundle: refresh_token is empty"
        );
        tokens.refresh_token = "synthetic".into();
        tokens.validate().unwrap();
        let identity = x25519::Identity::generate();
        let recovered =
            decrypt_tokens(&encrypt_tokens(&tokens, &identity).unwrap(), &identity).unwrap();
        assert_eq!(
            serde_json::to_value(&tokens).unwrap(),
            serde_json::to_value(recovered).unwrap()
        );
        let mut changed = tokens.clone();
        changed.access_token = "other access".into();
        assert!(tokens.has_same_session_snapshot(&changed));
        changed.created_at += chrono::Duration::seconds(1);
        assert!(!tokens.has_same_session_snapshot(&changed));
    }
}
