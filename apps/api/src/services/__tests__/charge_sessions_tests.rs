use super::*;
use chrono::TimeZone;

#[test]
fn normalizes_grouping_keys_stably() {
    assert_eq!(
        normalize_grouping_key(Some("VTXN-2, TXN-1,TXN-1")),
        Some("TXN-1,VTXN-2".to_string())
    );
}

#[test]
fn prefers_txn_alias_over_vtxn_alias() {
    let summary = ChargeSessionSummaryPayload {
        transaction_id: Some("d284".to_string()),
        start_instant: Some(Utc.with_ymd_and_hms(2026, 6, 20, 15, 17, 0).unwrap()),
        end_instant: Some(Utc.with_ymd_and_hms(2026, 6, 20, 16, 1, 0).unwrap()),
        charger_type: Some("dcfc".to_string()),
        currency_code: Some("USD".to_string()),
        total_energy_kwh: Some(45.2),
        range_added_km: Some(120.0),
        city: Some("Austin".to_string()),
        vehicle_id: Some("veh-1".to_string()),
        vehicle_name: Some("R1S".to_string()),
        vendor: Some("Rivian".to_string()),
        paid_total: Some(18.4),
        is_public: Some(true),
        is_home_charger: Some(false),
        is_roaming_network: Some(false),
        meta: Some(ChargeSessionSummaryMeta {
            transaction_id_grouping_key: Some("8fbf,d284".to_string()),
            data_sources: Some(vec![
                "VTXN:d284".to_string(),
                "TXN:8fbf".to_string(),
                "CDRS:USCPI123".to_string(),
            ]),
        }),
    };

    let aliases = parse_summary_aliases(&summary);
    let preferred = preferred_external_alias(&aliases).expect("preferred alias");
    assert_eq!(preferred.alias_kind, ExternalAliasKind::Cdrs);
    assert_eq!(preferred.external_id, "USCPI123");
    assert!(aliases
        .iter()
        .any(|alias| alias.alias_kind == ExternalAliasKind::Txn));
    assert!(aliases
        .iter()
        .any(|alias| alias.alias_kind == ExternalAliasKind::Vtxn));
}

#[test]
fn recent_only_insert_policy_respects_lookback() {
    let now = Utc.with_ymd_and_hms(2026, 6, 21, 12, 0, 0).unwrap();
    assert!(should_insert_unmatched_session(
        UnmatchedInsertPolicy::RecentOnly {
            now,
            lookback_days: 30,
        },
        Some(Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap()),
    ));
    assert!(!should_insert_unmatched_session(
        UnmatchedInsertPolicy::RecentOnly {
            now,
            lookback_days: 30,
        },
        Some(Utc.with_ymd_and_hms(2026, 4, 10, 12, 0, 0).unwrap()),
    ));
}
