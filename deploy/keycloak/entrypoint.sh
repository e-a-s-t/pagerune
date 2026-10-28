#!/bin/sh
set -eu

: "${KEYCLOAK_CLIENT_SECRET:?KEYCLOAK_CLIENT_SECRET is required}"
case "$KEYCLOAK_CLIENT_SECRET" in
  *[!A-Za-z0-9_-]*) echo "KEYCLOAK_CLIENT_SECRET must use URL-safe characters" >&2; exit 1 ;;
esac

mkdir -p /opt/keycloak/data/import
sed -e "s/__KEYCLOAK_CLIENT_SECRET__/${KEYCLOAK_CLIENT_SECRET}/g" \
  -e "s/__PAGERUNE_PROXY_PORT__/${PAGERUNE_PROXY_PORT:-8088}/g" \
  /realm-template.json > /opt/keycloak/data/import/pagerune-realm.json

# Keep this development realm declarative even when the PostgreSQL volume already exists.
# Import runs before the server starts, so the database is not concurrently modified.
/opt/keycloak/bin/kc.sh import --dir /opt/keycloak/data/import --override true
exec /opt/keycloak/bin/kc.sh start-dev
