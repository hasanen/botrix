#!/usr/bin/env bash
# Black-box tests for the Botrix container image (plan §15.5).
#
# Usage: scripts/container-test.sh [IMAGE]
#   IMAGE   image to test (default: botrix:test)
#   CONTAINER_ENGINE   "docker" or "podman" (default: whichever is installed)
#
# Starts real containers from IMAGE and checks: it starts and passes its
# health check, serves the UI, runs as UID 1000, mounts /config read-only and
# /data read-write, keeps state across a stop/start, refuses weak secrets, and
# never writes secrets to its logs.

set -euo pipefail

IMAGE="${1:-botrix:test}"
ENGINE="${CONTAINER_ENGINE:-}"
if [[ -z "$ENGINE" ]]; then
  if command -v podman >/dev/null 2>&1; then ENGINE=podman
  elif command -v docker >/dev/null 2>&1; then ENGINE=docker
  else echo "error: neither podman nor docker found" >&2; exit 2
  fi
fi

SESSION_SECRET="container-test-session-secret-0123456789abcdef"
CRYPTO_PASSPHRASE="container-test-crypto-passphrase-xyz"
ADMIN_PASSWORD="container-test-admin-password-1"
NAME="botrix-container-test-$$"

# Bind mounts live inside the repo: with podman on macOS only paths under the
# home directory are visible to the VM, so /tmp would not work.
WORK="$(mktemp -d "${PWD}/.container-test.XXXXXX")"
JAR="${WORK}/cookies.txt"
mkdir -p "${WORK}/data" "${WORK}/config"
chmod -R a+rwx "${WORK}/data"
cat > "${WORK}/config/config.yaml" <<'YAML'
matrix:
  homeserver: https://matrix.example.com
  destinations:
    alerts:
      room_id: "!abc123:example.com"
      enabled: true
webhooks:
  hook:
    enabled: true
    destination: alerts
    msg_template: "Hello ${name}"
server:
  host: "0.0.0.0"
  port: 8080
YAML
chmod -R a+rX "${WORK}/config"

cleanup() {
  "$ENGINE" rm -f "$NAME" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

PASSED=0
FAILED=0
pass() { PASSED=$((PASSED + 1)); printf '  ok   %s\n' "$1"; }
fail() { FAILED=$((FAILED + 1)); printf '  FAIL %s\n' "$1"; }
check() { # check "description" command...
  local desc="$1"; shift
  if "$@" >/dev/null 2>&1; then pass "$desc"; else fail "$desc"; fi
}

start_container() {
  "$ENGINE" run -d --name "$NAME" -p 127.0.0.1::8080 \
    -v "${WORK}/data:/data" \
    -v "${WORK}/config:/config:ro" \
    -e BOTRIX_SESSION_SECRET="$SESSION_SECRET" \
    -e BOTRIX_CRYPTO_PASSPHRASE="$CRYPTO_PASSPHRASE" \
    -e BOTRIX_INSECURE_COOKIES=1 \
    "$IMAGE" >/dev/null
}

host_port() {
  "$ENGINE" port "$NAME" 8080/tcp | head -n1 | sed 's/.*://'
}

wait_healthy() { # wait_healthy PORT
  local port="$1" i
  for i in $(seq 1 60); do
    if [[ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${port}/health" || true)" == "200" ]]; then
      return 0
    fi
    sleep 0.5
  done
  return 1
}

api() { # api METHOD PATH [JSON]  -> prints HTTP status
  local method="$1" path="$2" body="${3:-}" token
  token="$(curl -s -b "$JAR" -c "$JAR" "http://127.0.0.1:${PORT}/api/v1/auth/csrf" \
    | sed -n 's/.*"token":"\([^"]*\)".*/\1/p' || true)"
  curl -s -o /dev/null -w '%{http_code}' -b "$JAR" -c "$JAR" -X "$method" \
    -H "x-csrf-token: ${token}" -H 'Content-Type: application/json' \
    ${body:+-d "$body"} "http://127.0.0.1:${PORT}${path}" || true
}

echo "Testing image: $IMAGE (engine: $ENGINE)"

echo "Startup"
start_container
PORT="$(host_port || true)"
if wait_healthy "$PORT"; then pass "starts and answers /health with 200"; else fail "starts and answers /health with 200"; fi
check "built-in healthcheck command succeeds" "$ENGINE" exec "$NAME" botrix healthcheck

echo "UI"
INDEX="$(curl -s "http://127.0.0.1:${PORT}/" || true)"
check "serves the UI shell" grep -q '<title>Botrix</title>' <<<"$INDEX"
WASM="$(grep -o 'botrix-frontend-[a-z0-9]*_bg\.wasm' <<<"$INDEX" | head -n1 || true)"
if [[ -n "$WASM" ]]; then
  TYPE="$(curl -s -o /dev/null -w '%{content_type}' "http://127.0.0.1:${PORT}/${WASM}" || true)"
  [[ "$TYPE" == "application/wasm" ]] && pass "serves the WASM bundle as application/wasm" || fail "serves the WASM bundle as application/wasm (got '$TYPE')"
else
  fail "index references a WASM bundle"
fi
[[ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/webhooks")" == "200" ]] \
  && pass "client-side routes fall back to the UI" || fail "client-side routes fall back to the UI"
[[ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/api/v1/does-not-exist")" == "404" ]] \
  && pass "unknown API paths are a real 404" || fail "unknown API paths are a real 404"

echo "Security"
[[ "$("$ENGINE" exec "$NAME" id -u)" == "1000" ]] && pass "runs as UID 1000" || fail "runs as UID 1000"
check "/data is writable" "$ENGINE" exec "$NAME" sh -c 'touch /data/.write-test && rm /data/.write-test'
if "$ENGINE" exec "$NAME" sh -c 'touch /config/.write-test' >/dev/null 2>&1; then
  fail "/config is read-only"
else
  pass "/config is read-only"
fi

echo "State survives a restart"
SETUP="$(api POST /api/v1/setup "{\"username\":\"alice\",\"password\":\"${ADMIN_PASSWORD}\"}")"
[[ "$SETUP" == "200" ]] && pass "first-run setup creates the admin" || fail "first-run setup creates the admin (got $SETUP)"
"$ENGINE" stop "$NAME" >/dev/null
"$ENGINE" start "$NAME" >/dev/null
PORT="$(host_port || true)"
if wait_healthy "$PORT"; then pass "restarts and becomes healthy again"; else fail "restarts and becomes healthy again"; fi
AGAIN="$(api POST /api/v1/setup "{\"username\":\"eve\",\"password\":\"${ADMIN_PASSWORD}\"}")"
[[ "$AGAIN" == "409" ]] && pass "admin persisted (second setup is rejected)" || fail "admin persisted (second setup got $AGAIN)"
LOGIN="$(api POST /api/v1/auth/login "{\"username\":\"alice\",\"password\":\"${ADMIN_PASSWORD}\"}")"
[[ "$LOGIN" == "200" ]] && pass "admin can log in after restart" || fail "admin can log in after restart (got $LOGIN)"

STATUS_OUT="$("$ENGINE" exec "$NAME" botrix status 2>&1 || true)"
if grep -q "Matrix status:" <<<"$STATUS_OUT"; then pass "botrix status reads the live database"; else fail "botrix status reads the live database (got: $STATUS_OUT)"; fi

echo "Logs"
LOGS="$("$ENGINE" logs "$NAME" 2>&1)"
for secret in "$SESSION_SECRET" "$CRYPTO_PASSPHRASE" "$ADMIN_PASSWORD"; do
  if grep -qF "$secret" <<<"$LOGS"; then fail "logs do not contain a secret"; else pass "logs do not contain a secret"; fi
done
if grep -qi 'database is locked' <<<"$LOGS"; then fail "no database lock errors at startup"; else pass "no database lock errors at startup"; fi

echo "Configuration validation"
if "$ENGINE" run --rm \
  -v "${WORK}/data:/data" -v "${WORK}/config:/config:ro" \
  -e BOTRIX_SESSION_SECRET="too-short" -e BOTRIX_CRYPTO_PASSPHRASE="$CRYPTO_PASSPHRASE" \
  "$IMAGE" >/dev/null 2>&1; then
  fail "refuses to start with a short session secret"
else
  pass "refuses to start with a short session secret"
fi

echo
echo "${PASSED} passed, ${FAILED} failed"
[[ "$FAILED" -eq 0 ]]
