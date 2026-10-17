#!/bin/sh
set -eu

for endpoint in http://localhost:8080/health http://127.0.0.1:3002/health; do
  case "$endpoint" in
    *:3002/*) [ -f /tmp/panda-restore-in-progress ] || break ;;
  esac
  if curl -fsS "$endpoint" >/dev/null; then exit 0; else status=$?; fi
  case "$endpoint" in *:3002/*) exit "$status" ;; esac
done
exit 1
