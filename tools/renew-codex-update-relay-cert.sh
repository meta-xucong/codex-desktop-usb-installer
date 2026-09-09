#!/bin/sh
set -eu

DOMAIN='update.404token.xyz'
EDGE_ROOT="${EDGE_ROOT:-/opt/codex-update-relay-edge}"
TLS_ROOT="${TLS_ROOT:-/srv/codex-update-relay/tls}"
ACME_HOME="${ACME_HOME:-$EDGE_ROOT/acme.sh}"
ACME_BIN="${ACME_BIN:-$ACME_HOME/acme.sh}"
COMPOSE_FILE="$EDGE_ROOT/docker-compose.yml"

if [ ! -x "$ACME_BIN" ]; then
  echo "acme.sh is not installed at $ACME_BIN" >&2
  exit 69
fi
if ! command -v docker >/dev/null 2>&1 || ! command -v docker-compose >/dev/null 2>&1; then
  echo 'docker and docker-compose are required' >&2
  exit 69
fi
if [ ! -f "$COMPOSE_FILE" ]; then
  echo "missing edge compose file: $COMPOSE_FILE" >&2
  exit 66
fi

edge_was_running=0
if docker inspect codex-update-relay-edge >/dev/null 2>&1; then
  if [ "$(docker inspect -f '{{.State.Running}}' codex-update-relay-edge)" = 'true' ]; then
    edge_was_running=1
    docker-compose -f "$COMPOSE_FILE" stop codex-update-edge
  fi
fi

restore_edge() {
  if [ "$edge_was_running" -eq 1 ]; then
    set +e
    docker-compose -f "$COMPOSE_FILE" up -d codex-update-edge
    docker exec codex-update-relay-edge nginx -t
    set -e
  fi
}
trap restore_edge EXIT

renew_status=0
"$ACME_BIN" --home "$ACME_HOME" --renew --domain "$DOMAIN" --ecc || renew_status=$?
if [ "$renew_status" -ne 0 ] && [ "$renew_status" -ne 2 ]; then
  echo "acme.sh renewal failed with exit code $renew_status" >&2
  exit "$renew_status"
fi
"$ACME_BIN" --home "$ACME_HOME" --install-cert \
  --domain "$DOMAIN" \
  --ecc \
  --key-file "$TLS_ROOT/live/$DOMAIN/privkey.pem" \
  --fullchain-file "$TLS_ROOT/live/$DOMAIN/fullchain.pem" \
  --reloadcmd 'true'

chown -R root:root "$TLS_ROOT"
chmod 0700 "$TLS_ROOT"
for tls_dir in "$TLS_ROOT/live" "$TLS_ROOT/archive"; do
  if [ -d "$tls_dir" ]; then
    find "$tls_dir" -type d -exec chmod 0700 {} +
    find "$tls_dir" -type f -exec chmod 0600 {} +
  fi
done

restore_edge
trap - EXIT
echo "certificate renewal check completed for $DOMAIN"
