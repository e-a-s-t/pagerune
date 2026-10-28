# Local development

## Prerequisites and setup

Install Devbox, direnv with its shell hook, Docker Compose, and a compatible container
engine. Devbox supplies the Rust toolchain; direnv supplies app and Compose environment
variables. First setup:

```sh
cp .env.example .env
devbox install
direnv allow
direnv exec . docker compose up -d
```

The values in `.env.example` are development-only. `.env` is ignored by Git; use
`.envrc.local` for local overrides. No manual Keycloak console setup is needed. The
PostgreSQL service creates a separate `pagerune_keycloak` database idempotently, and
Keycloak imports the realm, client, group, and development user at startup.

## Compose application workflow

The default Compose workflow runs PostgreSQL, Keycloak, PageRune, OAuth2 Proxy, and Mailpit.
Only OAuth2 Proxy is published for PageRune traffic, at `127.0.0.1:8088`; the backend is
on private Compose networks and has no host-published port. Public site: <http://localhost:8088>.
Website management: <http://localhost:8088/management>. Sign in as `admin` / `admin`.
Keycloak uses the browser-visible issuer <http://keycloak.localhost:8081/realms/pagerune>;
the `auth` network DNS alias maps that same hostname to Keycloak for OAuth2 Proxy, preserving
exact issuer validation without assigning a fixed container IP or subnet.
Mailpit is at <http://127.0.0.1:8025>.

OAuth2 Proxy routes to PageRune over the private Compose network. Its explicit public path
rules preserve subscription registration, verification, subscriber email management,
unsubscribe, landing, and assets. Unauthenticated management page requests redirect
straight to Keycloak. Management paths require membership in the imported
`pagerune-management` group. The `/management` logout link clears the local proxy session;
the next management request starts authentication again. Keycloak may silently reuse its
own browser SSO session.

The Compose app applies embedded SQLx migrations through a one-shot service before startup.
Run `direnv exec . docker compose up -d --build` after source changes. PostgreSQL data is
retained in `postgres-data`; `docker compose down -v` deletes it.

## Host-run Devbox workflow

To run PageRune under Devbox while retaining Keycloak and OAuth2 Proxy, stop the default
proxy/app services and start the host-mode proxy profile:

```sh
direnv exec . docker compose stop oauth2-proxy pagerune
direnv exec . docker compose --profile host-app up -d postgres mailpit keycloak oauth2-proxy-host
```

The host-mode proxy reaches the host process at the Docker bridge gateway on port 8082.
Get the gateway and run migrations and PageRune bound to that interface:

```sh
GATEWAY=$(direnv exec . docker network inspect bridge --format '{{(index .IPAM.Config 0).Gateway}}')
direnv exec . env PAGERUNE_BIND="$GATEWAY:8082" PAGERUNE_TRUSTED_PROXY_MODE=true PAGERUNE_PUBLIC_BASE_URL=http://localhost:8088 devbox run migrate
direnv exec . env PAGERUNE_BIND="$GATEWAY:8082" PAGERUNE_TRUSTED_PROXY_MODE=true PAGERUNE_PUBLIC_BASE_URL=http://localhost:8088 devbox run run
```

OAuth2 Proxy uses `host.docker.internal:8082`; the host PageRune process does not use port
8088, which remains the browser-facing proxy port. This listener is a local development
transport and is reachable from the host itself. Production must use private service
networking, an authenticated HTTPS ingress, Secure cookies, and production credentials.

The existing direct host workflow remains available when authentication is not needed:
start PostgreSQL and Mailpit, run `direnv exec . devbox run migrate`, then
`direnv exec . devbox run run` with the default loopback bind. Do not expose this direct
development listener.

To switch back to the default Compose app, stop the host proxy first so it releases port
8088, then run `direnv exec . docker compose up -d --build`.

## Checks and email

Run the existing suite with:

```sh
direnv exec . devbox run check
```

This checks formatting, Clippy, unit tests, and PostgreSQL integration tests. For local
Compose authentication checks, start the stack and run `python3 tests/compose-auth.py`.

Edit `templates/email/change.html` and `templates/email/change.txt` while the app runs;
the next notification uses those changes without recompilation. Mailpit captures SMTP
locally, and no email is delivered externally.
