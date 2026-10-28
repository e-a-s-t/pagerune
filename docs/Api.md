# API and UI contract

## Current implementation
Generated OpenAPI: `GET /api/openapi.json`. Bundled Swagger UI: `GET /swagger-ui/`.
Operational routes: `GET /healthz` and `GET /readyz` (200 or 503 based on PostgreSQL).
Business routes are under `/api/v1`. In direct host development they are disabled by
default; `PAGERUNE_DEV_API_ENABLED=true` is limited to a loopback bind. The local Compose
stack enables them on an unpublished backend listener and exposes them only through
OAuth2 Proxy. `PAGERUNE_TRUSTED_PROXY_MODE` is a deployment declaration, not authentication;
never enable it unless network policy prevents direct access to the backend. Error responses use
`{code, message, request_id}` without provider details; `X-Request-ID` is returned.
Collections use `page` (default 1) and `limit` (default 50, maximum 100).

| Method | Route | Purpose |
|---|---|---|
| GET, POST | /api/v1/sites | Paginated list or register a named monitored URL |
| GET, PATCH, DELETE | /api/v1/sites/{id} | Retrieve, edit name/URL/interval/enrollment/monitoring state, or delete |
| POST | /api/v1/sites/{site_id}/subscriptions | Subscribe an opaque development identity |
| GET | /api/v1/subscriptions | List subscriptions for `subscriber_identity` |
| GET, DELETE | /api/v1/subscriptions/{id} | Retrieve or delete, scoped to `subscriber_identity` |
| GET | /api/v1/sites/{site_id}/changes | Paginated metadata history |
| GET | /api/v1/changes/{id} | Retrieve a change by stable UUID |
| GET | /unsubscribe/{token} | Display confirmation only; never changes subscription state |
| POST | /unsubscribe/{token} | Browser confirmation or RFC 8058 form post; disables that subscription |

Site intervals range from 300 to 2,592,000 seconds. The default is 86,400 seconds. URL
normalization lowercases the host, removes default ports and fragments, and preserves query
and path semantics. The normalized URL has a unique database constraint. Change UUIDs are
stable and suitable for a future public URL, but this phase does not publish public change
pages or retain page snapshots.

Site responses include `name`, `monitoring_enabled`, `active_subscriber_count`, and
`last_detected_change_at`. `POST /api/v1/sites` requires a 1–200 character name and URL;
equivalent normalized URLs retain the existing idempotent behavior and return the registered
site with 200. `PATCH /api/v1/sites/{id}` accepts optional `name`, `url`, `interval_seconds`,
`public_subscriptions_enabled`, and `monitoring_enabled`; a URL collision returns 409. URL
changes keep the site UUID, subscribers, and history, clear validators/current hash and
current check timestamps, and make the site due for a silent baseline. Monitoring is
enabled by default and disabling it prevents new scheduler claims. `DELETE` cascades the
site's subscriptions, tokens, pending notifications, and change history, and removes
related pending verification email work.

For Phase 4, the development subscription API accepts valid email addresses and creates
pending subscriptions through the same verification service as the browser flow. It never
activates an address directly. Site responses include the permanent `subscription_url` and
`public_subscriptions_enabled` setting (default `false`); PATCH changes the setting.
The authenticated server-rendered Management UI exposes the same setting on website
registration and editing forms. Its checkbox is unchecked by default. Public subscription
availability is independent of `monitoring_enabled`; disabling enrollment rejects new
registrations while preserving existing subscribers, the permanent URL, and subscriber
management and unsubscribe routes.

## Public subscription routes
These server-rendered routes need no account. A public page accepts only an existing site
UUID and is available only when the site owner has enabled public subscriptions. The URL
and monitoring interval are fixed by the monitored site.

| Method | Route | Purpose |
|---|---|---|
| GET | / | PageRune informational landing page |
| GET, POST | /subscribe/{site_id} | Render or request a pending subscription |
| GET, POST | /verify/{token} | Show a read-only form; POST consumes a token and activates |
| GET, POST | /manage | Request a temporary management email link |
| GET, POST | /manage/{token} | Show link exchange form; POST creates a session cookie |
| POST | /manage/unsubscribe/{id} | Unsubscribe within the verified email session |

Verification tokens expire after 24 hours, management links after 15 minutes, and sessions
after 30 minutes. Requests return generic responses that do not disclose existing addresses.
Form tokens and session actions are CSRF-protected. Turnstile is validated server-side.

## Planned authenticated APIs
All under `/api/v1`. Every implemented endpoint must define request/response schemas,
stable operationId, validation errors, security requirements and tenant scope in Utoipa.
Use UUIDs internally and stable site/watch slugs for public links. Error JSON uses code,
message and request_id, never raw database/provider errors. Pagination has bounded limits.

| Method | Route | Access / purpose |
|---|---|---|
| GET | /tenants | SSO: only memberships visible to caller |
| GET, POST | /tenants/{tenant}/sites | Tenant admin: list/register sites |
| GET, PATCH, DELETE | /tenants/{tenant}/sites/{site} | Tenant admin: inspect/edit/remove |
| POST | /tenants/{tenant}/sites/{site}/verify | Tenant admin: check site ownership proof |
| GET, POST | /tenants/{tenant}/sites/{site}/watches | Tenant admin: list/create watches |
| GET, PATCH, DELETE | /tenants/{tenant}/sites/{site}/watches/{watch} | Tenant admin: inspect/edit/pause/remove |
| GET | /tenants/{tenant}/sites/{site}/subscription-links | Tenant admin: general/all/per-watch URLs |
| GET | /public/sites/{slug} | Public: safe site details and enabled watch choices |
| POST | /public/sites/{slug}/subscriptions | Public: request opt-in; generic 202 response |
| POST | /public/subscriptions/confirm | Purpose-bound expiring token: activate subscription |
| POST | /public/preferences/request | Public: request a management link; generic 202 |
| GET | /public/preferences | Preference token: read permitted selections only |
| PUT | /public/preferences | Preference token: update selections, within authorized scope |
| POST | /public/subscriptions/unsubscribe | Unsubscribe token: revoke only authorized subscriptions |

Site creation does not grant permission to fetch arbitrary private URLs. Unverified
sites cannot publish active watches. No public endpoint reveals whether an email exists.
GET never confirms, unsubscribes or otherwise mutates state. A mail link opens a form;
POST performs the action. Do not send tokens in query-string API calls: forms exchange
link tokens for narrowly scoped state; scrub token-bearing URLs and use no-referrer.
The unsubscribe route is public and requires no session, cookie, or CSRF token; it accepts
standard form-encoded RFC 8058 one-click POSTs. Unknown and already-used tokens receive a
generic completion response.

## Administrative route protection
In the local authenticated environment, OAuth2 Proxy protects `/management`, all `/api/v1/*`
routes, `/api/openapi.json`, and `/swagger-ui/*`. Unauthenticated browser requests to the
management UI redirect to Keycloak; API paths return 401. Access requires membership in
the Keycloak `pagerune-management` group. The backend is not published as a host port in
Compose. `/management` provides the authenticated website list and create/edit/remove
workflows.

The following routes remain public at the proxy because they use their existing public or
email-token authorization: `/`, `/subscribe/{site_id}`, `/verify/{token}`, subscriber
`/manage` routes, `/unsubscribe/{token}`, and `/static/*`. Subscriber `/manage` is unrelated
to administrative management. Health/readiness are internal-only.

The local group gate is not tenant-level authorization. The current business API remains a
development API; future tenant APIs must enforce membership and tenant scope in the
application. The website management UI operates on the existing global site collection;
tenant membership and per-resource authorization remain future work.

## Identity
The local proxy validates the OIDC session and group before forwarding management routes.
This phase does not add OIDC token validation to Rust. Future admin JSON endpoints must
validate OAuth2 access tokens and resolve issuer+subject to explicit tenant membership on
every tenant-bound operation.

## API evolution
Before implementation choose the concrete IdP roles/scopes mapping and token-link
scope (site vs multi-site). Keep public, tenant admin, and operator capabilities separate.
Add a contract export/check command when business implementation begins. OpenAPI must
include ALL HTTP API operations; HTML navigation assets are not separate business APIs.
