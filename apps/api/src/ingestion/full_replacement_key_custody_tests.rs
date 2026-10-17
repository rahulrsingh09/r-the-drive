use super::*;
use age::secrecy::ExposeSecret;
use serde_json::{json, Value};

fn synthetic_bundle() -> RivianTokenBundle {
    RivianTokenBundle {
        access_token: "synthetic-access".into(),
        refresh_token: "synthetic-refresh".into(),
        app_session_token: "synthetic-app".into(),
        user_session_token: String::new(),
        csrf_token: "synthetic-csrf".into(),
        created_at: "2026-01-02T03:04:05Z".parse().unwrap(),
    }
}

#[test]
fn full_replacement_key_custody_authenticated_stream_and_snapshot_fields() {
    let identity = x25519::Identity::generate();
    let tokens = synthetic_bundle();
    let encrypted = encrypt_tokens(&tokens, &identity).unwrap();
    let wrong = x25519::Identity::generate();
    assert!(decrypt_tokens(&encrypted, &wrong).is_err());
    for length in [0, 1, encrypted.len() / 2, encrypted.len() - 1] {
        assert!(decrypt_tokens(&encrypted[..length], &identity).is_err());
    }
    let mut corrupt = encrypted.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(decrypt_tokens(&corrupt, &identity).is_err());
    for field in [
        "app_session_token",
        "user_session_token",
        "csrf_token",
        "created_at",
    ] {
        let mut changed = serde_json::to_value(&tokens).unwrap();
        changed[field] = if field == "created_at" {
            json!("2026-01-02T03:04:06Z")
        } else {
            json!("changed")
        };
        let changed: RivianTokenBundle = serde_json::from_value(changed).unwrap();
        assert!(!tokens.has_same_session_snapshot(&changed), "{field}");
    }
    let mut changed = tokens.clone();
    changed.access_token.clear();
    changed.refresh_token.clear();
    assert!(tokens.has_same_session_snapshot(&changed));

    let large = json!({"synthetic": "x".repeat(150_000)});
    let ciphertext = encrypt_json(&large, &identity).unwrap();
    assert_eq!(
        decrypt_json::<Value>(&ciphertext, &identity).unwrap(),
        large
    );
    let mut corrupt = ciphertext;
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(decrypt_json::<Value>(&corrupt, &identity).is_err());
}

#[test]
fn full_replacement_key_custody_age_archive_interoperability() {
    let identity = x25519::Identity::generate();
    let tokens = synthetic_bundle();
    let plaintext = serde_json::to_vec(&tokens).unwrap();
    let backup = crate::private_deployment::history_backup::encrypt(
        &plaintext,
        identity.to_string().expose_secret(),
    )
    .unwrap();
    let restored = decrypt_tokens(&backup, &identity).unwrap();
    assert_eq!(
        serde_json::to_value(restored).unwrap(),
        serde_json::to_value(tokens).unwrap()
    );
    let ciphertext = encrypt_json(&json!({"synthetic": true}), &identity).unwrap();
    let mut reader = age::Decryptor::new(ciphertext.as_slice())
        .unwrap()
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .unwrap();
    let mut plaintext = String::new();
    reader.read_to_string(&mut plaintext).unwrap();
    assert_eq!(plaintext, r#"{"synthetic":true}"#);
}

#[test]
#[ignore = "explicit baseline/candidate cross-version evidence directory"]
fn full_replacement_key_custody_cross_version_ciphertext() {
    let directory = std::path::PathBuf::from(std::env::var("CUSTODY_COMPATIBILITY_DIR").unwrap());
    let expected = synthetic_bundle();
    if std::env::var("CUSTODY_CAPTURE_BASELINE").as_deref() == Ok("yes") {
        assert!(
            !directory.exists(),
            "never overwrite baseline compatibility fixture"
        );
        std::fs::create_dir(&directory).unwrap();
        let identity = x25519::Identity::generate();
        std::fs::write(
            directory.join("synthetic-age.txt"),
            identity.to_string().expose_secret(),
        )
        .unwrap();
        std::fs::write(
            directory.join("baseline-tokens.age"),
            encrypt_tokens(&expected, &identity).unwrap(),
        )
        .unwrap();
        std::fs::write(
            directory.join("baseline-backup.age"),
            crate::private_deployment::history_backup::encrypt(
                &serde_json::to_vec(&expected).unwrap(),
                identity.to_string().expose_secret(),
            )
            .unwrap(),
        )
        .unwrap();
    }
    let identity: x25519::Identity = std::fs::read_to_string(directory.join("synthetic-age.txt"))
        .unwrap()
        .parse()
        .unwrap();
    for name in ["baseline-tokens.age", "baseline-backup.age"] {
        let bytes = std::fs::read(directory.join(name)).unwrap();
        let restored = decrypt_tokens(&bytes, &identity).unwrap();
        assert_eq!(
            serde_json::to_value(&restored).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
    }
}
