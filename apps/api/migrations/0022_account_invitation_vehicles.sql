CREATE TABLE panda.account_invitation_vehicles (
    invitation_id UUID NOT NULL REFERENCES panda.account_invitations(id) ON DELETE CASCADE,
    vehicle_id UUID NOT NULL REFERENCES panda.vehicles(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (invitation_id, vehicle_id)
);

CREATE INDEX account_invitation_vehicles_vehicle_idx
    ON panda.account_invitation_vehicles (vehicle_id);

INSERT INTO panda.account_invitation_vehicles (invitation_id, vehicle_id)
SELECT id, vehicle_id
FROM panda.account_invitations
WHERE vehicle_id IS NOT NULL;
