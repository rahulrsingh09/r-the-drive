#!/bin/sh
set -eu
umask 077

phase=$1
sha=$(printf %s "$2" | base64 -d)
run_id=$(printf %s "$3" | base64 -d)
case "$phase" in preflight|backup|verify) ;; *) exit 1 ;; esac
case "$sha" in ''|*[!0-9a-f]*) exit 1 ;; esac
test "${#sha}" = 40
case "$run_id" in ''|*[!0-9]*) exit 1 ;; esac
test "${#run_id}" -le 20
root=/backups/native-release
mkdir -p "$root"
test ! -L "$root"
chmod 700 "$root"
exec 9>"$root/lock"
flock -n 9
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
exec 2>"$tmp/error"
url=$(printf %s "$DATABASE_URL" | sed 's/options=-c+/options=-c%20/g')
dir="$root/$sha"
origin=https://origin.panda.invalid
access=https://gateway.panda.invalid

sql() { psql "$url" -X -qAt -v ON_ERROR_STOP=1 "$@"; }
assert_sql() {
  answer=$(sql "$@")
  test "$answer" = t
}
fetch() {
  destination=$2
  curl --silent --show-error --fail --proto '=https' --max-time 20 --max-filesize 65536 --noproxy '*' --output "$destination" "$1"
}
check_ci() {
  api=https://api.github.com/repos/your-org/r-the-drive
  for document in branch runs run; do
    case "$document" in
      branch) path=branches/mainline ;;
      runs) path="actions/workflows/0/runs?branch=mainline&head_sha=$sha&per_page=100" ;;
      run) path="actions/runs/$run_id" ;;
    esac
    fetch "$api/$path" "$tmp/$document"
  done
  attempt=$(sql --set "data=$(cat "$tmp/run")" <<'SQL'
SELECT cast(jsonb_extract_path_text(:'data'::jsonb, 'run_attempt') AS bigint);
SQL
)
  case "$attempt" in ''|*[!0-9]*) exit 1 ;; esac
  fetch "$api/actions/runs/$run_id/attempts/$attempt/jobs?per_page=100" "$tmp/jobs"
  assert_sql --set "branch=$(cat "$tmp/branch")" --set "runs=$(cat "$tmp/runs")" \
    --set "run=$(cat "$tmp/run")" --set "jobs=$(cat "$tmp/jobs")" --set "sha=$sha" --set "run_id=$run_id" <<'SQL'
WITH documents AS (
  SELECT :'branch'::jsonb AS branch, :'runs'::jsonb AS runs, :'run'::jsonb AS run, :'jobs'::jsonb AS jobs
), required(name) AS (VALUES ('Fork frontend and policy'), ('Fork backend and security regressions')),
run_fields(key, expected) AS (VALUES
  ('id', :'run_id'), ('workflow_id','0'), ('path','.github/workflows/fork-ci.yml'),
  ('name','Fork validation'), ('head_branch','mainline'), ('head_sha', :'sha'),
  ('status','completed'), ('conclusion','success')),
checks AS (
 SELECT
   branch->>'name' = 'mainline' AND branch->>'protected' = 'true'
     AND branch#>>'{commit,sha}' = :'sha'
     AND branch#>>'{protection,required_status_checks,enforcement_level}' = 'everyone' AS protected,
   NOT EXISTS (SELECT 1 FROM required WHERE NOT (
     branch#>'{protection,required_status_checks,contexts}' @> jsonb_build_array(name)
     AND branch#>'{protection,required_status_checks,checks}' @>
       jsonb_build_array(jsonb_build_object('context',name,'app_id',15368)))) AS signed_checks,
   NOT EXISTS (SELECT 1 FROM run_fields WHERE (run->>key) IS DISTINCT FROM expected)
     AND run->>'event' IN ('push','workflow_dispatch')
     AND run#>>'{repository,id}' = '0' AND run#>>'{head_repository,id}' = '0'
     AND run#>>'{repository,full_name}' = 'your-org/r-the-drive'
     AND run#>>'{head_repository,full_name}' = 'your-org/r-the-drive' AS exact_run,
   (runs->>'total_count')::int = jsonb_array_length(runs->'workflow_runs')
     AND (SELECT item->>'id' FROM jsonb_array_elements(runs->'workflow_runs') item
          ORDER BY (item->>'run_number')::bigint DESC LIMIT 1) = :'run_id' AS newest_run,
   (jobs->>'total_count')::int = jsonb_array_length(jobs->'jobs')
     AND NOT EXISTS (SELECT 1 FROM required WHERE
       (SELECT count(*) FROM jsonb_array_elements(jobs->'jobs') item WHERE item->>'name'=name) <> 1
       OR NOT EXISTS (SELECT 1 FROM jsonb_array_elements(jobs->'jobs') item
         WHERE item->>'name'=name AND item->>'run_id'=:'run_id'
           AND item->>'run_attempt'=run->>'run_attempt'
           AND item->>'head_sha'=:'sha' AND item->>'head_branch'='mainline'
           AND item->>'workflow_name'='Fork validation'
           AND item->>'status'='completed' AND item->>'conclusion'='success')) AS exact_jobs
 FROM documents
)
SELECT coalesce(protected AND signed_checks AND exact_run AND newest_run AND exact_jobs, false) FROM checks;
SQL
}
keys() {
  test "$PANDA_REQUIRE_GATEWAY:$PANDA_ENV" = true:production
  for name in JWT_SECRET JWT_PUBLIC_KEY AGE_ENCRYPTION_KEY DATABASE_URL REDIS_URL PANDA_GATEWAY_TOKEN; do
    value=$(printenv "$name")
    test -n "$value"
    hash=$(printf %s "$value" | sha256sum | cut -d' ' -f1)
    printf '%s %s\n' "$name" "$hash"
  done
}
http_code() {
  curl --silent --show-error --proto '=https' --noproxy '*' --max-time 20 -o /dev/null -w '%{http_code}' "$@"
}
gates() {
  test "$(http_code "$origin/")" = 403
  test "$(http_code "$origin/health")" = 200
  printf 'X-Panda-Edge: %s\n' "$PANDA_GATEWAY_TOKEN" > "$tmp/header"
  test "$(http_code -H "@$tmp/header" "$origin/v1/vehicles")" = 401
  code=$(http_code -D "$tmp/access-headers" "$access/")
  case "$code" in
    401|403) return ;;
    302|303|307|308)
      tr -d '\r' < "$tmp/access-headers" |
        grep -Ei '^location: https://[a-z0-9-]+\.cloudflareaccess\.com/cdn-cgi/access/login([/?]|$)' > /dev/null ;;
    *) return 1 ;;
  esac
}
audit() {
  sql <<'SQL'
BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;
WITH history(name, amount) AS (
 SELECT 'telemetry', count(*) FROM timeseries.telemetry UNION ALL
 SELECT 'trips', count(*) FROM panda.trips UNION ALL
 SELECT 'charges', count(*) FROM panda.charge_sessions UNION ALL
 SELECT 'statePeriods', count(*) FROM panda.vehicle_state_periods UNION ALL
 SELECT 'vehicles', count(*) FROM panda.vehicles UNION ALL
 SELECT 'users', count(*) FROM panda.users UNION ALL
 SELECT 'credentials', count(*) FROM panda.vehicle_credentials
), healthy AS (
 SELECT count(*) AS amount FROM panda.vehicle_credentials c JOIN panda.vehicle_runtime_state r USING(vehicle_id)
 WHERE r.auth_state='authorized' AND r.worker_health='connected'
   AND r.collector_heartbeat_at >= now()-interval '120 seconds' AND r.trip_persistence_error IS NULL
), ledger AS (
 SELECT jsonb_agg(jsonb_build_object('version',version,'checksum',encode(checksum,'hex'),'success',success)
                  ORDER BY version) AS entries FROM public._sqlx_migrations
)
SELECT jsonb_build_object('ledger', (SELECT entries FROM ledger),
 'counts', (SELECT jsonb_object_agg(name,amount) FROM history),
 'collectorReady', (SELECT amount FROM history WHERE name='credentials') = (SELECT amount FROM healthy),
 'idle', NOT EXISTS (SELECT 1 FROM panda.active_trip_checkpoints t JOIN panda.vehicle_credentials c USING(vehicle_id)
                    WHERE t.snapshot#>>'{detector,active_trip_id}' IS NOT NULL),
 'schemaReady', to_regclass('panda.active_trip_checkpoints') IS NOT NULL AND to_regclass('panda.pending_trip_completions') IS NOT NULL);
COMMIT;
SQL
}
check_audit() {
  assert_sql --set "data=$(cat "$1")" --set "before=$(cat "$2")" --set "expected=$EXPECTED_LEDGER" --set "need_idle=$3" <<'SQL'
WITH snapshots AS (SELECT :'data'::jsonb current, :'before'::jsonb previous),
policy AS (
 SELECT current->'ledger' = :'expected'::jsonb AS approved,
 current->>'collectorReady' = 'true' AND current->>'schemaReady' = 'true' AS ready,
 :'need_idle' = 'false' OR current->>'idle' = 'true' AS idle,
 current#>>'{counts,credentials}' = previous#>>'{counts,credentials}' AS credentials,
 NOT EXISTS (SELECT 1 FROM jsonb_each_text(previous->'counts') row
             WHERE (current->'counts'->>row.key)::bigint < row.value::bigint) AS retained
 FROM snapshots
)
SELECT coalesce(approved AND ready AND idle AND credentials AND retained, false) FROM policy;
SQL
}
check_catalog() {
  fetch "https://raw.githubusercontent.com/your-org/r-the-drive/$sha/config/native-release-catalog.json" "$tmp/catalog"
  assert_sql --set "catalog=$(cat "$tmp/catalog")" --set "expected=$EXPECTED_LEDGER" <<'SQL'
SELECT NOT (:'catalog'::jsonb IS DISTINCT FROM :'expected'::jsonb);
SQL
}
readiness() {
  check_ci
  check_catalog
  gates
}
unchanged_keys() {
  keys > "$tmp/keys"
  cmp "$tmp/keys" "$dir/keys.sha256"
}
preflight() {
  for path in "$root/attempt" "$dir"; do test ! -e "$path"; done
  readiness
  audit > "$tmp/before"
  check_audit "$tmp/before" "$tmp/before" true
  mkdir "$dir"
  keys > "$dir/keys.sha256"
  cp "$tmp/before" "$dir/before.json"
  printf '%s\n' "$sha" > "$root/attempt"
  sync "$dir/before.json" "$root/attempt"
}
backup() {
  test ! -e "$dir/database.dump"
  unchanged_keys
  readiness
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
}
verify() {
  test "$(cat /app/release-sha)" = "$sha"
  age=$(($(date -u +%s) - $(cat "$dir/backup-time")))
  test "$age" -lt 900
  (cd "$dir" && sha256sum --check --status database.sha256)
  unchanged_keys
  gates
  attempts=12
  ready=false
  while test "$attempts" -gt 0; do
    audit > "$dir/after.json"
    if check_audit "$dir/after.json" "$dir/before-deploy.json" false; then ready=true; break; fi
    attempts=$((attempts - 1))
    sleep 10
  done
  test "$ready" = true
  date -u +%FT%TZ > "$dir/success"
  sync "$dir/success"
  rm "$root/attempt"
}

if test "$phase" != preflight; then test "$(cat "$root/attempt")" = "$sha"; fi
"$phase"
printf 'PANDA_NATIVE_%s_OK %s %s\n' "$phase" "$sha" "$(date -u +%s)"
