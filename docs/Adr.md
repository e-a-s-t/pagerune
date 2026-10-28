# Architecture decision records

Status: Accepted means agreed product constraints or the scaffold's technical approach.
Proposed defaults require confirmation before being represented as settled product policy.

Phase 3 decision: use MiniJinja with external, bundled-default HTML and text notification
templates, loaded from a configurable directory for each send. Issue one 256-bit
unsubscribe token per notification and store only its SHA-256 hash. This keeps all
previously emailed links valid without persisting recoverable tokens. GET is confirmation
only; browser and RFC 8058 POST remove the matching subscription.

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
| ADR-025 | Accepted | Email notifications use a PostgreSQL transactional outbox | Change event and per-subscription deliveries commit atomically; replicas claim with `SKIP LOCKED`, expiring leases and fencing. SMTP occurs outside transactions. Delivery is at-least-once because SMTP acceptance may precede a crash before success is recorded. |
| ADR-026 | Accepted | Public site enrollment is explicitly enabled per site and defaults off | A stable UUID URL can be placed on the site owner’s website without allowing visitors to register arbitrary targets. Existing site registration stays on the private development API. |
| ADR-027 | Accepted | Public subscriptions use Turnstile behind a server-side provider adapter and email double opt-in | Avoid a custom CAPTCHA; provider validation is mandatory and replaceable. Only a POST with a valid single-use expiring email token activates a subscription. |
| ADR-028 | Accepted | Management uses short-lived email links exchanged by POST for PostgreSQL sessions | No password or identity provider is needed. Session hashes, email scope, expiry and CSRF state remain portable across replicas. |
| ADR-029 | Accepted | Verification and management email use a separate PostgreSQL outbox | Token IDs plus a shared deployment HMAC key allow replicas to regenerate links while storing only token hashes. SMTP is always outside transactions; delivery remains at-least-once. |
| ADR-030 | Accepted | Serialize notification delivery and unsubscribe with PostgreSQL advisory locks keyed by subscription | Unsubscribe waits for an already-running bounded SMTP attempt; after it commits, pending rows cascade and other replicas cannot begin delivery. The lock is session-scoped for the sender and transaction-scoped for unsubscribe, so no transaction spans SMTP and no extra infrastructure is required. |
| ADR-031 | Accepted | Local administration uses a declarative Keycloak realm behind OAuth2 Proxy | This supersedes ADR-021's loopback-only development API restriction for the local Compose stack: the API is enabled only on an unpublished backend port reachable through the proxy. Public subscriber routes, including `/manage`, retain their existing token authorization. Host Devbox mode remains local development; production must enforce ingress/network isolation and HTTPS independently. The proxy's group gate authenticates local administrators but does not implement tenant membership. |
| ADR-032 | Accepted | Website administration extends the existing global monitored-site model | Persist a display name and monitoring-enabled flag without introducing tenant tables or another lifecycle. Site URL changes preserve identity, subscriptions, and history while silently rebaselining. Deletion follows existing cascade semantics and serializes against notification sends with the existing subscription advisory locks. |

For a changed decision add context, options, rationale and consequences. Supersede an ADR
rather than silently rewriting an accepted decision. Keep the implementation roadmap
separate from architecture acceptance: accepted does not mean implemented.
