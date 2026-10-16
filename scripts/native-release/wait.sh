#!/bin/sh
set -eu
umask 077

mode=$1
phase=$2
sha=$(printf %s "$3" | base64 -d)
run_id=$(printf %s "$4" | base64 -d)
operation=$(printf %s "${5-}" | base64 -d)
case "$mode" in prepare|worker|poll|collect) ;; *) exit 1 ;; esac
case "$phase" in preflight|backup|verify) ;; *) exit 1 ;; esac
case "$sha" in ''|*[!0-9a-f]*) exit 1 ;; esac
test "${#sha}" -eq 40
case "$run_id" in ''|0*|*[!0-9]*) exit 1 ;; esac
test "${#run_id}" -le 20

root=/backups/native-release
secure_dir() {
  test -d "$1" && test ! -L "$1" && test -O "$1" || return 1
  case "$(stat -c %a "$1")" in 700|2700) ;; *) return 1 ;; esac
}
private_file() {
  test -f "$1" && test ! -L "$1" && test -O "$1" || return 1
  test "$(stat -c %a "$1")" = 600
  test "$(wc -c < "$1")" -le 1024
}
epoch() {
  case "$1" in ''|*[!0-9]*) return 1 ;; esac
  test "${#1}" -eq 10
}
mkdir -p "$root"
secure_dir "$root"
if test ! -e "$root/async"; then mkdir "$root/async"; fi
secure_dir "$root/async"
if test "$mode" = prepare; then
  test -z "$operation"
  op=$(mktemp -d "$root/async/op.XXXXXXXXXXXXXXXX")
  printf '%s %s %s\n' "$phase" "$sha" "$run_id" > "$op/identity"
  date -u +%s > "$op/created"
  printf %s "$GUARD_B64" | base64 -d > "$op/guard.sh"
  printf %s "$CONTROL_B64" | base64 -d > "$op/control.sh"
  sync "$op/identity" "$op/created" "$op/guard.sh" "$op/control.sh" "$op"
  printf 'PANDA_ASYNC_OP %s\n' "${op##*/}"
  exit
fi
case "$operation" in op.*) ;; *) exit 1 ;; esac
suffix=${operation#op.}
test "${#suffix}" -eq 16
case "$suffix" in *[!a-zA-Z0-9]*) exit 1 ;; esac
op="$root/async/$operation"
secure_dir "$op"
private_file "$op/identity"
private_file "$op/created"
test "$(cat "$op/identity")" = "$phase $sha $run_id"
created=$(cat "$op/created")
epoch "$created"
deadline=$((created + 300))
check_deadline() {
  current=$(date -u +%s)
  test "$current" -ge "$created" && test "$current" -lt "$deadline"
}
check_deadline

validate_result() {
  private_file "$op/result"
  IFS=' ' read -r result_sha result_run result_phase result_op result_code finished < "$op/result"
  test "$result_sha $result_run $result_phase $result_op" = "$sha $run_id $phase $operation"
  test "$result_code" = 0
  epoch "$finished"
  test "$finished" -ge "$created" && test "$finished" -le "$current"
  test "$finished" -lt "$deadline"
  marker=$(sed -n '2p' "$op/result")
  stamp=${marker##* }
  epoch "$stamp"
  test "$stamp" -ge "$created" && test "$stamp" -le "$finished"
  test "$marker" = "PANDA_NATIVE_${phase}_OK $sha $stamp"
  printf '%s %s %s %s 0 %s\n%s\n' "$sha" "$run_id" "$phase" "$operation" "$finished" "$marker" |
    cmp -s - "$op/result"
}

case "$mode" in
  worker)
    mkdir "$op/claimed"
    test -f "$op/guard.sh" && test ! -L "$op/guard.sh"
    expected=$(printf %s "$GUARD_B64" | base64 -d | sha256sum | cut -d' ' -f1)
    test "$(sha256sum "$op/guard.sh" | cut -d' ' -f1)" = "$expected"
    remaining=$((deadline - current - 5))
    test "$remaining" -gt 0
    code=0
    timeout --kill-after=5 "$remaining" sh "$op/guard.sh" "$phase" "$3" "$4" \
      > "$op/output" 2> "$op/error" || code=$?
    finished=$(date -u +%s)
    if test "$code" -eq 0; then
      test "$(wc -c < "$op/output")" -le 128 || code=1
      marker=$(cat "$op/output")
      stamp=${marker##* }
      epoch "$stamp" || code=1
      if test "$code" -eq 0; then
        test "$stamp" -ge "$created" && test "$stamp" -le "$finished" || code=1
        test "$marker" = "PANDA_NATIVE_${phase}_OK $sha $stamp" || code=1
        printf '%s\n' "$marker" | cmp -s - "$op/output" || code=1
      fi
    fi
    test "$finished" -lt "$deadline" || code=1
    printf '%s %s %s %s %s %s\n' "$sha" "$run_id" "$phase" "$operation" "$code" "$finished" \
      > "$op/result.tmp"
    if test "$code" -eq 0; then cat "$op/output" >> "$op/result.tmp"; fi
    sync "$op/result.tmp"
    mv "$op/result.tmp" "$op/result"
    sync "$op"
    rm -f "$op/output" "$op/error"
    test "$code" -eq 0
    ;;
  poll)
    if test ! -e "$op/result"; then sleep 5; fi
    check_deadline
    if test -e "$op/result"; then validate_result; else printf PANDA_ASYNC_PENDING; fi
    ;;
  collect)
    validate_result
    printf '%s\n' "$marker"
    ;;
esac
