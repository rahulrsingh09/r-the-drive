UPDATE panda.external_connection_settings
SET enabled = TRUE,
    mode = 'remote',
    basemap_provider = CASE
        WHEN basemap_provider = 'carto' THEN basemap_provider
        ELSE 'openfreemap'
    END,
    updated_at = now()
WHERE id = 'basemap'
  AND enabled = FALSE
  AND mode = 'disabled';
