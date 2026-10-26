# Implementation roadmap

## Phase 0 — scaffold (this delivery)
Rust HTTP skeleton, generated operational OpenAPI, bundled Swagger, PostgreSQL readiness,
Devbox/direnv definitions, local service examples, requirements, ADRs and test matrix.
Rust compilation is pending because the scaffolding environment lacks Devbox/Rust.

## Phase 1 — persistence and identity
Create migrations for tenant/site/watch/contact/subscription/token models. Implement OIDC
SSO, portable sessions, API token validation and membership enforcement. Add site verification.
Implement owner site/watch APIs with OpenAPI and tenant-isolation tests before exposing UI.

## Phase 2 — minimal frontend and subscription workflow
Server-rendered admin with watch status/copy links. Public general/preselected form,
double opt-in, safe token management, unsubscribe, CSRF and rate limits. Configurable SMTP
adapter and local mail integration tests. No production auth bypass or real mail in tests.

## Phase 3 — daily checks and digest
Normalization (text + hrefs), safe HTTP fetch, first baseline, durable daily cycles,
leases/fencing, cutoff/sealing, deduplicated digest/outbox and retry handling. Direct source
links only. Validate simultaneous replicas and crash recovery against real PostgreSQL.

## Phase 4 — operational release
Validated environment configuration; application/migration container builds; Deployment,
Service, ingress/Gateway, probes, resources and policies. CNPG backup/restore runbook,
metrics/alerts, retention cleanup, contract checks and chosen-host CI using Devbox.
Select license and resolve product defaults before release.

## Later
New communication-channel adapters, rendering support only if required, optional richer
site ownership/onboarding. Do not add snapshots/public diffs or hourly polling by default.
