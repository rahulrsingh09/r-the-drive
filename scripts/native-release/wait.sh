#!/bin/sh
set -eu
umask 077

mode=$1
phase=$2
sha=$(printf %s "$3" | base64 -d)
run_id=$(printf %s "$4" | base64 -d)
operation=$(printf %s "${5-}" | base64 -d)
root=/backups/native-release

valid_digits() {
  case "$1" in ''|*[!0-9]*) return 1 ;; esac
}
epoch() {
  valid_digits "$1" && test "${#1}" = 10
}
secure_dir() {
  for rule in -d -O; do test "$rule" "$1" || return 1; done
  test ! -L "$1" || return 1
  case "$(stat -c %a "$1")" in 700|2700) return 0 ;; *) return 1 ;; esac
}
private_file() {
  test ! -L "$1" && test -O "$1" && test -f "$1" || return 1
  test "$(stat -c %a "$1")" = 600 && test "$(wc -c < "$1")" -le 1024
}
check_deadline() {
  current=$(date -u +%s)
  test "$current" -lt "$deadline" && test "$current" -ge "$created"
}
marker_ok() {
  marker=$(cat "$1")
  stamp=${marker##* }
  epoch "$stamp" || return 1
  test "$stamp" -ge "$created" && test "$stamp" -le "$finished" || return 1
  test "$marker" = "PANDA_NATIVE_${phase}_OK $sha $stamp" || return 1
  printf '%s\n' "$marker" | cmp -s - "$1"
}
validate_result() {
  private_file "$op/result"
  records=$(awk -v identity="$sha $run_id $phase $operation 0" -v attestation="PANDA_NATIVE_${phase}_OK $sha" \
    -v created="$created" -v deadline="$deadline" -v current="$current" '
    function reject() { failed=1; exit 1 }
    function epoch(value) { return length(value)==10 && value ~ /^[0-9]+$/ }
    NR==1 {
      if ($0 != identity " " $6 || !epoch($6)) reject()
      finished=$6
      if (finished < created || finished >= deadline || finished > current) reject()
      metadata=$0
      next
    }
    NR==2 {
      if ($0 != attestation " " $3 || !epoch($3) || $3 < created || $3 > finished) reject()
      marker=$0
      next
    }
    { reject() }
    END {
      if (failed || NR!=2) exit 1
      print metadata
      print marker
    }
  ' "$op/result")
  printf '%s\n' "$records" | cmp -s - "$op/result"
  marker=$(printf '%s\n' "$records" | sed -n '2p')
}
prepare() {
  test -z "$operation"
  op=$(mktemp -d "$root/async/op.XXXXXXXXXXXXXXXX")
  printf '%s %s %s\n' "$phase" "$sha" "$run_id" > "$op/identity"
  date -u +%s > "$op/created"
  for file in guard control; do
    case "$file" in guard) encoded=$GUARD_B64 ;; control) encoded=$CONTROL_B64 ;; esac
    printf %s "$encoded" | base64 -d > "$op/$file.sh"
  done
  sync "$op/identity" "$op/created" "$op/guard.sh" "$op/control.sh" "$op"
  printf 'PANDA_ASYNC_OP %s\n' "${op##*/}"
}
load_operation() {
  case "$operation" in op.*) suffix=${operation#op.} ;; *) return 1 ;; esac
  test "${#suffix}" = 16
  case "$suffix" in *[!a-zA-Z0-9]*) return 1 ;; esac
  op="$root/async/$operation"
  secure_dir "$op"
  for file in identity created; do private_file "$op/$file"; done
  test "$(cat "$op/identity")" = "$phase $sha $run_id"
  created=$(cat "$op/created")
  epoch "$created"
  deadline=$((created + 300))
  check_deadline
}
worker() {
  mkdir "$op/claimed"
  test ! -L "$op/guard.sh" && test -f "$op/guard.sh"
  expected=$(printf %s "$GUARD_B64" | base64 -d | sha256sum | cut -d' ' -f1)
  actual=$(sha256sum "$op/guard.sh" | cut -d' ' -f1)
  test "$actual" = "$expected"
  remaining=$((deadline - current - 5))
  test "$remaining" -gt 0
  code=0
  timeout --kill-after=5 "$remaining" sh "$op/guard.sh" "$phase" "$1" "$2" > "$op/output" 2> "$op/error" || code=$?
  finished=$(date -u +%s)
  if test "$code" -eq 0; then
    if test "$(wc -c < "$op/output")" -gt 128 || ! marker_ok "$op/output"; then code=1; fi
  fi
  if test "$finished" -ge "$deadline"; then code=1; fi
  {
    printf '%s %s %s %s %s %s\n' "$sha" "$run_id" "$phase" "$operation" "$code" "$finished"
    if test "$code" -eq 0; then cat "$op/output"; fi
  } > "$op/result.tmp"
  sync "$op/result.tmp"
  mv "$op/result.tmp" "$op/result"
  sync "$op"
  rm -f "$op/output" "$op/error"
  test "$code" -eq 0
}

case "$mode:$phase" in
  prepare:preflight|prepare:backup|prepare:verify|worker:preflight|worker:backup|worker:verify|poll:preflight|poll:backup|poll:verify|collect:preflight|collect:backup|collect:verify) ;;
  *) exit 1 ;;
esac
case "$sha" in ''|*[!0-9a-f]*) exit 1 ;; esac
test "${#sha}" = 40
valid_digits "$run_id"
case "$run_id" in 0*) exit 1 ;; esac
test "${#run_id}" -le 20
mkdir -p "$root"
secure_dir "$root"
if test ! -e "$root/async"; then mkdir "$root/async"; fi
secure_dir "$root/async"
if test "$mode" = prepare; then prepare; exit; fi
load_operation
case "$mode" in
  worker) worker "$3" "$4" ;;
  poll)
    if test ! -e "$op/result"; then sleep 5; fi
    check_deadline
    if test -e "$op/result"; then validate_result; else printf PANDA_ASYNC_PENDING; fi
    ;;
  collect) validate_result; printf '%s\n' "$marker" ;;
esac
