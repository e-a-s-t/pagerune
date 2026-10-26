# Architecture

## Components
One Rust application serves server-rendered UI, JSON APIs, OpenAPI/Swagger, scheduler
and workers. Multiple copies run as a Deployment. PostgreSQL is the durable coordination
and storage layer, managed separately by CloudNativePG. SMTP is an external protocol
endpoint operated by the deployer; no SaaS is required.

## Data model (planned; product schema not yet implemented)
- Tenant and membership: customer boundary; membership maps trusted OIDC issuer + subject
  to tenant + role. One identity can have multiple explicit memberships.
- Site: tenant ID, public slug, verified domain and ownership proof state.
- Watch: tenant/site IDs, stable site-local slug, label, original URL, selector, enabled flag,
  current normalized content/hash, last successful check and error status.
- Subscriber/contact: tenant-scoped email verification and delivery contact. Deduplicate
  destination across tenants internally for one global digest; never expose cross-tenant
  subscription lists to owners. Normalize email conservatively; do not strip provider tags.
- Subscription: contact + site, mode `all` or explicit watch selection, activation time.
- Token/session: purpose, contact/identity binding, digest, expiry and consumed/revoked state.
- Daily cycle: unique scheduled date/timezone, lease/cutoff, sealed state.
- Check job: unique cycle/watch, attempts, lease owner, fencing generation, next attempt.
- Cycle change: unique cycle/watch, detection timestamp, label + original URL. No page snapshot.
- Digest/outbox: unique cycle/destination/channel, immutable payload, state, attempts/next attempt.
- Delivery: SMTP outcome metadata and bounded retention; no open/click tracking.

Use composite foreign keys including tenant_id where objects reference tenant-owned
objects; enforce site/watch consistency in schema and service queries. Global digest
assembly is an internal privileged workflow, not a tenant API. Never expose foreign
customer subscriber details. Subscription verification and unsubscribe scopes are explicit.

## Daily cycle and concurrency
1. Any replica may create the scheduled cycle; a unique key prevents duplicates.
2. Insert one check job for each eligible watch. Workers claim due jobs in a short
   transaction using `FOR UPDATE SKIP LOCKED`, then commit before network I/O.
3. Claim has expiry + generation. Renew long jobs; apply results only if the worker
   still owns that generation. An expired worker cannot overwrite newer results.
4. A successful first check establishes baseline silently. On a changed hash, write
   current baseline + lightweight cycle change + job completion atomically.
5. At completion or cutoff, seal the cycle transactionally. Late checks cannot add
   changes to a sealed cycle. Preserve/retry failed sources in a later cycle.
6. Build immutable per-destination digests after sealing, filtered by verified active
   subscriptions and activation time. Do not backfill pre-activation changes. Insert
   outbox rows with unique keys so concurrent assemblers cannot duplicate work.
7. Delivery workers lease outbox jobs; honor unsubscribe/pause before send and persist result.

Persist progress so crashed replicas can resume. No transaction spans HTTP or SMTP.
Use database time for leases, UTC for instants and an IANA timezone for daily schedules.
Define a single logical daily cycle across DST and enforce uniqueness on the local date.
Do not use a per-replica in-memory timer as the source of truth for due work.
SMTP has an ambiguous success window: accepted mail followed by a database write failure
may be resent. Stable Message-ID helps tracing but does not guarantee deduplication.

## Frontend and APIs
Minimal server-rendered templates, progressive enhancement only for small interactions
such as copy-link buttons. Admin uses OIDC sessions persisted in PostgreSQL. CSRF-protected
HTML forms delegate to the same application services as documented JSON APIs. No React,
Node build pipeline, or separate frontend deployment is needed.

## Storage and retention
Only one current normalized body/hash per watch. Cycle metadata and immutable digest
payloads remain until their defined retention period; redact/purge addresses and token
material on deletion. No original raw HTML archive. Version normalization rules explicitly:
a selector/normalization change triggers a silent rebaseline rather than a false update.
