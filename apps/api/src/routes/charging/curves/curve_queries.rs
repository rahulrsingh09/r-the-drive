use super::CurveSource;

pub(super) const LINKED_COUNT: &str = r#"SELECT COUNT(*)::int8
           FROM timeseries.telemetry
           WHERE vehicle_id=$1
             AND charge_session_id=$2
             AND ts >= $3
             AND ts <= $4"#;

pub(super) const MINUTE_COUNT: &str = r#"SELECT COUNT(*)::int8
           FROM timeseries.telemetry_1min
           WHERE vehicle_id=$1
             AND bucket >= $2
             AND bucket <= $3
             AND avg_soc IS NOT NULL"#;

pub(super) const DETAIL_FALLBACK: &str = r#"WITH candidates AS (
                   SELECT ts,power_kw,NULL::float8 AS soc,'rivian_charge_curve_points'::text AS sample_source,1 AS precedence
                   FROM panda.rivian_charge_curve_points
                   WHERE vehicle_id=$1 AND charge_session_id=$3 AND power_kw IS NOT NULL AND power_kw > 0
                   UNION ALL
                   SELECT source_at AS ts,power_kw,soc,'parallax_charge_curve_points'::text AS sample_source,2 AS precedence
                   FROM timeseries.parallax_charge_curve_points
                   WHERE vehicle_id=$1 AND charge_session_id=$3 AND power_kw IS NOT NULL AND power_kw > 0
               ), resolved AS (
                   SELECT DISTINCT ON (ts) ts,power_kw,soc,sample_source
                   FROM candidates ORDER BY ts,precedence DESC
               )
               SELECT EXTRACT(EPOCH FROM (ts - $2))::float8 / 60.0 AS minutes_elapsed,
                      LEAST(CASE WHEN $4 THEN 300.0 ELSE 22.0 END,
                            GREATEST(0.0, power_kw)) AS charge_rate_kw,
                      soc,
                      'recorded'::text AS power_method,
                      sample_source
               FROM resolved
               ORDER BY ts"#;

pub(super) const ANALYSIS_LINKED: &str = r#"WITH cap AS (
                   SELECT COALESCE(
                       (SELECT battery_capacity_wh
                        FROM panda.vehicles
                        WHERE id=$1
                          AND battery_capacity_wh IS NOT NULL
                        LIMIT 1),
                       (SELECT max(battery_capacity_wh)
                        FROM timeseries.telemetry
                        WHERE vehicle_id=$1
                          AND battery_capacity_wh IS NOT NULL)
                   ) AS default_cap_wh
               ),
                samples AS (
                    SELECT t.ts AS bucket,
                           t.battery_level AS soc,
                           ABS(t.power_kw) AS power_kw,
                           t.battery_capacity_wh AS capacity_wh
                    FROM timeseries.telemetry t
                   WHERE t.vehicle_id=$1
                     AND t.charge_session_id=$2
                     AND t.ts >= $3
                     AND t.ts <= $4
                ),
                intervals AS (
                    SELECT bucket,
                           soc,
                           power_kw,
                           capacity_wh,
                           LAG(bucket) OVER (ORDER BY bucket) AS previous_bucket,
                           LAG(soc) OVER (ORDER BY bucket) AS previous_soc
                    FROM samples
                    WHERE soc IS NOT NULL
                ),
                calculated AS (
                    SELECT bucket,
                           soc,
                           power_kw,
                           CASE
                               WHEN power_kw IS NULL
                                AND soc > previous_soc
                                AND EXTRACT(EPOCH FROM (bucket - previous_bucket)) BETWEEN 10 AND 300
                               THEN 3600.0 * (soc - previous_soc) / 100.0
                                   * COALESCE(capacity_wh, cap.default_cap_wh) / 1000.0
                                   / EXTRACT(EPOCH FROM (bucket - previous_bucket))
                           END AS derived_power_kw
                    FROM intervals
                    CROSS JOIN cap
                ),
                session_soc_points AS (
                    SELECT MIN(bucket) AS ts,
                           EXTRACT(EPOCH FROM (MIN(bucket) - $3))::float8 / 60.0 AS minutes_elapsed,
                           soc,
                           CASE
                               WHEN COUNT(*) FILTER (WHERE power_kw IS NOT NULL) > 0
                               THEN AVG(power_kw) FILTER (WHERE power_kw IS NOT NULL)
                               ELSE AVG(derived_power_kw)
                           END AS charge_rate_kw,
                           CASE
                               WHEN COUNT(*) FILTER (WHERE power_kw IS NOT NULL) > 0 THEN 'recorded'
                               WHEN COUNT(*) FILTER (WHERE derived_power_kw IS NOT NULL) > 0 THEN 'soc_delta'
                           END AS power_method
                    FROM calculated
                    WHERE power_kw IS NOT NULL OR derived_power_kw IS NOT NULL
                    GROUP BY soc
                )
               SELECT ts,
                      minutes_elapsed,
                      LEAST(
                          CASE WHEN $5 THEN 300.0 ELSE 22.0 END,
                          GREATEST(
                              0.0,
                              charge_rate_kw
                          )
                      ) AS charge_rate_kw,
                      soc,
                      power_method
               FROM session_soc_points
               ORDER BY ts"#;

pub(super) const ANALYSIS_FALLBACK: &str = r#"WITH candidates AS (
               SELECT ts,power_kw,'rivian_charge_curve_points'::text AS sample_source,1 AS precedence
               FROM panda.rivian_charge_curve_points
               WHERE vehicle_id=$1 AND charge_session_id=$2 AND power_kw IS NOT NULL AND power_kw > 0
               UNION ALL
               SELECT source_at AS ts,power_kw,'parallax_charge_curve_points'::text AS sample_source,2 AS precedence
               FROM timeseries.parallax_charge_curve_points
               WHERE vehicle_id=$1 AND charge_session_id=$2 AND power_kw IS NOT NULL AND power_kw > 0
           )
           SELECT DISTINCT ON (ts) ts,power_kw,sample_source
           FROM candidates ORDER BY ts,precedence DESC"#;

pub(super) fn bucket_query(source: CurveSource, analysis: bool) -> String {
    let (samples, start, dc, power, capacity) = match source {
        CurveSource::Linked => (
            "SELECT time_bucket('1 minute',t.ts) AS bucket,
            AVG(t.battery_level) AS avg_soc,
            AVG(ABS(t.power_kw)) FILTER (WHERE t.power_kw IS NOT NULL) AS avg_power_kw,
            AVG(t.battery_capacity_wh) AS bucket_cap_wh
            FROM timeseries.telemetry t WHERE t.vehicle_id=$1 AND t.charge_session_id=$2
              AND t.ts>=$3 AND t.ts<=$4 GROUP BY 1",
            "$3",
            "$5",
            "avg_power_kw",
            "bucket_cap_wh",
        ),
        CurveSource::Minute => (
            "SELECT bucket,avg_soc,avg_power_kw,battery_capacity_wh
            FROM timeseries.telemetry_1min WHERE vehicle_id=$1 AND bucket>=$2 AND bucket<=$3",
            "$2",
            "$4",
            "ABS(avg_power_kw)",
            "battery_capacity_wh",
        ),
    };
    let timestamp = if analysis { "bucket AS ts," } else { "" };
    format!("WITH cap AS (SELECT COALESCE(
            (SELECT battery_capacity_wh FROM panda.vehicles WHERE id=$1 AND battery_capacity_wh IS NOT NULL LIMIT 1),
            (SELECT max(battery_capacity_wh) FROM timeseries.telemetry WHERE vehicle_id=$1 AND battery_capacity_wh IS NOT NULL)
        ) AS default_cap_wh), samples AS ({samples}),
        intervals AS (SELECT *, LAG(avg_soc) OVER (ORDER BY bucket) AS previous_soc,
            EXTRACT(EPOCH FROM (bucket-LAG(bucket) OVER (ORDER BY bucket))) AS elapsed
            FROM samples WHERE avg_soc IS NOT NULL)
        SELECT {timestamp} EXTRACT(EPOCH FROM (bucket-{start}))::float8/60.0 AS minutes_elapsed,
            LEAST(CASE WHEN {dc} THEN 300.0 ELSE 22.0 END,
                GREATEST(0.0,COALESCE({power},CASE WHEN avg_soc>previous_soc AND elapsed BETWEEN 30 AND 300
                    THEN 3600.0*(avg_soc-previous_soc)/100.0*COALESCE({capacity},cap.default_cap_wh)/1000.0/elapsed END))) AS charge_rate_kw,
            avg_soc AS soc,
            CASE WHEN avg_power_kw IS NOT NULL THEN 'recorded'
                 WHEN avg_soc>previous_soc AND elapsed BETWEEN 30 AND 300 THEN 'soc_delta' END AS power_method
        FROM intervals CROSS JOIN cap ORDER BY bucket")
}
