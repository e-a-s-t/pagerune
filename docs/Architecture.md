# Architecture

## Components
One Rust application serves server-rendered UI, JSON APIs, OpenAPI/Swagger, scheduler
and workers. Multiple copies run as a Deployment. PostgreSQL is the durable coordination
and storage layer, managed separately by CloudNativePG. SMTP is an external protocol
endpoint operated by the deployer; no SaaS is required.

## Data model
- Monitored site: UUID, original URL, normalized unique URL, configurable interval,
  current hash/ETag/Last-Modified, last check/success timestamps and current status.
- Subscription: monitored site + opaque subscriber identity. The identity is a development
  association only; it is not proof of ownership or authorization.
- Site change: stable UUID, detected time and previous/new SHA-256 hashes. No body snapshot
  or public change page is stored or exposed in this phase.
- Lease: owner UUID, expiry and monotonically increasing fencing generation stored on the
  monitored site row. PostgreSQL is the scheduler and coordination store.

The following tenant-oriented entities are planned for the authenticated phase:
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

## Scheduling and concurrency
1. Every replica polls PostgreSQL for due sites. A short transaction claims one due row
   with `FOR UPDATE SKIP LOCKED`, sets a 180-second lease and increments its generation.
2. The transaction commits before DNS resolution and HTTP fetching. HTTP timeout defaults
   to 15 seconds and body size defaults to 2 MiB; both are configurable.
3. Results apply only when the lease owner and generation still match and the lease has not
   expired. Baseline/hash, change metadata, status, timestamps and next due time update in
   one transaction. A crash leaves an expiring lease that another replica can reclaim.

Default interval is 24 hours; site intervals range from five minutes to 30 days. A failed
check retries after at most five minutes. On a changed hash, a site-change row and one
notification-outbox row per active subscription are committed in the same transaction.
Initial baselines and unchanged checks write neither.

## Notification outbox (implemented)
The outbox is PostgreSQL-backed and shared by all replicas. A unique `(change_id,
subscription_id)` constraint prevents duplicate rows. Workers claim due rows with
`FOR UPDATE SKIP LOCKED`, increment a fencing generation and set a 180-second lease. The
claim commits before SMTP. Success, retry, and terminal failure updates require the same
owner and generation and an unexpired lease. Expired `sending` rows are recoverable.
Concurrency is bounded per replica. Retry delay is exponential with jitter capped at six
hours; after the configured attempt limit the row is terminal.

SMTP delivery is at-least-once. If SMTP accepts a message and the process crashes before
PostgreSQL records success, a recovered row can send again. A stable Message-ID aids
correlation but does not provide exactly-once semantics. Deleting a subscription cascades
to queued notifications; an SMTP-accepted message cannot be recalled. Messages link only
to the monitored URL until PageRune frontend links exist.

## Daily cycle and digest (planned)
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
