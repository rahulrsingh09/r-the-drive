use panda_api::ingestion::session_store::{decrypt_json, encrypt_json};
use serde_json::{json, Value};

#[test]
fn age_dependency_preserves_encrypted_json_round_trip() {
    let identity = age::x25519::Identity::generate();
    let payload = json!({"fixture": "synthetic dependency regression", "revision": 1});
    let ciphertext = encrypt_json(&payload, &identity).unwrap();
    assert_ne!(ciphertext, serde_json::to_vec(&payload).unwrap());
    assert_eq!(
        decrypt_json::<Value>(&ciphertext, &identity).unwrap(),
        payload
    );
}

#[test]
fn age_dependency_rejects_wrong_recipient_and_modified_ciphertext() {
    let identity = age::x25519::Identity::generate();
    let other_identity = age::x25519::Identity::generate();
    let mut ciphertext = encrypt_json(&json!({"fixture": "synthetic"}), &identity).unwrap();
    assert!(decrypt_json::<Value>(&ciphertext, &other_identity).is_err());
    *ciphertext.last_mut().unwrap() ^= 1;
    assert!(decrypt_json::<Value>(&ciphertext, &identity).is_err());
    assert!(decrypt_json::<Value>(&ciphertext[..ciphertext.len() / 2], &identity).is_err());
}
