#!/usr/bin/env bash
# Node 7 end-to-end run against a real host binary and real docker.
# ROBROZ_MODEL=echo keeps the runner echoing the prompt (no live model).
# The runner image then applies that prompt deterministically.
# Flow: create task -> sandbox blocks on approval -> host restarts -> approve
# (resume) -> blocks on the next line only -> restart -> approve -> done once.
# Then restart again and check nothing is replayed.
#
# Needs docker, curl, jq. Run from the repo root: scripts/e2e.sh
set -euo pipefail

IMAGE="${ROBROZ_IMAGE:-robroz-runner:e2e}"
PORT="${E2E_PORT:-18790}"
BASE="http://127.0.0.1:${PORT}"
WORK="$(mktemp -d)"
HOST_PID=""
export ROBROZ_ADDR="127.0.0.1:${PORT}" ROBROZ_DB="$WORK/robroz.db" \
  ROBROZ_VOLUMES="$WORK/groups" ROBROZ_CONTROL="$WORK/control" ROBROZ_IMAGE="$IMAGE" \
  ROBROZ_MODEL=echo

log() { printf '[e2e] %s\n' "$*"; }
fail() { printf '[e2e] FAIL: %s\n' "$*" >&2; [ -f "$WORK/host.log" ] && tail -n 40 "$WORK/host.log" >&2; exit 1; }

stop_host() {
  if [ -n "$HOST_PID" ] && kill -0 "$HOST_PID" 2>/dev/null; then
    kill "$HOST_PID"; wait "$HOST_PID" 2>/dev/null || true
  fi
  HOST_PID=""
}
cleanup() { stop_host; rm -rf "$WORK"; }
trap cleanup EXIT

start_host() {
  ./target/debug/robroz-host >>"$WORK/host.log" 2>&1 &
  HOST_PID=$!
  for _ in $(seq 1 50); do
    # 404 for an unknown group means the server is up.
    [ "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/api/groups/none/events")" = 404 ] && return 0
    sleep 0.2
  done
  fail "host did not start"
}

api() { # method path [json]
  local method=$1 path=$2 body=${3:-}
  if [ -n "$body" ]; then
    curl -sS -X "$method" -H 'content-type: application/json' -d "$body" "$BASE$path"
  else
    curl -sS -X "$method" "$BASE$path"
  fi
}

events() { api GET "/api/groups/$GROUP/events"; }
task_state() { events | jq -r --arg t "$TASK" '.events[] | select(.type=="status" and .taskId==$t) | .state'; }
delivery_count() { events | jq '[.events[] | select(.type=="delivery")] | length'; }

wait_state() { # expected
  for _ in $(seq 1 150); do
    [ "$(task_state)" = "$1" ] && return 0
    sleep 0.2
  done
  fail "task never reached $1 (now: $(task_state))"
}

log "build runner image and host"
docker build -q -f runner/Dockerfile -t "$IMAGE" . >/dev/null
cargo build -q -p robroz-host

start_host
GROUP=$(api POST /api/groups '{"name":"e2e"}' | jq -r .id)
api POST "/api/groups/$GROUP/members" '{"botId":"bot-e2e","name":"E2E"}' >/dev/null
PROMPT=$'第一行\nrobroz-action:external\n中间\nrobroz-action:captcha\n最后一行'
TASK=$(api POST "/api/groups/$GROUP/tasks" "$(jq -nc --arg p "$PROMPT" '{assigneeBotId:"bot-e2e",prompt:$p}')" | jq -r .id)
[ -n "$TASK" ] && [ "$TASK" != null ] || fail "task not created"
log "task $TASK created"

wait_state blocked
[ "$(delivery_count)" = 0 ] || fail "delivery before approval"
log "blocked on approval; restarting host"
stop_host; start_host
[ "$(task_state)" = blocked ] || fail "restart lost blocked state"

code=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/api/tasks/$TASK/resume")
[ "$code" = 200 ] || fail "resume 1 returned $code"
wait_state blocked
# Second stop must be the captcha line, not the external line again.
code=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/api/tasks/$TASK/resume")
[ "$code" = 409 ] && fail "task not blocked for second approval"
[ "$code" = 200 ] || fail "resume 2 returned $code"
TAKEOVER="$ROBROZ_CONTROL/$TASK/takeover"
[ "$(grep -c . "$TAKEOVER")" = 2 ] || fail "expected 2 takeover hashes"
wait_state done
log "done after two approvals"

RESULT="$ROBROZ_VOLUMES/$GROUP/tasks/$TASK/result.txt"
[ -f "$RESULT" ] || RESULT=$(find "$ROBROZ_VOLUMES" -path "*tasks/$TASK/result.txt" | head -n1)
[ -f "$RESULT" ] || fail "result.txt missing"
expected=$'第一行\n中间\n最后一行'
[ "$(cat "$RESULT")" = "$expected" ] || fail "unexpected result: $(cat "$RESULT")"
[ "$(delivery_count)" = 1 ] || fail "expected exactly one delivery"

log "restart after done; nothing may replay"
before=$(stat -c %Y "$RESULT")
stop_host; start_host
sleep 3
[ "$(task_state)" = done ] || fail "done task changed state after restart"
[ "$(delivery_count)" = 1 ] || fail "delivery replayed after restart"
[ "$(stat -c %Y "$RESULT")" = "$before" ] || fail "result rewritten after restart"
code=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/api/tasks/$TASK/resume")
[ "$code" = 409 ] || fail "resume on done task returned $code"
[ -z "$(docker ps -aq --filter "ancestor=$IMAGE")" ] || fail "containers left behind"

log "PASS"
