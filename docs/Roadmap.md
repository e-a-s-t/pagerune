# Implementation roadmap

## Phase 0 — scaffold
Rust HTTP skeleton, generated operational OpenAPI, bundled Swagger, PostgreSQL readiness,
Devbox/direnv definitions, local service examples, requirements, ADRs and test matrix.

## Phase 1 — application foundation (implemented)
Validated PostgreSQL configuration, SQLx migration tracking, Axum health/readiness,
structured logs, graceful shutdown, OpenAPI/Swagger, and PostgreSQL integration tests.
The initial migration is an empty baseline; no product tables are created.

## Phase 2 — core monitoring (implemented)
Persist normalized monitored URLs and subscriptions, asynchronously fetch HTTP(S) content,
track SHA-256 baselines and metadata-only changes, and coordinate due checks with expiring
PostgreSQL leases and fencing. Expose paginated site, subscription and change APIs with
generated OpenAPI. Until authentication exists, the business API is disabled by default;
development access requires explicit opt-in and a loopback bind. Polling defaults to 24
hours and is configurable per site (minimum five minutes).

## Phase 3 — persistence and identity
Add tenants, memberships, contacts, verification tokens and site ownership proof. Implement
OIDC SSO, portable sessions, OAuth2 access-token validation and membership enforcement.
Move site/watch APIs behind authenticated tenant boundaries; remove the development-only
API gate only after the authenticated replacement is complete and tested.

## Phase 4 — minimal frontend and subscription workflow
Server-rendered admin with watch status/copy links. Public general/preselected form,
double opt-in, safe token management, unsubscribe, CSRF and rate limits. Configurable SMTP
adapter and local mail integration tests. No production auth bypass or real mail in tests.

## Phase 5 — daily digest
Normalize public static HTML (text + hrefs), implement durable daily cycles, cutoff/sealing,
deduplicated digest/outbox and retry handling. Checks may run at configured intervals, while
digest delivery remains a separate daily schedule. Direct source links only. Validate
simultaneous replicas and crash recovery against real PostgreSQL.

## Phase 6 — operational release
Validated environment configuration; application/migration container builds; Deployment,
Service, ingress/Gateway, probes, resources and policies. CNPG backup/restore runbook,
metrics/alerts, retention cleanup, contract checks and chosen-host CI using Devbox.
Select license and resolve product defaults before release.

## Later
New communication-channel adapters, rendering support only if required, optional richer
site ownership/onboarding. Do not add snapshots/public diffs or hourly polling by default.
