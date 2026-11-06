# Operations

## Target topology
Kubernetes Deployment with 2+ stateless PageRune replicas, Service and ingress/Gateway
TLS termination. No application PVC. CNPG-managed PostgreSQL has its own storage,
backup and recovery lifecycle. SMTP is configured independently. Example manifests in
deploy/ are starting points only: not a complete production installation.

## Application image and release

Create the Docker Hub repository **`erjoda/pagerune`** before enabling publication. The
default can be changed without editing the workflow through the optional GitHub repository
variable `DOCKERHUB_IMAGE`. Configure these GitHub repository secrets:

- `DOCKERHUB_USERNAME`: Docker Hub account or organization publisher name.
- `DOCKERHUB_TOKEN`: Docker Hub access token with permission to push that repository.

The `Docker image` workflow builds every pull request without authenticating or pushing.
A push to `main` publishes `erjoda/pagerune:latest` and
`erjoda/pagerune:sha-<short-commit-SHA>`. A Git tag matching `v*` uses semantic Docker
tags: `v1.2.3` publishes `1.2.3`, `1.2`, and `1`. Exact `X.Y.Z` tags are immutable: the
workflow fails before publishing if that tag already exists. The `X.Y` and `X` aliases are
intentionally updated by later compatible releases. Tag builds do not publish `latest`.

To release, complete the checks, merge the intended commit to `main`, then create and push
an annotated semantic tag:

```sh
git tag -a v1.2.3 -m 'Release v1.2.3'
git push origin v1.2.3
```

The production image contains a release PageRune binary, runtime templates/static assets,
SQLx migrations, CA certificates, and the small HTTP client used by the existing Compose
health check. It runs as the unprivileged `pagerune` user and contains no source tree,
Cargo cache, Devbox, PostgreSQL, Mailpit, Keycloak, or OAuth2 Proxy. OCI labels record the
source repository, revision, version, build timestamp, title, and description.

Use `pagerune migrate` from this image as an explicit pre-rollout migration action. Do not
run migrations automatically from every application replica. Runtime configuration remains
environment based; inject database, SMTP, Turnstile, and token-key secrets outside the
image and set `PAGERUNE_BIND=0.0.0.0:8080` behind a private authenticated HTTPS ingress.

## Email templates and unsubscribe
Mount customized `change.html` and `change.txt` files in the directory selected by
`PAGERUNE_EMAIL_TEMPLATE_DIR` (default `templates/email`). Files are read per notification,
so changes apply without a restart. Missing files use bundled defaults; unreadable or
invalid files fail delivery as a permanent template error. A message already in SMTP
delivery when an unsubscribe completes may still be accepted and cannot be recalled. Set
`PAGERUNE_PUBLIC_BASE_URL` to the externally reachable HTTPS origin. Token hashes cascade
with subscription deletion.

## Public subscriptions
Enable enrollment per site only after registration, by setting
`public_subscriptions_enabled=true` through private owner API access. Keep the development
API disabled on public ingress. Configure Turnstile site/secret keys and a shared 64-hex
`PAGERUNE_TOKEN_KEY` in the deployment secret; every replica must use the same key. Never
enable `PAGERUNE_TURNSTILE_TEST_MODE` outside a loopback development instance or the local
Compose trusted-proxy profile. Verification and management outbox jobs use PostgreSQL
leases and token hashes. Monitor their pending,
delivered and failed counts without logging recipient addresses or token URLs. A shared
cleanup task removes expired tokens, sessions, form nonces and rate buckets.

A transaction-pool proxy needs careful review if advisory locks are ever used; this
architecture instead uses persisted job leases and row locking in short transactions.
Account for pool connections per replica and graceful termination when autoscaling.

## Secrets and settings
ConfigMap for non-sensitive settings; Secret/injection for DATABASE_URL, SMTP credentials,
Turnstile secret and token key. Do not use command-line secrets. Validate TLS to the database
and mail server. Configure public base URL using the externally visible HTTPS origin.

## Lifecycle
Migrations run once as an explicit deployment Job before rollout, with compatible
expand/contract schema changes. Do not have every web replica independently execute DDL.
Migrations are embedded and applied explicitly through `pagerune migrate` from the
production image (or the Devbox migrate command for local development); add readiness
schema-version checks as schema evolution requires.
Liveness is independent of database availability. Drain requests and stop claiming work
on SIGTERM; expired leases recover unfinished jobs. Keep app builds separate from migrator.

## Observability and recovery
Current: JSON logs, `/healthz`, database-only `/readyz`, and PostgreSQL-backed check and
notification workers on each replica. The notification worker uses bounded per-replica
concurrency, 180-second leases, fencing generations and `SKIP LOCKED`. It commits claims
before SMTP; expired leases recover work after a crash. Retry delay is exponential with
jitter, capped at six hours; eight attempts is the default before terminal failure. SMTP
health does not affect `/readyz`. Inspect `notification_outbox` for pending/sending/failed
rows and `last_error`; correct SMTP configuration or recipient problems before manually
requeueing terminal rows. Do not log message bodies or SMTP credentials. The monitoring
worker uses a 180-second lease and fencing generation; HTTP timeout is capped at 120 seconds.
Planned: metrics
for check lateness, fetch success/failure, outbox age, retries and terminal delivery errors.
No PII/high-cardinality recipient labels in metrics. Alert on missed daily cycles, not
only process health. Use NetworkPolicy and SMTP/fetch egress constraints where practical.
Configure CNPG backups and test restoration; determine recovery objectives and retention
with the operator. A replica count alone does not constitute a tested HA/backup strategy.

## Before first rollout
Generate/review tool lockfiles, run all checks, configure tenant setup/site verification,
choose retention/license, provision image and
TLS/secrets, run tenant-isolation and multi-replica recovery tests, then test backup restore.
