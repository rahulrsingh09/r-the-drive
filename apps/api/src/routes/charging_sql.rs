pub(super) const LIST: &str = r#"
                cs.live_current_price, cs.live_current_currency, cs.live_total_charged_kwh,
                cs.live_range_added_km, COALESCE(latest.power_kw, cs.live_power_kw) AS live_power_kw, cs.live_charge_rate_kph,
                CASE WHEN cs.ended_at IS NULL THEN GREATEST(COALESCE(cs.live_time_elapsed_seconds, 0), EXTRACT(EPOCH FROM (now() - COALESCE(cs.live_session_started_at, cs.started_at)))::int) ELSE cs.live_time_elapsed_seconds END AS live_time_elapsed_seconds,
                CASE WHEN cs.ended_at IS NULL THEN COALESCE(latest.time_to_end_of_charge_min, cs.live_time_remaining_minutes) ELSE cs.live_time_remaining_minutes END AS live_time_remaining_min, cs.live_session_started_at AS live_started_at,
                latest.battery_level AS live_soc_pct, latest.charger_state AS live_charger_state, latest.charger_status AS live_charger_status,
                (SELECT COUNT(*)::int8 FROM timeseries.telemetry t WHERE t.vehicle_id=cs.vehicle_id AND t.charge_session_id=cs.id AND t.ts >= cs.started_at AND t.ts <= COALESCE(cs.ended_at, now())) AS telemetry_sample_count"#;

pub(super) const COST: &str =
    "CASE WHEN cs.ended_at IS NULL THEN NULL ELSE cs.cost_usd END AS cost_usd";

pub(super) const DETAIL: &str = r#"
                cs.live_current_price, cs.live_current_currency, cs.live_total_charged_kwh,
                cs.live_range_added_km, COALESCE(latest.power_kw, cs.live_power_kw) AS live_power_kw, cs.live_charge_rate_kph,
                CASE WHEN cs.ended_at IS NULL THEN GREATEST(COALESCE(cs.live_time_elapsed_seconds, 0), EXTRACT(EPOCH FROM (now() - COALESCE(cs.live_session_started_at, cs.started_at)))::int) ELSE cs.live_time_elapsed_seconds END AS live_time_elapsed_seconds,
                CASE WHEN cs.ended_at IS NULL THEN COALESCE(latest.time_to_end_of_charge_min, cs.live_time_remaining_minutes) ELSE cs.live_time_remaining_minutes END AS live_time_remaining_min, cs.live_session_started_at AS live_started_at,
                latest.battery_level AS live_soc_pct, latest.charger_state AS live_charger_state, latest.charger_status AS live_charger_status,
                COALESCE(telem.sample_count, 0)::int8 AS telemetry_sample_count"#;

pub(super) const LATEST: &str = r#"
         LEFT JOIN LATERAL (SELECT t.battery_level, t.power_kw, t.charger_state, t.charger_status, t.time_to_end_of_charge_min FROM timeseries.telemetry t WHERE t.vehicle_id=cs.vehicle_id AND t.charge_session_id=cs.id AND t.ts >= cs.started_at AND t.ts <= COALESCE(cs.ended_at, now()) ORDER BY t.ts DESC LIMIT 1) latest ON true"#;

pub(super) const TELEM: &str = r#"
         LEFT JOIN LATERAL (
             SELECT t.battery_level, t.power_kw, t.charger_state, t.charger_status, t.time_to_end_of_charge_min
             FROM timeseries.telemetry t
             WHERE t.vehicle_id = cs.vehicle_id
               AND t.charge_session_id = cs.id
               AND t.ts BETWEEN cs.started_at AND COALESCE(cs.ended_at, now())
             ORDER BY t.ts DESC LIMIT 1
         ) latest ON true"#;

#[derive(Clone, Copy)]
pub(super) enum SessionProjection {
    History,
    Detail,
}

pub(super) fn session_query(projection: SessionProjection) -> String {
    let (live, joins, predicate) = match projection {
        SessionProjection::History => (LIST, LATEST.to_owned(),
            "cs.vehicle_id=$1 AND cs.started_at>=$2 AND cs.started_at<=$3
             AND ($6 IS NULL OR TO_CHAR((cs.started_at AT TIME ZONE COALESCE((SELECT value FROM panda.system_config WHERE key='app_timezone'),'UTC'))::date,'YYYY-MM-DD')=$6)
             ORDER BY cs.started_at DESC, cs.id DESC LIMIT $4 OFFSET $5"),
        SessionProjection::Detail => (DETAIL, format!("LEFT JOIN LATERAL (
            SELECT COUNT(*)::int8 AS sample_count FROM timeseries.telemetry t
            WHERE t.vehicle_id=cs.vehicle_id AND t.ts BETWEEN cs.started_at AND COALESCE(cs.ended_at,now())
            ) telem ON true {TELEM}"), "cs.id=$1 AND cs.vehicle_id=$2"),
    };
    format!("SELECT cs.id,cs.started_at,
        TO_CHAR((cs.started_at AT TIME ZONE COALESCE((SELECT value FROM panda.system_config WHERE key='app_timezone'),'UTC'))::date,'YYYY-MM-DD') AS session_day_local,
        cs.ended_at,cs.location_lat,cs.location_lng,cs.source_location_lat,cs.source_location_lng,
        cs.location_override_mode,cs.cost_override_mode,cs.cost_override_usd,
        COALESCE(g.name,a.display_name,CASE WHEN cs.is_home THEN 'Home' END) AS location_name,
        cs.is_home,COALESCE(cs.charger_type, CASE
            WHEN lower(COALESCE(cs.network_vendor,''))=ANY(ARRAY['tesla','rivian','electrify america','evgo']) THEN 'dc'
            WHEN rate.kw<12 THEN 'ac' WHEN rate.kw<50 THEN 'ac_l2' WHEN rate.kw IS NOT NULL THEN 'dc'
        END) AS charger_type,
        energy.kwh AS kwh_added,cs.soc_start,cs.soc_end,rate.kw AS max_charge_rate_kw,cs.duration_minutes,
        {COST},cs.network_vendor,cs.range_added_km,cs.is_free_session,cs.is_rivian_network,cs.rivian_paid_total,cs.source,
        cs.rivian_charger_type,cs.currency_code,cs.rivian_city,cs.is_public,cs.charger_id,{live}
        FROM panda.charge_sessions cs
        CROSS JOIN LATERAL (SELECT COALESCE(cs.kwh_added,cs.energy_added_wh/1000.0) AS kwh) energy
        CROSS JOIN LATERAL (SELECT COALESCE(cs.max_charge_rate_kw,cs.avg_charge_rate_kw,
            CASE WHEN cs.duration_minutes>0 THEN energy.kwh/(cs.duration_minutes::float8/60.0) END) AS kw) rate
        LEFT JOIN panda.geofences g ON g.id=cs.geofence_id
        LEFT JOIN panda.addresses a ON a.id=cs.address_id
        {joins} WHERE {predicate}")
}
