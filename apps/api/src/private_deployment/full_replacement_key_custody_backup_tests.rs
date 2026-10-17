use super::*;
use axum::{body::Bytes, http::StatusCode, routing::post, Json, Router};
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[tokio::test]
async fn full_replacement_key_custody_backup_receipts_and_token_denials() {
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let app = Router::new().route(
        "/{case}",
        post(
            move |axum::extract::Path(case): axum::extract::Path<String>, bytes: Bytes| {
                let seen = seen.clone();
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    let checksum = hex::encode(Sha256::digest(&bytes));
                    let mut receipt =
                        json!({"key":"daily/30","sha256":checksum,"bytes":bytes.len()});
                    match case.as_str() {
                        "slot31" => receipt["key"] = json!("daily/31"),
                        "slot1" => receipt["key"] = json!("daily/1"),
                        "slotpath" => receipt["key"] = json!("daily/../00"),
                        "checksum" => receipt["sha256"] = json!("incorrect"),
                        "size" => receipt["bytes"] = json!(bytes.len() + 1),
                        _ => {}
                    }
                    (StatusCode::CREATED, Json(receipt))
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    for token in [
        "A".repeat(64),
        "0".repeat(63),
        "g".repeat(64),
        "0".repeat(65),
    ] {
        assert!(publish(vec![1, 2], &token, &format!("{base}/ok"))
            .await
            .is_err());
    }
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    let token = "a".repeat(64);
    for case in ["slot31", "slot1", "slotpath", "checksum", "size"] {
        assert!(
            publish(vec![1, 2], &token, &format!("{base}/{case}"))
                .await
                .is_err(),
            "{case}"
        );
    }
    publish(vec![1, 2], &token, &format!("{base}/ok"))
        .await
        .unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 6);
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
