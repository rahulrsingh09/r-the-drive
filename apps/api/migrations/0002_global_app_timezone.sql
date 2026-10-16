INSERT INTO panda.system_config (key, value)
SELECT 'app_timezone', COALESCE(
    (SELECT timezone FROM panda.backup_settings WHERE id = TRUE),
    'UTC'
)
WHERE NOT EXISTS (
    SELECT 1 FROM panda.system_config WHERE key = 'app_timezone'
);
