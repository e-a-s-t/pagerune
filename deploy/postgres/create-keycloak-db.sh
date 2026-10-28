#!/bin/sh
set -eu

# Idempotent so the existing postgres-data volume can be reused safely.
psql -v ON_ERROR_STOP=1 -v keycloak_password="$KEYCLOAK_DB_PASSWORD" <<'SQL'
SELECT format('CREATE ROLE keycloak LOGIN PASSWORD %L', :'keycloak_password')
WHERE NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'keycloak')
\gexec

SELECT format('ALTER ROLE keycloak PASSWORD %L', :'keycloak_password')
\gexec

SELECT 'CREATE DATABASE pagerune_keycloak OWNER keycloak'
WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = 'pagerune_keycloak')
\gexec
SQL
