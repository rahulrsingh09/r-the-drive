ALTER TABLE panda.authentication_settings
    ADD COLUMN oidc_auto_login boolean NOT NULL DEFAULT FALSE;
