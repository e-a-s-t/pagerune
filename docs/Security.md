# Security design

These controls describe the current implementation and remaining requirements.

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
- Public enrollment: monitored sites default to `public_subscriptions_enabled=false`; public
  forms can only target existing enabled site IDs. The owner registration API remains
  loopback-only and disabled by default.
- Human verification: Turnstile is validated server-side with bounded response size and
  timeout, expected action and hostname. Fail closed on errors. The test bypass is only
  accepted when the development API is enabled on loopback.
- Double opt-in: subscriptions start pending. Email tokens are purpose-bound, expire after
  24 hours, are single-use, and are stored only as SHA-256 hashes. GET never consumes a
  token or activates a subscription; POST also requires a one-use form nonce.
- Management: requests return generic responses. Email links expire after 15 minutes and
  are exchanged only through POST. Sessions last 30 minutes; PostgreSQL stores session
  hashes, verified-email scope, and CSRF state. Management unsubscribe is POST-only.
- Abuse controls: form bodies are capped at 16 KiB. PostgreSQL hourly buckets are shared
  across replicas and limit requests per remote IP and normalized email. Do not trust
  forwarded IP headers unless a trusted-proxy configuration is added.

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
