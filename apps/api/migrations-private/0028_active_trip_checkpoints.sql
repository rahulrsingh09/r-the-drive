-- Checkpoint advancement and completion enqueueing share a transaction.
CREATE TABLE panda.active_trip_checkpoints (
    vehicle_id uuid PRIMARY KEY REFERENCES panda.vehicles(id) ON DELETE CASCADE,
    snapshot jsonb NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE panda.pending_trip_completions (
    trip_id uuid PRIMARY KEY,
    vehicle_id uuid NOT NULL REFERENCES panda.vehicles(id) ON DELETE CASCADE,
    trip jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX pending_trip_completions_vehicle_idx
    ON panda.pending_trip_completions(vehicle_id, created_at);

ALTER TABLE panda.vehicle_runtime_state
    ADD COLUMN collector_heartbeat_at timestamptz,
    ADD COLUMN trip_persistence_error text;
