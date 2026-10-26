# Operations

## Target topology
Kubernetes Deployment with 2+ stateless PageRune replicas, Service and ingress/Gateway
TLS termination. No application PVC. CNPG-managed PostgreSQL has its own storage,
backup and recovery lifecycle. SMTP is configured independently. Example manifests in
deploy/ are starting points only: not a complete production installation.

A transaction-pool proxy needs careful review if advisory locks are ever used; this
architecture instead uses persisted job leases and row locking in short transactions.
Account for pool connections per replica and graceful termination when autoscaling.

## Secrets and settings
ConfigMap for non-sensitive settings; Secret/injection for DATABASE_URL, SMTP credentials,
OIDC secret. Do not use command-line secrets. Validate TLS to the database and mail server.
Configure public base URL and OIDC redirect using externally visible HTTPS addresses.

## Lifecycle
Migrations run once as an explicit deployment Job before rollout, with compatible
expand/contract schema changes. Do not have every web replica independently execute DDL.
No migrations are shipped yet. Add readiness schema-version checks when schema exists.
Liveness is independent of database availability. Drain requests and stop claiming work
on SIGTERM; expired leases recover unfinished jobs. Keep app builds separate from migrator.

## Observability and recovery
Current: JSON logs, /healthz and database-only /readyz. Planned: request IDs; metrics for
cycle lateness, fetch success/failure, outbox age, retries and terminal delivery errors.
No PII/high-cardinality recipient labels in metrics. Alert on missed daily cycles, not
only process health. Use NetworkPolicy and SMTP/fetch egress constraints where practical.
Configure CNPG backups and test restoration; determine recovery objectives and retention
with the operator. A replica count alone does not constitute a tested HA/backup strategy.

## Before first rollout
Generate/review tool lockfiles, run all checks, implement auth and business flows,
configure tenant setup/site verification, choose retention/license, provision image and
TLS/secrets, run tenant-isolation and multi-replica recovery tests, then test backup restore.
