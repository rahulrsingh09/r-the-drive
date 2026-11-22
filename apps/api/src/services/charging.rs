//! Charging computation helpers used by the ingestion worker and routes.

/// Compute energy drawn from grid (Wh) by integrating |power_kw| × Δt samples.
///
/// `samples` is an ordered sequence of `(timestamp_seconds, power_kw)` where
/// power_kw is the AC draw (positive or negative depending on sign convention;
/// this function takes the absolute value).
pub fn compute_energy_used_wh(samples: &[(f64, f64)]) -> f64 {
    let mut previous = None;
    let mut watt_hours = 0.0;
    for &(timestamp, kilowatts) in samples {
        if let Some((at, power)) = previous {
            let hours = (timestamp - at) / 3_600.0;
            watt_hours += f64::abs(power) * 1_000.0 * hours;
        }
        previous = Some((timestamp, kilowatts));
    }
    watt_hours
}

/// Lowest charge rate that only a DC charger can give. AC charging on a J1772
/// or NACS circuit stops at 80 A × 240 V = 19.2 kW, so a higher rate is DC.
pub const DC_MIN_RATE_KW: f64 = 20.0;

/// Highest charge-curve reading that is kept. Higher values are sensor faults.
pub const PLAUSIBLE_MAX_CHARGE_KW: f64 = 300.0;

/// Classify charger type by peak observed power: "ac" or "dc".
pub fn classify_charger_type(peak_kw: f64) -> &'static str {
    if peak_kw < DC_MIN_RATE_KW {
        "ac"
    } else {
        "dc"
    }
}

/// SQL for the one charger-type rule that every read path uses. The argument
/// is the alias of `panda.charge_sessions`. The result is 'dc', 'ac' or
/// NULL:
///
/// 1. A stored DC type, or a DC type from Rivian, is DC.
/// 2. An AC type from Rivian, or a home session, is AC.
/// 3. A stored AC type stays AC unless the energy over duration (an average,
///    which one glitched reading cannot raise) reaches `DC_MIN_RATE_KW`.
/// 4. Without a type, a known rate (peak, then average, then energy over
///    duration) of `DC_MIN_RATE_KW` or more is DC, and a lower rate is AC.
/// 5. With no rate, a DC-only network is DC.
#[macro_export]
macro_rules! charger_type_sql {
    ($cs:literal) => {
        concat!(
            "(CASE WHEN lower(COALESCE(", $cs, ".charger_type, '')) IN ('dc', 'dcfc') THEN 'dc' ",
            "WHEN lower(btrim(COALESCE(", $cs, ".rivian_charger_type, ''))) IN ('dc', 'dcfc', 'ran15_dispenser', 'supercharger') THEN 'dc' ",
            "WHEN lower(btrim(COALESCE(", $cs, ".rivian_charger_type, ''))) IN ('wallbox', 'ac', 'level1', 'level2', 'j1772') THEN 'ac' ",
            "WHEN ", $cs, ".is_home IS TRUE THEN 'ac' ",
            "WHEN lower(COALESCE(", $cs, ".charger_type, '')) IN ('ac', 'ac_l2') THEN ",
            "CASE WHEN COALESCE(", $cs, ".avg_charge_rate_kw, CASE WHEN ", $cs, ".duration_minutes > 0 THEN COALESCE(",
            $cs, ".kwh_added, ", $cs, ".energy_added_wh / 1000.0) / (", $cs, ".duration_minutes::float8 / 60.0) END) >= 20 ",
            "THEN 'dc' ELSE 'ac' END ",
            "WHEN COALESCE(", $cs, ".max_charge_rate_kw, ", $cs, ".avg_charge_rate_kw, ",
            "CASE WHEN ", $cs, ".duration_minutes > 0 THEN COALESCE(", $cs, ".kwh_added, ",
            $cs, ".energy_added_wh / 1000.0) / (", $cs, ".duration_minutes::float8 / 60.0) END) >= 20 THEN 'dc' ",
            "WHEN COALESCE(", $cs, ".max_charge_rate_kw, ", $cs, ".avg_charge_rate_kw, ",
            "CASE WHEN ", $cs, ".duration_minutes > 0 THEN COALESCE(", $cs, ".kwh_added, ",
            $cs, ".energy_added_wh / 1000.0) / (", $cs, ".duration_minutes::float8 / 60.0) END) IS NOT NULL THEN 'ac' ",
            "WHEN lower(COALESCE(", $cs, ".network_vendor, '')) IN ('tesla', 'rivian', 'electrify america', 'evgo') THEN 'dc' ",
            "END)"
        )
    };
}

/// Highest recorded charge power for a session, from the Parallax charging
/// graph and the Rivian live-session points. Vehicles that send no power on
/// the canonical stream (the R2) depend on these points for a peak. Only
/// points inside the session window count, because a live-history poll can
/// link points from an earlier session.
pub async fn recorded_peak_kw(
    pool: &sqlx::PgPool,
    vehicle_id: uuid::Uuid,
    session_id: uuid::Uuid,
    started_at: chrono::DateTime<chrono::Utc>,
    ended_at: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Option<f64>> {
    Ok(sqlx::query_scalar::<_, Option<f64>>(
        r#"SELECT max(power_kw) FROM (
               SELECT source_at AS ts, power_kw FROM timeseries.parallax_charge_curve_points
               WHERE vehicle_id = $1 AND charge_session_id = $2
               UNION ALL
               SELECT ts, power_kw FROM panda.rivian_charge_curve_points
               WHERE vehicle_id = $1 AND charge_session_id = $2
           ) points
           WHERE power_kw > 0 AND power_kw <= $3
             AND ts >= $4 - interval '2 minutes' AND ts <= $5 + interval '2 minutes'"#,
    )
    .bind(vehicle_id)
    .bind(session_id)
    .bind(PLAUSIBLE_MAX_CHARGE_KW)
    .bind(started_at)
    .bind(ended_at)
    .fetch_one(pool)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_energy_1kw_for_1hr() {
        // 1 kW for exactly 1 hour = 1 000 Wh
        let samples = vec![(0.0_f64, 1.0_f64), (3600.0, 1.0)];
        let wh = compute_energy_used_wh(&samples);
        assert!((wh - 1_000.0).abs() < 0.01);
    }

    #[test]
    fn compute_energy_handles_negative_sign() {
        // −11 kW for 30 min = 5 500 Wh (sign convention: negative = grid intake)
        let samples = vec![(0.0_f64, -11.0_f64), (1800.0, -11.0)];
        let wh = compute_energy_used_wh(&samples);
        assert!((wh - 5_500.0).abs() < 1.0);
    }

    #[test]
    fn classify_charger() {
        assert_eq!(classify_charger_type(7.2), "ac");
        assert_eq!(classify_charger_type(19.2), "ac");
        assert_eq!(classify_charger_type(20.0), "dc");
        // The average of a DC fast charge that stopped early is still DC.
        assert_eq!(classify_charger_type(45.3), "dc");
        assert_eq!(classify_charger_type(150.0), "dc");
    }

    #[test]
    fn charger_type_sql_uses_the_shared_threshold_and_alias() {
        let sql = crate::charger_type_sql!("cs");
        assert!(sql.contains(">= 20 THEN 'dc'"));
        assert_eq!(DC_MIN_RATE_KW, 20.0);
        assert!(sql.contains("cs.max_charge_rate_kw"));
        assert!(
            !sql.contains("THEN 'ac_l2'"),
            "the rule never returns ac_l2"
        );
        assert!(sql.starts_with("(CASE") && sql.ends_with("END)"));
        assert!(
            sql.contains("cs.is_home IS TRUE THEN 'ac'"),
            "home sessions stay AC"
        );
        assert!(
            sql.contains("rivian_charger_type"),
            "Rivian's explicit type wins"
        );
    }
}
