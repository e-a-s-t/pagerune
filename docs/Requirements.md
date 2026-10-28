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
- No OIDC or subscriber accounts in the public self-service subscription phase. Administrative login is a separate local Keycloak/OAuth2 Proxy concern; subscriber access remains email-token based.
- Every API documented in generated OpenAPI and available through bundled Swagger UI.
- Minimal owner UI: sites, watches, pause/edit, latest check/error and ready-to-copy links.
- Public site-specific subscription pages are enabled explicitly per monitored site (default off); visitors cannot register arbitrary sites.
- Public subscriptions require server-validated human verification and email double opt-in. Only verified subscriptions receive notifications.
- Subscribers can request temporary email-based management sessions, unsubscribe individually, and use the existing token unsubscribe mechanism.
- No subscriber password account. Ownership of an email is proved with a token flow.
- Daily check immediately before daily digest; configurable schedule/timezone.
- All relevant daily updates in one message per subscriber/channel across sites/customers.
- No message if no selected page changed. First successful fetch only establishes baseline.
- Every update link goes to that watch's original canonical page URL.
- Verification, preference and unsubscribe links point to PageRune.
- Store current normalized content/hash; no historical snapshots or public diff links.
- Persist lightweight cycle change metadata and pending deliveries for reliable retries.
- Future channels are extension points; email is the only MVP channel.

## Phase 2 core monitoring behavior
- A monitored URL is normalized and stored once; subscriptions to equivalent URLs share it.
- Checks use per-site intervals (24-hour default, five-minute minimum) and PostgreSQL
  leases/fencing across replicas. The daily digest schedule remains a separate later feature.
- Current content hashes and lightweight change records are retained; page snapshots and
  public change pages remain out of scope. Change records have stable UUIDs for future URLs.
- Before the local authentication phase, the business API is disabled by default and may
  be enabled on a host loopback bind for development. With the local proxy stack, it is
  enabled only on the unpublished PageRune container port behind OAuth2 Proxy. Subscriber
  identities are opaque labels, not authentication credentials.

## Phase 5 requirements adjustment
The accepted Phase 2 development API and Phase 4 Turnstile test-mode requirements were
loopback-only. This phase narrowly extends those development settings to the local Compose
topology: they may run on the private backend listener when trusted proxy mode is explicitly
enabled. Direct host development remains loopback-only, and neither setting is a production
security boundary.

## Phase 5 local management authentication
- Keycloak provides OIDC identity for local administrative access; OAuth2 Proxy enforces
  the management group at the HTTP boundary. OIDC authentication logic is not added to Rust.
- The declarative local `pagerune` realm includes the `pagerune-management` client and
  group, plus the development-only `admin` / `admin` account. The import is automatic.
- The initial owner UI is an authenticated landing page. Phase 6 replaces it with the
  website management interface; business APIs, OpenAPI and Swagger remain proxy-protected.
- Public landing, subscription, verification, email-token subscriber management and
  unsubscribe routes remain unauthenticated at the proxy. Subscriber `/manage` continues
  to require its existing email-token flow.
- Compose publishes OAuth2 Proxy only for PageRune traffic; the backend port is not
  published. Host-run Devbox mode remains available for local development and connects
  through the Docker bridge gateway. Neither local HTTP nor the seeded credentials are
  production configuration.
- Authentication through a management group does not implement tenant membership or
  per-resource authorization. Those remain application requirements for the authenticated
  owner product phase.

## Phase 6 website management UI
- Authenticated administrators can list, register, edit, pause/resume, and remove monitored
  websites using the existing Keycloak/OAuth2 Proxy boundary and server-rendered Rust UI.
- Each monitored website has a required display name and a `monitoring_enabled` flag. Existing
  websites remain enabled after migration; newly registered websites use the existing
  24-hour interval and public subscriptions remain disabled by default.
- Administrators can update the existing check interval (300 seconds to 30 days) along with
  name, URL, monitoring enabled state, and the independent public subscription setting.
- Website registration includes an “Allow public subscriptions” option, disabled by default;
  administrators can change it when editing a website without changing monitoring state.
- The management list shows monitoring status, last check, latest detected change, and the
  count of active subscribers. Site listing is paginated.
- The management list always displays each site's permanent absolute `/subscribe/{site_id}`
  URL from the configured public base URL, with View and Copy link actions and an explicit
  Enabled or Disabled subscription state. Disabled sites still reject new public subscriptions.
- Editing a URL preserves the site identity, subscriptions, and lightweight change history.
  A URL change clears fetch validators, current check timestamps, and the baseline; its first
  successful check establishes a new silent baseline.
- Disabling monitoring preserves subscriptions and history but prevents scheduler claims.
  Re-enabling makes a site due through PostgreSQL without restarting the application.
- Removing a website permanently cascades subscriptions, their tokens and pending
  notifications, plus monitoring history. Pending verification email jobs are removed;
  management email jobs unrelated to that site remain. An in-flight bounded send completes
  before deletion; SMTP-accepted mail cannot be recalled.
- Management browser writes require one-use PostgreSQL-backed CSRF nonces. Public
  subscription, verification, subscriber management, and unsubscribe flows remain intact.

## Phase 3 email notifications
- A detected change and one outbox notification per active subscription are committed atomically; initial baselines and unchanged checks are silent.
- Replicas claim deliveries using PostgreSQL `SKIP LOCKED`, expiring leases and fencing generations. SMTP work occurs outside transactions.
- Delivery is at-least-once: SMTP acceptance followed by a crash before recording success can produce a duplicate. Exactly-once delivery is not guaranteed.
- Subscription deletion cascades to queued outbox rows, suppressing pending notifications. A message already accepted by SMTP cannot be recalled.
- Delivery checks subscription status under PostgreSQL coordination immediately before SMTP. Unsubscribe waits for any already-started bounded send to finish; after it succeeds no queued or concurrent worker may begin another delivery.
- Email contains PageRune branding, source URL, detection time and stable change ID; it includes no snapshots or raw page content.
- Notification email has customizable bundled HTML and plain-text templates. Each message has a cryptographically random unsubscribe token; PostgreSQL stores only SHA-256 token hashes. Tokens remain valid until subscription deletion. GET displays confirmation while POST removes only the associated subscription; RFC 8058 one-click POST is supported.

## Phase 4 public subscriptions
- Site registration returns a permanent `/subscribe/{site_id}` URL. `public_subscriptions_enabled` defaults to false and is controlled through the private development API. Disabled sites reject public enrollment.
- Visitors submit only an email and a server-validated human-verification response; the site URL and interval are fixed by the registered monitored site. The interval defaults to 24 hours and remains subject to the five-minute minimum at site registration.
- Cloudflare Turnstile is the first replaceable human-verification adapter. Development test mode is permitted only for loopback development or the explicitly trusted local Compose proxy; it is not a CAPTCHA provider or production option.
- Pending subscriptions are stored in PostgreSQL and deduplicated by normalized email and site. A generic response is returned whether the address is new, pending, or already active.
- Verification tokens are high-entropy purpose-scoped capabilities, stored by hash and expiring after 24 hours. GET is read-only; a one-use CSRF nonce is required for POST activation.
- Verification and management email requests use a PostgreSQL outbox, leased across replicas. SMTP work occurs after the transaction commits. Outbox rows store token IDs; purpose-scoped tokens are deterministically regenerated with a deployment secret, while PostgreSQL stores only token hashes.
- Management links expire after 15 minutes and exchange via POST into a PostgreSQL-backed 30-minute HttpOnly session. Management views and unsubscribe actions are scoped to the verified email identity and protected by CSRF.
- PostgreSQL is reset before the phase is deployed in development. The migration adds schema normally; no legacy subscriber reverification/backfill logic is required.

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
