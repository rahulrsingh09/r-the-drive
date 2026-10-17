#!/bin/bash
set -u

declare -A children=()

format_stderr() {
  local source=$1 line level
  while IFS= read -r line || [ -n "$line" ]; do
    level=ERROR
    if [ "$source" = nginx ]; then
      case "$line" in
        *'[warn]'*) level=WARN ;;
        *'[notice]'*|*'[info]'*|*'[debug]'*) level=INFO ;;
      esac
      if [[ "$line" =~ ^[^\[]+\[[^]]+\][^:]*:[[:space:]]?(.*)$ ]]; then line=${BASH_REMATCH[1]}; fi
    fi
    line=${line//\\/\\\\}
    line=${line//\"/\\\"}
    printf '[panda][%s] source=%s message="%s"\n' "$level" "$source" "$line" >&2
  done
}

launch() {
  local name=$1
  shift
  "$@" 2> >(format_stderr "$name") &
  children[$name]=$!
  if [ "$name" = api ]; then printf '%s\n' "${children[api]}" > /tmp/panda-api.pid; fi
}

shutdown() {
  trap - TERM INT
  local name
  for name in api restore-agent nginx; do
    if [ -n "${children[$name]:-}" ]; then kill -TERM "${children[$name]}" 2>/dev/null || true; fi
  done
  wait 2>/dev/null || true
}

prepare_storage() {
  local directory probe key
  mkdir -p /backups /data/cache/panda/vehicle-images || return 1
  for directory in /backups /data/cache; do
    if [ ! -w "$directory" ]; then
      printf 'Panda cannot write to %s as UID %s.\n' "$directory" "$(id -u)" >&2
      printf '%s\n' 'For a bind mount, grant the container user write access to the host folder.' >&2
      return 1
    fi
    probe=$(mktemp "$directory/.panda-write-test.XXXXXX" 2>/dev/null) || {
      printf 'Panda cannot create a writable storage probe in %s.\n' "$directory" >&2
      return 1
    }
    if ! printf '%s\n' 'storage probe' > "$probe"; then rm -f "$probe"; return 1; fi
    sync -f "$probe" 2>/dev/null || true
    rm -f "$probe"
  done
  [ ! -s /backups/.restore-agent-key ] || return 0
  umask 077
  key=$(mktemp /backups/.restore-agent-key.tmp.XXXXXX) || return 1
  if (set -o pipefail; dd if=/dev/urandom bs=32 count=1 2>/dev/null | base64 > "$key") && [ -s "$key" ]; then
    mv "$key" /backups/.restore-agent-key || { rm -f "$key"; return 1; }
  else
    rm -f "$key"
    return 1
  fi
}

supervise() {
  local mode=running api_status=0 name
  while true; do
    for name in nginx restore-agent; do
      kill -0 "${children[$name]}" 2>/dev/null || return 1
    done
    case "$mode" in
      running)
        if ! kill -0 "${children[api]}" 2>/dev/null; then
          wait "${children[api]}"; api_status=$?
          [ -f /tmp/panda-restore-in-progress ] || return "$api_status"
          mode=restoring
        fi
        ;;
      restoring)
        if [ ! -f /tmp/panda-restore-in-progress ]; then
          launch api /app/panda-api
          mode=running
        fi
        ;;
    esac
    if [ "$mode" = restoring ]; then sleep 1; else sleep 0.2; fi
  done
}

trap 'shutdown; exit 0' TERM INT
prepare_storage || exit 1
/app/render-origin-gate.sh || exit 1
unset PANDA_GATEWAY_TOKEN
launch restore-agent /app/panda-restore-agent
launch nginx nginx -g 'daemon off;'
launch api /app/panda-api
supervise
status=$?
shutdown
exit "$status"
