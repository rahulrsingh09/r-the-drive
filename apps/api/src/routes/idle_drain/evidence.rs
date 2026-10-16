pub(super) const GAP_EVIDENCE: &str = r#"WITH candidate_periods AS (
    SELECT * FROM unnest($5::timestamptz[], $6::timestamptz[]) AS gap(period_start, period_end)
), filtered_candidates AS (
    SELECT period_start, period_end FROM candidate_periods
    WHERE period_end > period_start AND period_start >= $2 AND period_start <= $3
      AND (EXTRACT(EPOCH FROM (period_end - period_start)) / 3600.0) >= $4
)
        SELECT
            c.period_start,
            c.period_end,
            (EXTRACT(EPOCH FROM (c.period_end - c.period_start)) / 3600.0)::float8 AS duration_hours,
            start_soc_sample.soc AS soc_start,
            end_soc_sample.soc AS soc_end,
            start_soc_sample.sample_at AS start_sample_at,
            end_soc_sample.sample_at AS end_sample_at,
            start_soc_sample.gap_minutes AS start_sample_gap_minutes,
            end_soc_sample.gap_minutes AS end_sample_gap_minutes,
            start_range_sample.range_mi AS range_start_mi,
            end_range_sample.range_mi AS range_end_mi,
            start_range_sample.gap_minutes AS start_range_gap_minutes,
            end_range_sample.gap_minutes AS end_range_gap_minutes,
            cap_sample.capacity_wh AS capacity_wh,
            COALESCE(telemetry_window.sample_count, 0) AS sample_count,
            COALESCE(telemetry_window.battery_sample_count, 0) AS battery_sample_count,
            COALESCE(telemetry_window.interior_battery_sample_count, 0) AS interior_battery_sample_count,
            telemetry_window.min_soc_in_window,
            telemetry_window.max_soc_in_window,
            telemetry_window.odometer_delta_mi,
            COALESCE(telemetry_window.drive_sample_count, 0) AS drive_sample_count,
            COALESCE(trip_overlap.trip_overlap_count, 0) AS trip_overlap_count,
            COALESCE(charge_overlap.charge_overlap_count, 0) AS charge_overlap_count,
            state_overlap.sleep_seconds,
            state_overlap.covered_state_seconds,
            telemetry_window.has_reduced_range
        FROM filtered_candidates c
        LEFT JOIN LATERAL (
            SELECT
                t.ts AS sample_at,
                t.battery_level::float8 AS soc,
                ABS(EXTRACT(EPOCH FROM (t.ts - c.period_start)))::float8 / 60.0::float8 AS gap_minutes
            FROM timeseries.telemetry t
            WHERE t.vehicle_id = $1
            AND t.ts >= c.period_start - INTERVAL '72 hours'
              AND t.ts <= c.period_start + INTERVAL '72 hours'
              AND t.battery_level IS NOT NULL
            ORDER BY ABS(EXTRACT(EPOCH FROM (t.ts - c.period_start))) ASC
            LIMIT 1
        ) start_soc_sample ON TRUE
        LEFT JOIN LATERAL (
            SELECT
                t.ts AS sample_at,
                t.battery_level::float8 AS soc,
                ABS(EXTRACT(EPOCH FROM (t.ts - c.period_end)))::float8 / 60.0::float8 AS gap_minutes
            FROM timeseries.telemetry t
            WHERE t.vehicle_id = $1
            AND t.ts >= c.period_end - INTERVAL '72 hours'
              AND t.ts <= c.period_end + INTERVAL '72 hours'
              AND t.battery_level IS NOT NULL
            ORDER BY ABS(EXTRACT(EPOCH FROM (t.ts - c.period_end))) ASC
            LIMIT 1
        ) end_soc_sample ON TRUE
        LEFT JOIN LATERAL (
            SELECT
                t.distance_to_empty_mi::float8 AS range_mi,
                ABS(EXTRACT(EPOCH FROM (t.ts - c.period_start)))::float8 / 60.0::float8 AS gap_minutes
            FROM timeseries.telemetry t
            WHERE t.vehicle_id = $1
            AND t.ts >= c.period_start - INTERVAL '72 hours'
              AND t.ts <= c.period_start + INTERVAL '72 hours'
              AND t.distance_to_empty_mi IS NOT NULL
            ORDER BY ABS(EXTRACT(EPOCH FROM (t.ts - c.period_start))) ASC
            LIMIT 1
        ) start_range_sample ON TRUE
        LEFT JOIN LATERAL (
            SELECT
                t.distance_to_empty_mi::float8 AS range_mi,
                ABS(EXTRACT(EPOCH FROM (t.ts - c.period_end)))::float8 / 60.0::float8 AS gap_minutes
            FROM timeseries.telemetry t
            WHERE t.vehicle_id = $1
            AND t.ts >= c.period_end - INTERVAL '72 hours'
              AND t.ts <= c.period_end + INTERVAL '72 hours'
              AND t.distance_to_empty_mi IS NOT NULL
            ORDER BY ABS(EXTRACT(EPOCH FROM (t.ts - c.period_end))) ASC
            LIMIT 1
        ) end_range_sample ON TRUE
        LEFT JOIN LATERAL (
            SELECT t.battery_capacity_wh::float8 AS capacity_wh
            FROM timeseries.telemetry t
            WHERE t.vehicle_id = $1
              AND t.ts <= c.period_end
              AND t.battery_capacity_wh IS NOT NULL
              AND t.battery_capacity_wh > 10000
            ORDER BY t.ts DESC
            LIMIT 1
        ) cap_sample ON TRUE
        LEFT JOIN LATERAL (
            SELECT
                COUNT(*)::bigint AS sample_count,
                COUNT(*) FILTER (WHERE t.battery_level IS NOT NULL)::bigint AS battery_sample_count,
                COUNT(*) FILTER (
                    WHERE t.battery_level IS NOT NULL
                      AND t.ts > c.period_start
                      AND t.ts < c.period_end
                )::bigint AS interior_battery_sample_count,
                MIN(t.battery_level::float8) FILTER (WHERE t.battery_level IS NOT NULL) AS min_soc_in_window,
                MAX(t.battery_level::float8) FILTER (WHERE t.battery_level IS NOT NULL) AS max_soc_in_window,
                CASE
                    WHEN COUNT(*) FILTER (WHERE t.odometer_miles IS NOT NULL) = 0 THEN NULL
                    ELSE MAX(t.odometer_miles::float8) - MIN(t.odometer_miles::float8)
                END AS odometer_delta_mi,
                COUNT(*) FILTER (
                    WHERE LOWER(COALESCE(t.power_state, '')) IN ('drive', 'go')
                )::bigint AS drive_sample_count,
                CASE
                    WHEN COUNT(*) = 0 THEN NULL
                    ELSE BOOL_OR(
                        (t.outside_temp_c IS NOT NULL AND t.outside_temp_c < -5)
                        OR COALESCE(t.hvac_active, FALSE)
                    )
                END AS has_reduced_range
            FROM timeseries.telemetry t
            WHERE t.vehicle_id = $1
              AND t.ts >= c.period_start
              AND t.ts <= c.period_end
        ) telemetry_window ON TRUE
        LEFT JOIN LATERAL (
            SELECT COUNT(*)::bigint AS trip_overlap_count
            FROM panda.trips trip
            WHERE trip.vehicle_id = $1
              AND trip.started_at < c.period_end
              AND COALESCE(trip.ended_at, c.period_end) > c.period_start
        ) trip_overlap ON TRUE
        LEFT JOIN LATERAL (
            SELECT COUNT(*)::bigint AS charge_overlap_count
            FROM panda.charge_sessions charge
            WHERE charge.vehicle_id = $1
              AND charge.started_at < c.period_end
              AND COALESCE(charge.ended_at, c.period_end) > c.period_start
        ) charge_overlap ON TRUE
        LEFT JOIN LATERAL (
            SELECT
                (SUM(
                    EXTRACT(
                        EPOCH FROM (
                            LEAST(COALESCE(v.ended_at, c.period_end), c.period_end)
                            - GREATEST(v.started_at, c.period_start)
                        )
                    )
                ) FILTER (
                    WHERE v.state = 'sleep'
                ))::float8 AS sleep_seconds,
                (SUM(
                    EXTRACT(
                        EPOCH FROM (
                            LEAST(COALESCE(v.ended_at, c.period_end), c.period_end)
                            - GREATEST(v.started_at, c.period_start)
                        )
                    )
                ))::float8 AS covered_state_seconds
            FROM panda.vehicle_state_periods v
            WHERE v.vehicle_id = $1
              AND v.started_at < c.period_end
              AND COALESCE(v.ended_at, c.period_end) > c.period_start
        ) state_overlap ON TRUE
        ORDER BY c.period_start DESC
"#;
