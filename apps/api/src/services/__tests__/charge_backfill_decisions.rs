use super::*;

#[test]
fn stale_admission_retains_strict_database_and_application_clock_boundaries() {
    let now: DateTime<Utc> = "2026-01-01T12:00:00Z".parse().unwrap();
    let vehicle = Uuid::from_u128(1);
    let row = |updated_at: Option<DateTime<Utc>>, credentials: bool| AdmissionRow {
        rivian_vehicle_id: "synthetic".into(),
        encrypted_tokens: credentials.then(|| vec![0]),
        history_backfill_status: Some("running".into()),
        updated_at,
        database_now: now,
    };
    assert!(matches!(
        Admission::decide(vehicle, None, now),
        Admission::MissingCredentials
    ));
    assert!(matches!(
        Admission::decide(vehicle, Some(row(None, true)), now),
        Admission::MissingCredentials
    ));
    let boundary = now - Duration::minutes(BACKFILL_STALE_AFTER_MINUTES);
    assert!(matches!(
        Admission::decide(vehicle, Some(row(Some(boundary), true)), now),
        Admission::MissingCredentials
    ));
    assert!(matches!(
        Admission::decide(
            vehicle,
            Some(row(Some(boundary - Duration::microseconds(1)), true)),
            now
        ),
        Admission::Claimed(_)
    ));
    assert!(matches!(
        Admission::decide(
            vehicle,
            Some(row(Some(boundary + Duration::microseconds(1)), false)),
            now
        ),
        Admission::Running
    ));
    assert!(matches!(
        Admission::decide(
            vehicle,
            Some(row(Some(boundary - Duration::microseconds(1)), false)),
            now
        ),
        Admission::MissingCredentials
    ));
}
