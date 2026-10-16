#!/bin/sh
set -eu
export LC_ALL=C

destination=${1:-/tmp/panda-origin-gate.conf}
token=${PANDA_GATEWAY_TOKEN:-}
case "${PANDA_REQUIRE_GATEWAY:-false}" in
  true|1) required=1 ;;
  false|0|'') required=0 ;;
  *) echo "Invalid gateway requirement setting." >&2; exit 1 ;;
esac

if [ -z "$token" ]; then
  if [ "$required" = 1 ]; then
    echo "The required origin gateway token is missing." >&2
    exit 1
  fi
else
  case "$token" in
    *[!A-Za-z0-9_-]*) echo "Invalid origin gateway token format." >&2; exit 1 ;;
  esac
  if [ "${#token}" -lt 43 ] || [ "${#token}" -gt 128 ]; then
    echo "Invalid origin gateway token length." >&2
    exit 1
  fi
fi

umask 077
temporary=$(mktemp "${destination}.XXXXXX")
trap 'rm -f "$temporary"' EXIT HUP INT TERM
if [ -n "$token" ]; then
  printf 'map $http_x_panda_edge $panda_origin_authorized {\n    default 0;\n    ~^%s$ 1;\n}\n' "$token" > "$temporary"
else
  printf 'map $http_x_panda_edge $panda_origin_authorized {\n    default 1;\n}\n' > "$temporary"
fi
mv "$temporary" "$destination"
