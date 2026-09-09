#!/bin/sh
set -eu

DOMAIN='update.404token.xyz'
EDGE_ROOT="${EDGE_ROOT:-/opt/codex-update-relay-edge}"
TLS_ROOT="${TLS_ROOT:-/srv/codex-update-relay/tls}"
ACME_HOME="${ACME_HOME:-$EDGE_ROOT/acme.sh}"
ACME_BIN="${ACME_BIN:-$ACME_HOME/acme.sh}"
EMAIL="${1:-${ACME_EMAIL:-}}"
COMPOSE_FILE="$EDGE_ROOT/docker-compose.yml"

if [ ! -x "$ACME_BIN" ]; then
  echo "acme.sh is not installed at $ACME_BIN; install the pinned official acme.sh release first" >&2
  exit 69
fi

if ! command -v docker >/dev/null 2>&1 || ! command -v docker-compose >/dev/null 2>&1; then
  echo 'docker and docker-compose are required to safely coordinate the isolated edge container' >&2
  exit 69
fi

if [ ! -f "$COMPOSE_FILE" ]; then
  echo "missing edge compose file: $COMPOSE_FILE" >&2
  exit 66
fi

case "$EMAIL" in
  '') ;;
  *[!A-Za-z0-9._%+-@]*) echo 'invalid email address' >&2; exit 64 ;;
  *@*.*) ;;
  *) echo 'invalid email address' >&2; exit 64 ;;
esac

install -d -o root -g root -m 0700 "$TLS_ROOT"

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

"$ACME_BIN" --home "$ACME_HOME" --set-default-ca --server letsencrypt

if [ ! -f "$ACME_HOME/account.conf" ]; then
  if [ -n "$EMAIL" ]; then
    "$ACME_BIN" --home "$ACME_HOME" --register-account --server letsencrypt --accountemail "$EMAIL"
  else
    "$ACME_BIN" --home "$ACME_HOME" --register-account --server letsencrypt --register-unsafely-without-email
  fi
fi

"$ACME_BIN" --home "$ACME_HOME" --issue \
  --alpn \
  --server letsencrypt \
  --keylength ec-256 \
  --domain "$DOMAIN"

"$ACME_BIN" --home "$ACME_HOME" --install-cert \
  --domain "$DOMAIN" \
  --ecc \
  --key-file "$TLS_ROOT/live/$DOMAIN/privkey.pem" \
  --fullchain-file "$TLS_ROOT/live/$DOMAIN/fullchain.pem" \
  --reloadcmd "docker-compose -f $COMPOSE_FILE up -d codex-update-edge"

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
echo "certificate ready for $DOMAIN"
