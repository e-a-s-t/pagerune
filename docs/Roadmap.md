# Implementation roadmap

## Phase 0 — scaffold
Rust HTTP skeleton, generated operational OpenAPI, bundled Swagger, PostgreSQL readiness,
Devbox/direnv definitions, local service examples, requirements, ADRs and test matrix.

## Phase 1 — application foundation (implemented)
Validated PostgreSQL configuration, SQLx migration tracking, Axum health/readiness,
structured logs, graceful shutdown, OpenAPI/Swagger, and PostgreSQL integration tests.
The initial migration is an empty baseline; no product tables are created.

## Phase 2 — persistence and identity
Create migrations for tenant/site/watch/contact/subscription/token models. Implement OIDC
SSO, portable sessions, API token validation and membership enforcement. Add site verification.
Implement owner site/watch APIs with OpenAPI and tenant-isolation tests before exposing UI.

## Phase 3 — minimal frontend and subscription workflow
Server-rendered admin with watch status/copy links. Public general/preselected form,
double opt-in, safe token management, unsubscribe, CSRF and rate limits. Configurable SMTP
adapter and local mail integration tests. No production auth bypass or real mail in tests.

## Phase 4 — daily checks and digest
Normalization (text + hrefs), safe HTTP fetch, first baseline, durable daily cycles,
leases/fencing, cutoff/sealing, deduplicated digest/outbox and retry handling. Direct source
links only. Validate simultaneous replicas and crash recovery against real PostgreSQL.

## Phase 5 — operational release
Validated environment configuration; application/migration container builds; Deployment,
Service, ingress/Gateway, probes, resources and policies. CNPG backup/restore runbook,
metrics/alerts, retention cleanup, contract checks and chosen-host CI using Devbox.
Select license and resolve product defaults before release.

## Later
New communication-channel adapters, rendering support only if required, optional richer
site ownership/onboarding. Do not add snapshots/public diffs or hourly polling by default.
