use super::*;
use serde_json::json;

#[tokio::test]
async fn outcome_and_retry_after_contract() {
    let mut cases = Vec::new();
    for base in [0, 1100] {
        for max in [0, 15000] {
            let s = NominatimScheduler::new(base, max);
            for (status, retry) in [
                (None, None),
                (Some(500), None),
                (Some(429), None),
                (Some(429), None),
                (Some(200), None),
                (Some(429), Some(0)),
                (Some(429), Some(2)),
                (Some(429), Some(60)),
                (Some(299), None),
                (Some(300), None),
                (Some(429), None),
            ] {
                s.report_outcome(status, retry.map(Duration::from_secs))
                    .await;
                cases.push(json!({"base":base,"max":max,"status":status,"retry":retry,"extra":s.adaptive_extra_ms().await}));
            }
        }
    }
    for value in [
        "0",
        "1",
        " 2 ",
        "-1",
        "+2",
        "1.5",
        "Wed, 21 Oct 2015 07:28:00 GMT",
        "18446744073709551615",
        "18446744073709551616",
        "",
    ] {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, value.parse().unwrap());
        cases.push(json!({"header":value,"seconds":retry_after_from_headers(&headers).map(|d|d.as_secs())}));
    }
    if let Some(dir) = std::env::var_os("PLACES_EVIDENCE_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join("scheduler-actual.json"),
            serde_json::to_string_pretty(&cases).unwrap(),
        )
        .unwrap();
    }
    if std::env::var_os("PLACES_CAPTURE").is_none() {
        assert_eq!(
            json!(cases),
            serde_json::from_str::<Value>(include_str!("scheduler.json")).unwrap()
        );
    }
}

#[tokio::test]
async fn admission_priority_and_cancellation_contract() {
    let s = Arc::new(NominatimScheduler::new(20, 15000));
    let first = s
        .acquire_slot(NominatimLane::BackgroundReverseGeocode)
        .await;
    assert_eq!(first.effective_interval_ms, 20);
    assert_eq!(first.gate_wait_ms, 0);
    let next = s.acquire_slot(NominatimLane::InteractiveSearch).await;
    assert_eq!(next.lane, NominatimLane::InteractiveSearch);
    assert!(next.gate_wait_ms > 0);
    assert_eq!(s.interactive_waiters.load(Ordering::SeqCst), 0);
    s.state.lock().await.next_allowed = Instant::now() + Duration::from_secs(60);
    let queued = s.clone();
    let task =
        tokio::spawn(async move { queued.acquire_slot(NominatimLane::InteractiveSearch).await });
    while s.interactive_waiters.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    task.abort();
    let _ = task.await;
    assert_eq!(
        s.interactive_waiters.load(Ordering::SeqCst),
        1,
        "baseline cancellation retains waiter accounting"
    );
    assert!(tokio::time::timeout(
        Duration::from_millis(60),
        s.acquire_slot(NominatimLane::BackgroundReverseGeocode)
    )
    .await
    .is_err());
}
use std::sync::Arc;
