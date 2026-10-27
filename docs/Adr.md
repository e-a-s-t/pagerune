# Architecture decision records

Status: Accepted means agreed product constraints or the scaffold's technical approach.
Proposed defaults require confirmation before being represented as settled product policy.

| ID | Status | Decision | Rationale / consequence |
|---|---|---|---|
| ADR-001 | Accepted | Rust, Axum and Tokio; one application | Small service and a single operational unit. |
| ADR-002 | Accepted | PostgreSQL only; CNPG in Kubernetes | Durable state and replica coordination; no SQLite migration path. |
| ADR-003 | Accepted | Multiple tenants/sites/watches | Customers own several pages; backend enforces tenant isolation. |
| ADR-004 | Accepted | Stateless interchangeable replicas | PostgreSQL stores sessions, baselines, jobs and delivery state. |
| ADR-005 | Accepted | Daily fetch before daily digest | Avoid intermediate snapshots and unnecessary polling. |
| ADR-006 | Accepted | Direct source links; no public diff/history | Keep notifications useful and storage small. |
| ADR-007 | Accepted | Digest contains all matching daily updates | One message per subscriber/channel; group by customer/site. |
| ADR-008 | Accepted | OIDC SSO and OAuth2; no local admin passwords | IdP provides identity; PageRune enforces membership and roles. |
| ADR-009 | Accepted | Verified subscriber email and scoped token links | No full subscriber accounts required. |
| ADR-010 | Accepted | Generated OpenAPI and bundled Swagger UI | Implementation and contract evolve together; UI assets need no CDN. |
| ADR-011 | Accepted | Minimal server-rendered frontend | No separate frontend toolchain/deployment. |
| ADR-012 | Accepted | PostgreSQL durable jobs and outbox | Avoid another queue dependency; leases/fencing recover crashes. |
| ADR-013 | Accepted | Configurable SMTP, email-only MVP | Add future delivery adapters without modifying page checking. |
| ADR-014 | Accepted | Devbox tools; direnv development variables | One documented toolchain/environment workflow; no app dotenv loader. |
| ADR-015 | Accepted | General and watch-preselected public links | Same form supports both entry points; admin offers copy buttons. |
| ADR-016 | Proposed | Global default 18:00 Europe/Stockholm | Consistent combined daily digest; make configurable. |
| ADR-017 | Proposed | Operator-invited tenant owners in v1 | Avoid building public commercial onboarding initially. |
| ADR-018 | Proposed | DNS TXT or same-origin well-known proof | Site control verification must not rely on email alone. |
| ADR-019 | Proposed | Shared SMTP sender with customer/site labels | Sender identity stays within the configured verified domain. |
| ADR-020 | Open | Retention limits and license | Owner must choose; no license or deletion periods fabricated. |
| ADR-021 | Accepted | Phase 2 implements core monitoring before authentication | Ship useful fetching, subscriptions and scheduling while preserving OIDC/OAuth2 as a required later phase. The unauthenticated business API is disabled by default and may only be enabled on loopback for development. |
| ADR-022 | Accepted | PostgreSQL row leases with expiry and fencing for checks | Replicas coordinate durably without another queue. Network I/O occurs outside transactions; stale generations cannot write results. |
| ADR-023 | Accepted | Configurable per-site intervals with 24-hour default and five-minute minimum | Supports core interval configuration while retaining the existing daily cadence as the default. |
| ADR-024 | Accepted | Change history stores stable identifiers and hash metadata only | Supports future permanent change URLs without storing page snapshots or publishing public diff pages. |

For a changed decision add context, options, rationale and consequences. Supersede an ADR
rather than silently rewriting an accepted decision. Keep the implementation roadmap
separate from architecture acceptance: accepted does not mean implemented.
