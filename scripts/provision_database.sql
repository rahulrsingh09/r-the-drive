-- Invoked only by deploy.provision_database, as the separate bootstrap role.
-- The application password arrives over stdin through a psql variable.
\set ECHO none
SET log_statement = 'none';
SET log_min_error_statement = 'panic';
SET client_min_messages = 'warning';
BEGIN;
SELECT pg_advisory_xact_lock(736281904231);

DO $$
BEGIN
    IF current_user <> 'panda_bootstrap'
       OR NOT (SELECT rolsuper FROM pg_roles WHERE rolname = current_user)
       OR current_database() <> 'panda' THEN
        RAISE EXCEPTION 'unexpected bootstrap connection';
    END IF;
    IF EXISTS (
        SELECT FROM (VALUES ('pgcrypto', 'public'), ('timescaledb', 'public'),
                            ('cube', 'panda'), ('earthdistance', 'panda')) AS required(name, schema)
        JOIN pg_extension e ON e.extname = required.name
        JOIN pg_namespace n ON n.oid = e.extnamespace
        WHERE n.nspname <> required.schema
    ) THEN
        RAISE EXCEPTION 'existing extension schema does not match this installer';
    END IF;
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'panda') THEN
        IF NOT EXISTS (
            SELECT FROM pg_roles
            WHERE rolname = 'panda' AND rolcanlogin AND NOT rolsuper
              AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolreplication
              AND NOT rolbypassrls AND NOT rolinherit
              AND shobj_description(oid, 'pg_authid') =
                  'R:The Drive installer application role v1'
        ) OR EXISTS (
            SELECT FROM pg_auth_members
            WHERE member = (SELECT oid FROM pg_roles WHERE rolname = 'panda')
        ) OR NOT EXISTS (
            SELECT FROM pg_database
            WHERE datname = 'panda' AND datdba = (SELECT oid FROM pg_roles WHERE rolname = 'panda')
        ) OR EXISTS (
            SELECT FROM (VALUES ('panda'), ('timeseries')) AS required(name)
            LEFT JOIN pg_namespace n ON n.nspname = required.name
            WHERE n.oid IS NULL OR n.nspowner <> (SELECT oid FROM pg_roles WHERE rolname = 'panda')
        ) OR EXISTS (
            SELECT FROM (VALUES ('pgcrypto', 'public'), ('timescaledb', 'public'),
                                ('cube', 'panda'), ('earthdistance', 'panda')) AS required(name, schema)
            LEFT JOIN pg_extension e ON e.extname = required.name
            LEFT JOIN pg_namespace n ON n.oid = e.extnamespace
            WHERE e.oid IS NULL OR n.nspname <> required.schema
        ) THEN
            RAISE EXCEPTION 'existing application role or extension state does not match this installer';
        END IF;
    ELSE
        -- Refuse adoption of a database with untracked application objects.
        -- Extension-owned objects are expected from the Timescale image.
        IF NOT EXISTS (
            SELECT FROM pg_database
            WHERE datname = 'panda' AND datdba = (SELECT oid FROM pg_roles WHERE rolname = current_user)
        ) OR EXISTS (
            SELECT FROM pg_namespace WHERE nspname IN ('panda', 'timeseries')
        ) OR EXISTS (
            SELECT FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE n.nspname NOT IN ('pg_catalog', 'information_schema')
              AND n.nspname NOT LIKE 'pg_toast%'
              AND c.relkind IN ('r', 'p', 'v', 'm', 'S', 'f')
              AND NOT EXISTS (
                  SELECT FROM pg_depend d WHERE d.classid = 'pg_class'::regclass
                    AND d.objid = c.oid AND d.deptype = 'e'
              )
        ) THEN
            RAISE EXCEPTION 'existing database is not an empty managed installation';
        END IF;
    END IF;
END
$$;

SELECT NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'panda') AS create_app \gset
\if :create_app
CREATE ROLE panda LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE
    NOINHERIT NOREPLICATION NOBYPASSRLS PASSWORD :'app_password';
COMMENT ON ROLE panda IS 'R:The Drive installer application role v1';
ALTER DATABASE panda OWNER TO panda;
CREATE SCHEMA panda AUTHORIZATION panda;
CREATE SCHEMA timeseries AUTHORIZATION panda;
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
REVOKE ALL ON SCHEMA panda, timeseries FROM PUBLIC;
CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;
CREATE EXTENSION IF NOT EXISTS timescaledb WITH SCHEMA public;
CREATE EXTENSION IF NOT EXISTS cube WITH SCHEMA panda;
CREATE EXTENSION IF NOT EXISTS earthdistance WITH SCHEMA panda;
\endif
COMMIT;
