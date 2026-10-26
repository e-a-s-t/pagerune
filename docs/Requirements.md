# PageRune requirements

## Product
PageRune is a fully self-hosted multi-tenant subscription service. Each customer
can manage multiple sites, each with multiple named page watches. A visitor selects
all updates for a site or individual watches, verifies their email and receives
one daily combined message if any selected pages changed.

## Accepted scope
- Rust application, PostgreSQL operated by CloudNativePG; no SQLite.
- Stateless, interchangeable app replicas, no local persistent volume or sticky sessions.
- Configurable SMTP host/port/auth/from/TLS via environment and Kubernetes secrets.
- Admin authentication through configurable OIDC SSO; OAuth2-protected business API.
- Every API documented in generated OpenAPI and available through bundled Swagger UI.
- Minimal owner UI: sites, watches, pause/edit, latest check/error and ready-to-copy links.
- Public subscription UI: choices, email verification, preference management and unsubscribe.
- No subscriber password account. Ownership of an email is proved with a token flow.
- Daily check immediately before daily digest; configurable schedule/timezone.
- All relevant daily updates in one message per subscriber/channel across sites/customers.
- No message if no selected page changed. First successful fetch only establishes baseline.
- Every update link goes to that watch's original canonical page URL.
- Verification, preference and unsubscribe links point to PageRune.
- Store current normalized content/hash; no historical snapshots or public diff links.
- Persist lightweight cycle change metadata and pending deliveries for reliable retries.
- Future channels are extension points; email is the only MVP channel.

## Selection and links
A site has a stable public slug, e.g. `/allmaningen/`.
Supported preselection: `?all=true`, `?watch=nyheter`, and repeated `watch` parameters.
No parameter means the visitor chooses; do not silently subscribe to everything.
“All” includes future watches for that site. Explicit selection includes only chosen watches.
A source-specific link selects that watch but the visitor may change the form before submit.
Unknown/cross-site watch slugs never subscribe the visitor to another site.
Public links do not contain personal tokens. Admin displays general/all/per-watch links
with copy buttons, generated from configured public base URL.
Changing existing preferences requires authorization from a purpose-bound email link;
an anonymous form must not overwrite an already verified person's subscriptions.

## Owner workflow
Owner signs in with SSO, obtains tenant membership, registers a site, verifies control,
creates named watches and copies subscription links to the source website. Scope for
v1 is public static HTML; allow a CSS selector and ignore script/style/irrelevant chrome.
Normalize text and link targets so changed document URLs are detectable. A linked PDF's
contents do not get fetched automatically. Fail a check on empty/missing selector rather
than treating it as a mass deletion.

## Daily behavior
Each source is fetched once per scheduled cycle, independent of subscriber count.
Compare against the previous successful baseline. Timeout/HTTP failures preserve it.
Persist a lightweight change and baseline atomically. A cycle has a cutoff: failed
sources do not block successful sources. Assemble digests only after the cycle is sealed.
Each subscriber receives each matching watch at most once in the digest. Changes detected
before subscription activation are not backfilled. Recheck active selections before send
so unsubscribe is honored. Pending messages survive restarts.

## Out of scope for MVP
Payments, Meta channels, SMS, JavaScript browser rendering, authenticated source pages,
PDF content scraping, historical page storage, public change views and analytics tracking.

## Choices still to confirm
Customer onboarding (operator invitation vs public self-registration); site verification
method; default schedule; delivery/change metadata retention; default shared sender and
branding. Proposed defaults are documented as proposals in Adr.md, not product decisions.
