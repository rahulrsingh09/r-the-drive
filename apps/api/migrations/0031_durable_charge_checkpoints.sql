CREATE TABLE panda.active_charge_checkpoints (
    vehicle_id uuid PRIMARY KEY REFERENCES panda.vehicles(id) ON DELETE CASCADE,
    active_session_id uuid REFERENCES panda.charge_sessions(id) ON DELETE RESTRICT,
    snapshot jsonb NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX active_charge_checkpoints_session_idx
    ON panda.active_charge_checkpoints(active_session_id)
    WHERE active_session_id IS NOT NULL;

CREATE TABLE panda.pending_charge_completions (
    session_id uuid PRIMARY KEY REFERENCES panda.charge_sessions(id) ON DELETE RESTRICT,
    vehicle_id uuid NOT NULL REFERENCES panda.vehicles(id) ON DELETE CASCADE,
    session jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX pending_charge_completions_vehicle_idx
    ON panda.pending_charge_completions(vehicle_id, created_at);
