#!/bin/sh
set -eu
umask 077

phase=$1
sha=$(printf '%s' "$2" | base64 -d)
run_id=$(printf '%s' "$3" | base64 -d)
case "$sha" in ''|*[!0-9a-f]*) exit 1 ;; esac
test "${#sha}" -eq 40
case "$run_id" in ''|*[!0-9]*) exit 1 ;; esac
test "${#run_id}" -le 20
case "$phase" in preflight|backup|verify) ;; *) exit 1 ;; esac

root=/backups/native-release
mkdir -p "$root"
test ! -L "$root"
chmod 700 "$root"
exec 9>"$root/lock"
flock -n 9
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
exec 2>"$tmp/error"
url=$(printf '%s' "$DATABASE_URL" | sed 's/options=-c+/options=-c%20/g')
dir="$root/$sha"
origin=https://origin.panda.invalid
access=https://gateway.panda.invalid

sql() {
  psql "$url" -X -qAt -v ON_ERROR_STOP=1 "$@"
}

fetch() {
  curl --silent --show-error --fail --proto '=https' --max-time 20 \
    --max-filesize 65536 --noproxy '*' --output "$2" "$1"
}

assert_sql() {
  test "$(sql "$@")" = t
}

check_ci() {
  api=https://api.github.com/repos/your-org/r-the-drive
  fetch "$api/branches/mainline" "$tmp/branch"
  fetch "$api/actions/workflows/0/runs?branch=mainline&head_sha=$sha&per_page=100" "$tmp/runs"
  fetch "$api/actions/runs/$run_id" "$tmp/run"
  attempt=$(sql --set "data=$(cat "$tmp/run")" <<'SQL'
SELECT (:'data'::jsonb ->> 'run_attempt')::bigint;
SQL
)
  case "$attempt" in ''|*[!0-9]*) exit 1 ;; esac
  fetch "$api/actions/runs/$run_id/attempts/$attempt/jobs?per_page=100" "$tmp/jobs"
  assert_sql --set "branch=$(cat "$tmp/branch")" --set "runs=$(cat "$tmp/runs")" \
    --set "run=$(cat "$tmp/run")" --set "jobs=$(cat "$tmp/jobs")" \
    --set "sha=$sha" --set "run_id=$run_id" <<'SQL'
WITH d AS (SELECT :'branch'::jsonb b, :'runs'::jsonb rs, :'run'::jsonb r, :'jobs'::jsonb j),
required(name) AS (VALUES ('Fork frontend and policy'), ('Fork backend and security regressions'))
SELECT coalesce(
  b->>'name' = 'mainline' AND b->>'protected' = 'true'
  AND b#>>'{commit,sha}' = :'sha'
  AND b#>>'{protection,required_status_checks,enforcement_level}' = 'everyone'
  AND NOT EXISTS (SELECT FROM required WHERE NOT (
    b#>'{protection,required_status_checks,contexts}' @> jsonb_build_array(name)
    AND b#>'{protection,required_status_checks,checks}' @>
      jsonb_build_array(jsonb_build_object('context', name, 'app_id', 15368))))
  AND r->>'id' = :'run_id' AND r->>'workflow_id' = '0'
  AND r->>'path' = '.github/workflows/fork-ci.yml' AND r->>'name' = 'Fork validation'
  AND r->>'event' IN ('push', 'workflow_dispatch') AND r->>'head_branch' = 'mainline'
  AND r->>'head_sha' = :'sha' AND r->>'status' = 'completed' AND r->>'conclusion' = 'success'
  AND r#>>'{repository,id}' = '0'
  AND r#>>'{repository,full_name}' = 'your-org/r-the-drive'
  AND r#>>'{head_repository,id}' = '0'
  AND r#>>'{head_repository,full_name}' = 'your-org/r-the-drive'
  AND (rs->>'total_count')::int = jsonb_array_length(rs->'workflow_runs')
  AND (SELECT x->>'id' FROM jsonb_array_elements(rs->'workflow_runs') x
       ORDER BY (x->>'run_number')::bigint DESC LIMIT 1) = :'run_id'
  AND (j->>'total_count')::int = jsonb_array_length(j->'jobs')
  AND NOT EXISTS (SELECT FROM required WHERE
    (SELECT count(*) FROM jsonb_array_elements(j->'jobs') x WHERE x->>'name' = name) <> 1
    OR NOT EXISTS (SELECT FROM jsonb_array_elements(j->'jobs') x WHERE x->>'name' = name
      AND x->>'run_id' = :'run_id' AND x->>'run_attempt' = r->>'run_attempt'
      AND x->>'head_sha' = :'sha' AND x->>'head_branch' = 'mainline'
      AND x->>'workflow_name' = 'Fork validation'
      AND x->>'status' = 'completed' AND x->>'conclusion' = 'success')), false)
FROM d;
SQL
}

keys() {
  test "$PANDA_REQUIRE_GATEWAY" = true
  test "$PANDA_ENV" = production
  for name in JWT_SECRET JWT_PUBLIC_KEY AGE_ENCRYPTION_KEY DATABASE_URL REDIS_URL PANDA_GATEWAY_TOKEN; do
    value=$(printenv "$name")
    test -n "$value"
    printf '%s ' "$name"
    printf '%s' "$value" | sha256sum | cut -d' ' -f1
  done
}

gates() {
  test "$(curl --silent --show-error --proto '=https' --noproxy '*' --max-time 20 \
    -o /dev/null -w '%{http_code}' "$origin/")" = 403
  test "$(curl --silent --show-error --proto '=https' --noproxy '*' --max-time 20 \
    -o /dev/null -w '%{http_code}' "$origin/health")" = 200
  printf 'X-Panda-Edge: %s\n' "$PANDA_GATEWAY_TOKEN" > "$tmp/header"
  test "$(curl --silent --show-error --proto '=https' --noproxy '*' --max-time 20 \
    -H "@$tmp/header" -o /dev/null -w '%{http_code}' "$origin/v1/vehicles")" = 401
  code=$(curl --silent --show-error --proto '=https' --noproxy '*' --max-time 20 \
    -D "$tmp/access-headers" -o /dev/null -w '%{http_code}' "$access/")
  case "$code" in
    401|403) ;;
    302|303|307|308)
      tr -d '\r' < "$tmp/access-headers" |
        grep -Ei '^location: https://[a-z0-9-]+\.cloudflareaccess\.com/cdn-cgi/access/login([/?]|$)' > /dev/null ;;
    *) exit 1 ;;
  esac
}

audit() {
  sql <<'SQL'
BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;
SELECT jsonb_build_object(
  'ledger', (SELECT jsonb_agg(jsonb_build_object('version',version,
    'checksum',encode(checksum,'hex'),'success',success) ORDER BY version) FROM public._sqlx_migrations),
  'counts', jsonb_build_object(
    'telemetry',(SELECT count(*) FROM timeseries.telemetry),
    'trips',(SELECT count(*) FROM panda.trips),
    'charges',(SELECT count(*) FROM panda.charge_sessions),
    'statePeriods',(SELECT count(*) FROM panda.vehicle_state_periods),
    'vehicles',(SELECT count(*) FROM panda.vehicles),
    'users',(SELECT count(*) FROM panda.users),
    'credentials',(SELECT count(*) FROM panda.vehicle_credentials)),
  'collectorReady', (SELECT count(*) FROM panda.vehicle_credentials) =
    (SELECT count(*) FROM panda.vehicle_credentials c JOIN panda.vehicle_runtime_state r USING(vehicle_id)
     WHERE r.auth_state='authorized' AND r.worker_health='connected'
       AND r.collector_heartbeat_at >= now()-interval '120 seconds' AND r.trip_persistence_error IS NULL),
  'idle', NOT EXISTS(SELECT FROM panda.active_trip_checkpoints t
    JOIN panda.vehicle_credentials c USING(vehicle_id) WHERE t.snapshot#>>'{detector,active_trip_id}' IS NOT NULL),
  'schemaReady', to_regclass('panda.active_trip_checkpoints') IS NOT NULL
    AND to_regclass('panda.pending_trip_completions') IS NOT NULL);
COMMIT;
SQL
}

check_audit() {
  assert_sql --set "data=$(cat "$1")" --set "before=$(cat "$2")" \
    --set "expected=$EXPECTED_LEDGER" --set "need_idle=$3" <<'SQL'
WITH d AS (SELECT :'data'::jsonb a, :'before'::jsonb b)
SELECT coalesce(a->'ledger' = :'expected'::jsonb
  AND a->>'collectorReady' = 'true' AND a->>'schemaReady' = 'true'
  AND (:'need_idle' = 'false' OR a->>'idle' = 'true')
  AND a#>>'{counts,credentials}' = b#>>'{counts,credentials}'
  AND NOT EXISTS(SELECT FROM jsonb_each_text(b->'counts') c
    WHERE (a->'counts'->>c.key)::bigint < c.value::bigint), false) FROM d;
SQL
}

check_catalog() {
  fetch "https://raw.githubusercontent.com/your-org/r-the-drive/$sha/config/native-release-catalog.json" "$tmp/catalog"
  assert_sql --set "catalog=$(cat "$tmp/catalog")" --set "expected=$EXPECTED_LEDGER" <<'SQL'
SELECT :'catalog'::jsonb = :'expected'::jsonb;
SQL
}

case "$phase" in
  preflight)
    test ! -e "$root/attempt"
    test ! -e "$dir"
    check_ci
    check_catalog
    gates
    audit > "$tmp/before"
    check_audit "$tmp/before" "$tmp/before" true
    mkdir "$dir"
    keys > "$dir/keys.sha256"
    cp "$tmp/before" "$dir/before.json"
    printf '%s\n' "$sha" > "$root/attempt"
    sync "$dir/before.json" "$root/attempt"
    ;;
  backup)
    test "$(cat "$root/attempt")" = "$sha"
    test ! -e "$dir/database.dump"
    keys > "$tmp/keys"
    cmp "$tmp/keys" "$dir/keys.sha256"
    check_ci
    check_catalog
    gates
    audit > "$dir/before-deploy.json"
    check_audit "$dir/before-deploy.json" "$dir/before.json" true
    available=$(df -Pk "$root" | awk 'NR==2 {print $4}')
    size=$(sql <<'SQL'
SELECT pg_database_size(current_database()) / 1024;
SQL
)
    test "$available" -gt "$((size * 3 + 131072))"
    pg_dump "$url" --format=custom --no-owner --no-privileges --file="$dir/database.dump"
    test -s "$dir/database.dump"
    pg_restore --list "$dir/database.dump" > "$dir/database.toc"
    (cd "$dir" && sha256sum database.dump > database.sha256)
    sync "$dir/database.dump" "$dir/database.sha256"
    audit > "$tmp/fresh"
    check_audit "$tmp/fresh" "$dir/before-deploy.json" true
    check_ci
    date -u +%s > "$dir/backup-time"
    ;;
  verify)
    test "$(cat "$root/attempt")" = "$sha"
    test "$(cat /app/release-sha)" = "$sha"
    test "$(($(date -u +%s) - $(cat "$dir/backup-time")))" -lt 900
    (cd "$dir" && sha256sum --check --status database.sha256)
    keys > "$tmp/keys"
    cmp "$tmp/keys" "$dir/keys.sha256"
    gates
    ready=false
    for step in 1 2 3 4 5 6 7 8 9 10 11 12; do
      audit > "$dir/after.json"
      if check_audit "$dir/after.json" "$dir/before-deploy.json" false; then ready=true; break; fi
      sleep 10
    done
    test "$ready" = true
    date -u +%FT%TZ > "$dir/success"
    sync "$dir/success"
    rm "$root/attempt"
    ;;
esac
printf 'PANDA_NATIVE_%s_OK %s %s\n' "$phase" "$sha" "$(date -u +%s)"
