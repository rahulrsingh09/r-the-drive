pub(super) fn reconstruction_query(mutation: &'static str, paired_location: bool) -> String {
    let (latitude_filter, longitude_filter) = match paired_location {
        false => ("latitude IS NOT NULL", "longitude IS NOT NULL"),
        true => (
            "latitude IS NOT NULL AND longitude IS NOT NULL AND NOT (latitude = 0 AND longitude = 0)",
            "latitude IS NOT NULL AND longitude IS NOT NULL AND NOT (latitude = 0 AND longitude = 0)",
        ),
    };
    format!(
        r#"WITH evidence AS (
            SELECT ts, vehicle_id, latitude, longitude, battery_level, battery_limit,
                   battery_capacity_wh, power_kw, LAG(ts) OVER (ORDER BY ts) AS prev_ts
            FROM timeseries.telemetry
            WHERE vehicle_id = $1
              AND (charger_state = 'charging'
                OR charger_status = 'chrgr_sts_connected_charging'
                OR COALESCE(time_to_end_of_charge_min, 0) > 0)
        ), segmented AS (
            SELECT *, COUNT(*) FILTER (
                WHERE prev_ts IS NULL OR ts - prev_ts > interval '2 hours'
            ) OVER (ORDER BY ts) AS segment_id
            FROM evidence
        ), summaries AS (
            SELECT vehicle_id, segment_id, MIN(ts) AS started_at, MAX(ts) AS ended_at,
                   MIN(ts) FILTER (WHERE {latitude_filter}) AS latitude_ts,
                   MIN(ts) FILTER (WHERE {longitude_filter}) AS longitude_ts,
                   MIN(ts) FILTER (WHERE battery_level IS NOT NULL) AS soc_start_ts,
                   MAX(ts) FILTER (WHERE battery_level IS NOT NULL) AS soc_end_ts,
                   MAX(battery_limit) AS charge_limit,
                   AVG(battery_capacity_wh) FILTER (WHERE battery_capacity_wh IS NOT NULL) AS battery_capacity_wh,
                   MAX(ABS(power_kw)) FILTER (WHERE power_kw IS NOT NULL) AS max_charge_rate_kw,
                   EXTRACT(EPOCH FROM (MAX(ts) - MIN(ts)))::int / 60 AS duration_minutes
            FROM segmented
            GROUP BY vehicle_id, segment_id
        ), windows AS (
            SELECT gen_random_uuid() AS id, summary.vehicle_id,
                   summary.started_at, summary.ended_at,
                   latitude.latitude AS location_lat, longitude.longitude AS location_lng,
                   first_soc.battery_level AS soc_start, last_soc.battery_level AS soc_end,
                   summary.charge_limit, summary.battery_capacity_wh,
                   summary.max_charge_rate_kw, summary.duration_minutes
            FROM summaries summary
            LEFT JOIN evidence latitude ON latitude.ts = summary.latitude_ts
            LEFT JOIN evidence longitude ON longitude.ts = summary.longitude_ts
            LEFT JOIN evidence first_soc ON first_soc.ts = summary.soc_start_ts
            LEFT JOIN evidence last_soc ON last_soc.ts = summary.soc_end_ts
        ), candidates AS (
            SELECT *, CASE WHEN soc_end > soc_start AND battery_capacity_wh > 0
                THEN (soc_end - soc_start) / 100.0 * battery_capacity_wh END AS energy_added_wh
            FROM windows
            WHERE ended_at > started_at AND duration_minutes >= 5
              AND soc_end IS NOT NULL AND soc_start IS NOT NULL AND soc_end - soc_start >= 1.0
        ), {mutation}"#
    )
}
