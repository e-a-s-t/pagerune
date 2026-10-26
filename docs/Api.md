# API and UI contract

## Current scaffold
Generated OpenAPI: `GET /api/openapi.json`. Bundled Swagger UI: `GET /swagger-ui/`.
Implemented operations only: `GET /healthz` (200), `GET /readyz` (200 or 503 based on
PostgreSQL reachability). Infrastructure/docs routes are public. The OpenAPI document
must never claim business routes are implemented before they are registered and secured.

## Planned business APIs
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
