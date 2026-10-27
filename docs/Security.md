# Security design

These are required implementation controls, not features delivered by the scaffold.

## Phase 2 development API
The site/subscription business API is disabled by default. Enabling it requires
`PAGERUNE_DEV_API_ENABLED=true` and a loopback `PAGERUNE_BIND`; configuration validation
rejects any other bind. This is a temporary local-development safeguard, not authentication
or tenant isolation. Do not expose the process through a proxy, tunnel, or public network.
Subscriber identities are caller-supplied opaque labels and do not establish ownership.

- SSO: OIDC code flow + PKCE, state and nonce; secure HttpOnly SameSite cookies.
  Sessions in PostgreSQL. CSRF protections on browser mutations including logout.
- API: OAuth2 access tokens, strict issuer/audience/signature/expiry/algorithm validation,
  controlled JWKS refresh and explicit scopes. Never accept arbitrary issuer URLs from users.
- Tenancy: resolve trusted issuer+subject to membership; constrain every query and foreign
  key to the authorized tenant/site. Operator privileges must be explicit and audited.
- Subscribers: secure random purpose-bound tokens, hashes in DB, verification expiry and
  single-use. Preferences/unsubscribe scoped and revocable; no anonymous preference overwrite.
  Never mutate on GET because email link scanners follow links automatically.
- SSRF: public HTTP(S) only, reject userinfo/nonstandard schemes; verify site control,
  resolve and validate all A/AAAA results, validate the actual connection and every redirect.
  Block loopback, private, link-local, multicast and cloud metadata networks; egress policy
  is defense in depth. Disable environment proxy inheritance unless explicitly approved.
- Fetching: bounded concurrency/timeouts/redirects/decompressed sizes, status validation,
  no execution of remote scripts. Disallow empty selector results from replacing baseline.
- Output: template autoescaping, safe URL schemes, no raw remote HTML injection; CSP,
  no-referrer on token pages; no third-party resources there. Rate-limit public endpoints.
- Privacy: no click/open tracking, no addresses or token URLs in logs. Owner dashboards
  must not expose other customers' subscriptions. Define deletion/retention before launch.
- Dependencies: review and lock, no runtime Swagger CDN; no local password auth bypass.
- SMTP: explicit TLS mode, credential redaction; do not downgrade after TLS errors.

Email messages contain only PageRune branding, the monitored source URL, detection time
and stable change UUID. They never include fetched page content or snapshots. SMTP
credentials are read from environment injection and never included in tracing fields.
Delivery remains at-least-once across the SMTP-acceptance/database-commit ambiguity.

Owner must select retention periods, privacy notice and license before public launch.

Unsubscribe tokens are random, per-message bearer capabilities. Only SHA-256 digests are
stored; raw tokens and token URLs must not be logged. The public unsubscribe route returns
generic responses and scopes deletion through the token digest. GET is read-only. POST
accepts RFC 8058 form data without cookies or CSRF state. Email templates receive trusted
metadata only; monitored page HTML is never rendered, and HTML values are autoescaped.
