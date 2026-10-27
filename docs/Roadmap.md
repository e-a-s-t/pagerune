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

## Phase 3 — email notifications (implemented)
Create per-subscription PostgreSQL outbox rows atomically with detected changes. Deliver
through configurable SMTP with HTML/text alternatives, bounded worker concurrency,
expiring fenced leases, retries, and Mailpit development support. Delivery is at-least-once.

## Phase 4 — public self-service subscriptions (implemented)
Site registration returns a permanent public subscription URL. Owners explicitly enable
public subscriptions per site; enrollment defaults off and cannot register arbitrary URLs.
Visitors pass server-validated Turnstile and email double opt-in before activation. Add
PostgreSQL-backed verification/management email jobs, expiring hashed tokens, CSRF-protected
email management sessions, unsubscribe, rate limits and server-rendered templates. No OIDC
or subscriber accounts are introduced.

## Later — authenticated owner interface
Add tenant membership and OIDC SSO only when owner dashboard/API requirements are selected.
Keep the existing private development API gate until a secure replacement is designed.

## Phase 6 — daily digest
Normalize public static HTML (text + hrefs), implement durable daily cycles, cutoff/sealing,
deduplicated digest/outbox and retry handling. Checks may run at configured intervals, while
digest delivery remains a separate daily schedule. Direct source links only. Validate
simultaneous replicas and crash recovery against real PostgreSQL.

## Phase 7 — operational release
Validated environment configuration; application/migration container builds; Deployment,
Service, ingress/Gateway, probes, resources and policies. CNPG backup/restore runbook,
metrics/alerts, retention cleanup, contract checks and chosen-host CI using Devbox.
Select license and resolve product defaults before release.

## Later
New communication-channel adapters, rendering support only if required, optional richer
site ownership/onboarding. Do not add snapshots/public diffs or hourly polling by default.
