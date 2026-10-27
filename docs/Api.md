# API and UI contract

## Current implementation
Generated OpenAPI: `GET /api/openapi.json`. Bundled Swagger UI: `GET /swagger-ui/`.
Operational routes: `GET /healthz` and `GET /readyz` (200 or 503 based on PostgreSQL).
Business routes are under `/api/v1`. They are disabled by default. Set
`PAGERUNE_DEV_API_ENABLED=true` only for development and bind `PAGERUNE_BIND` to loopback;
startup rejects enabling this mode on a non-loopback bind. This temporary mode is not
authentication and must not be exposed publicly. Error responses use
`{code, message, request_id}` without provider details; `X-Request-ID` is returned.
Collections use `page` (default 1) and `limit` (default 50, maximum 100).

| Method | Route | Purpose |
|---|---|---|
| GET, POST | /api/v1/sites | List or register a monitored URL |
| GET, PATCH, DELETE | /api/v1/sites/{id} | Retrieve, configure interval, or delete |
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

For Phase 3 delivery, `subscriber_identity` is also used as the SMTP recipient. Supply a
valid email address to receive notifications. Existing non-email development labels remain
valid API identities but are recorded as permanent delivery failures if a change occurs.

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

## Planned HTML routes
- `/{site_slug}/`: subscription form, query preselection `all=true` or repeated `watch`.
- `/admin/`: SSO-only site/watch dashboard, checks/errors and copy-link controls.
- `/subscribe/confirm`, `/preferences`, `/unsubscribe`: token entry/confirmation pages.
- `/auth/login`, `/auth/callback`, `/auth/logout`: OIDC Authorization Code with PKCE,
  state/nonce validation, CSRF-safe logout. Sessions in PostgreSQL, secure cookies.
Reserve these path segments so public site slugs cannot collide with system routes.

## Identity
Admin JSON endpoints validate OAuth2 access tokens (issuer, signature, audience, expiry,
allowed algorithm and scopes). Do not use ID tokens as API bearer tokens. Resolve
issuer+subject to explicit tenant membership on every tenant-bound operation.
Swagger authorization uses Authorization Code with PKCE and a public IdP client; never
embed a client secret. Browser UI may use a confidential server-side OIDC client.

## API evolution
Before implementation choose the concrete IdP roles/scopes mapping and token-link
scope (site vs multi-site). Keep public, tenant admin, and operator capabilities separate.
Add a contract export/check command when business implementation begins. OpenAPI must
include ALL HTTP API operations; HTML navigation assets are not separate business APIs.
