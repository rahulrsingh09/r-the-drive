#!/bin/sh
set -eu
export LC_ALL=C

fail() { printf '%s\n' "$1" >&2; exit 1; }
destination=${1:-/tmp/panda-origin-gate.conf}
token=${PANDA_GATEWAY_TOKEN:-}
requirement=${PANDA_REQUIRE_GATEWAY:-false}
case "$requirement" in
  true|1|false|0|'') ;;
  *) fail 'Invalid gateway requirement setting.' ;;
esac

default=1
case "$token" in
  '')
    case "$requirement" in true|1) fail 'The required origin gateway token is missing.' ;; esac
    ;;
  *[!A-Za-z0-9_-]*) fail 'Invalid origin gateway token format.' ;;
  *)
    length=${#token}
    [ "$length" -ge 43 ] && [ "$length" -le 128 ] || fail 'Invalid origin gateway token length.'
    default=0
    ;;
esac

umask 077
staging=$(mktemp "${destination}.XXXXXX")
trap 'rm -f "$staging"' 0
trap 'exit 1' HUP INT TERM
{
  printf 'map $http_x_panda_edge $panda_origin_authorized {\n    default %s;\n' "$default"
  [ "$default" = 1 ] || printf '    ~^%s$ 1;\n' "$token"
  printf '}\n'
} > "$staging"
mv "$staging" "$destination"
