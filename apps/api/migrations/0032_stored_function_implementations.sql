CREATE OR REPLACE FUNCTION panda.semantic_charge_payload(input jsonb)
RETURNS jsonb
LANGUAGE sql
IMMUTABLE
STRICT
AS $$
    SELECT CASE jsonb_typeof(input)
        WHEN 'object' THEN COALESCE((
            SELECT jsonb_object_agg(entry.key, panda.semantic_charge_payload(entry.value))
            FROM jsonb_each(input) AS entry
            WHERE entry.key <> ALL (ARRAY[
                'updatedAt', 'retrievedAt', 'capturedAt',
                'observedAt', 'fetchedAt', 'lastUpdatedAt'
            ])
        ), '{}'::jsonb)
        WHEN 'array' THEN COALESCE((
            SELECT jsonb_agg(panda.semantic_charge_payload(entry.value) ORDER BY entry.position)
            FROM jsonb_array_elements(input) WITH ORDINALITY AS entry(value, position)
        ), '[]'::jsonb)
        ELSE input
    END
$$;

CREATE OR REPLACE FUNCTION panda.project_legacy_theme_write()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    palette_changed boolean := NEW.theme_palette IS DISTINCT FROM OLD.theme_palette;
    mode_changed boolean := NEW.theme_mode IS DISTINCT FROM OLD.theme_mode;
    version_unchanged boolean := NEW.theme_etag_version IS NOT DISTINCT FROM OLD.theme_etag_version;
BEGIN
    IF ROW(NEW.theme_selection_kind, NEW.theme_builtin_id, NEW.theme_custom_id, NEW.theme_custom_revision)
       IS DISTINCT FROM
       ROW(OLD.theme_selection_kind, OLD.theme_builtin_id, OLD.theme_custom_id, OLD.theme_custom_revision) THEN
        RETURN NEW;
    END IF;

    IF palette_changed THEN
        NEW.theme_selection_kind := 'builtin';
        NEW.theme_builtin_id := NEW.theme_palette;
        NEW.theme_custom_id := NULL;
        NEW.theme_custom_revision := NULL;
    END IF;
    IF palette_changed OR (mode_changed AND version_unchanged) THEN
        NEW.theme_etag_version := OLD.theme_etag_version + 1;
    END IF;
    RETURN NEW;
END;
$$;
