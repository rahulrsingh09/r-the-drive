-- Additive session lineage: each legacy refresh token starts its own family.
CREATE TABLE panda.session_families (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES panda.users(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz
);
CREATE INDEX session_families_user_idx ON panda.session_families(user_id);

ALTER TABLE panda.refresh_tokens ADD COLUMN family_id uuid DEFAULT gen_random_uuid();
INSERT INTO panda.session_families (id, user_id, created_at, revoked_at)
SELECT family_id, user_id, created_at, revoked_at FROM panda.refresh_tokens;
ALTER TABLE panda.refresh_tokens ALTER COLUMN family_id DROP DEFAULT;
ALTER TABLE panda.refresh_tokens ALTER COLUMN family_id SET NOT NULL;
ALTER TABLE panda.refresh_tokens ADD CONSTRAINT refresh_tokens_family_fk
    FOREIGN KEY (family_id) REFERENCES panda.session_families(id) ON DELETE CASCADE;
ALTER TABLE panda.refresh_tokens ADD COLUMN consumed_at timestamptz;
ALTER TABLE panda.refresh_tokens ADD COLUMN parent_hash bytea;
CREATE UNIQUE INDEX refresh_tokens_parent_idx ON panda.refresh_tokens(parent_hash)
    WHERE parent_hash IS NOT NULL;
CREATE INDEX refresh_tokens_family_idx ON panda.refresh_tokens(family_id);
