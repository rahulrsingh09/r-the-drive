REVOKE DELETE, TRUNCATE ON
    panda.vehicles,
    panda.trips,
    timeseries.telemetry,
    panda.vehicle_state_periods,
    panda.software_versions,
    panda.battery_capacity_snapshots
FROM CURRENT_USER;

-- Charging repair merges duplicate sessions into a durable canonical session.
REVOKE TRUNCATE ON panda.charge_sessions FROM CURRENT_USER;

SELECT alter_job(job_id, scheduled => false)
FROM timescaledb_information.jobs
WHERE proc_name = 'policy_retention'
  AND scheduled
  AND (
      (hypertable_schema = 'timeseries' AND hypertable_name = 'telemetry')
      OR (hypertable_schema = 'panda' AND hypertable_name IN (
          'trips', 'charge_sessions', 'rivian_charge_payloads'
      ))
  );
