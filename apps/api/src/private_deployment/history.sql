DO $history_policy$
DECLARE
    rule record;
    retention_job integer;
BEGIN
    FOR rule IN
        SELECT * FROM (VALUES
            ('panda.vehicles'::regclass, 'DELETE, TRUNCATE'),
            ('panda.trips'::regclass, 'DELETE, TRUNCATE'),
            ('timeseries.telemetry'::regclass, 'DELETE, TRUNCATE'),
            ('panda.vehicle_state_periods'::regclass, 'DELETE, TRUNCATE'),
            ('panda.software_versions'::regclass, 'DELETE, TRUNCATE'),
            ('panda.battery_capacity_snapshots'::regclass, 'DELETE, TRUNCATE'),
            -- Charging repair still merges duplicate sessions into a canonical session.
            ('panda.charge_sessions'::regclass, 'TRUNCATE')
        ) AS policy(relation, privileges)
    LOOP
        EXECUTE format('REVOKE %s ON TABLE %s FROM CURRENT_USER', rule.privileges, rule.relation);
    END LOOP;

    FOR retention_job IN
        SELECT job_id FROM timescaledb_information.jobs
        WHERE scheduled AND proc_name = 'policy_retention'
          AND (hypertable_schema, hypertable_name) IN (
              ('timeseries', 'telemetry'), ('panda', 'trips'),
              ('panda', 'charge_sessions'), ('panda', 'rivian_charge_payloads')
          )
    LOOP
        PERFORM alter_job(retention_job, scheduled => false);
    END LOOP;
END
$history_policy$;
