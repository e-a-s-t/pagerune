# Local development

## Prerequisites

- [Devbox](https://www.jetify.com/docs/devbox/) for the Rust toolchain and development commands.
- [direnv](https://direnv.net/) with its shell hook enabled, so application settings are loaded into your shell.
- Docker Compose and a running compatible container engine for PostgreSQL and Mailpit.

## Email templates
Edit `templates/email/change.html` and `templates/email/change.txt` while the app runs;
the next notification uses those changes without recompilation or restart. Inspect
multipart messages in Mailpit at `http://127.0.0.1:8025`. Set `PAGERUNE_PUBLIC_BASE_URL`
in `.env` to the origin used for unsubscribe links.

The Rust application runs on the host through Devbox. Only PostgreSQL and Mailpit run in containers. Devbox does not provide a container engine.

## Initial setup

From the repository root, copy the example environment file, review its values, and approve direnv's environment for this directory:

```sh
cp .env.example .env
# Review .env, especially database credentials and connection settings.
devbox install
direnv allow
```

`.env` is ignored by Git. `.envrc` loads `.env` and then `.env.example` through direnv; keep development application variables in these direnv-managed files (or ignored `.envrc.local` overrides), not in Devbox configuration or application dotenv code. If your shell hook is active, entering the directory loads the environment. `direnv exec . <command>` explicitly runs a command with it loaded.

## Start local services

Start the existing PostgreSQL and Mailpit services from the repository root:

```sh
direnv exec . docker compose up -d
```

Compose reads `POSTGRES_DB`, `POSTGRES_USER`, and `POSTGRES_PASSWORD` from the direnv environment. It publishes PostgreSQL on `127.0.0.1:5432` and Mailpit's SMTP and web interfaces on `127.0.0.1:1025` and `127.0.0.1:8025`, respectively. PostgreSQL stores its data in the named `postgres-data` volume.

The locally running Rust application uses `DATABASE_URL` from `.env` to connect to PostgreSQL at `127.0.0.1:5432`. The example's database name, user, and password match the Compose variables. SMTP settings point at Mailpit (`SMTP_HOST=127.0.0.1`, `SMTP_PORT=1025`, `SMTP_TLS_MODE=none`, empty auth, `SMTP_FROM_EMAIL=updates@localhost`, `SMTP_FROM_NAME=PageRune`).

Apply the versioned SQLx migrations before starting the application:

```sh
direnv exec . devbox run migrate
```

Migrations create the monitoring and notification outbox tables. Run this command again after pulling new migrations.

## Run PageRune and checks

Run the server locally. The worker sends email for changes detected after subscription; baseline fetches remain silent:

```sh
direnv exec . devbox run run
```

In another terminal, run the configured check suite:

```sh
direnv exec . devbox run check
```

Available Devbox scripts are:

- `devbox run run`: run PageRune with Cargo.
- `devbox run migrate`: apply embedded SQLx migrations to `DATABASE_URL`.
- `devbox run check`: check formatting, run Clippy with warnings denied, and run unit and PostgreSQL integration tests.
- `devbox run test`: run tests only.
- `devbox run fmt` / `devbox run fmt-check`: format Rust code or verify formatting.
- `devbox run lint`: run Clippy with warnings denied.
- `devbox run build` / `devbox run build-release`: build debug or release binaries.

Prefix these with `direnv exec .` if direnv has not already loaded the project environment in the current shell. `DATABASE_URL` is required at startup. The server binds to `127.0.0.1:8080` by default. Swagger UI is available at <http://127.0.0.1:8080/swagger-ui/> and the generated OpenAPI document at <http://127.0.0.1:8080/api/openapi.json>. Mailpit's web interface is <http://127.0.0.1:8025>.

## Verify an email in Mailpit

1. Start PostgreSQL and Mailpit with `direnv exec . docker compose up -d`.
2. Apply migrations and start PageRune using the commands above.
3. Open Swagger at <http://127.0.0.1:8080/swagger-ui/>. With the development API enabled, register a public monitored website using `POST /api/v1/sites`.
4. Create a subscription using `POST /api/v1/sites/{site_id}/subscriptions` with an email address as `subscriber_identity`.
5. Wait for a later check to detect a content change (the initial baseline does not notify).
6. Open <http://localhost:8025> and inspect the captured email. It includes the source URL, detection time and stable change UUID, but no page snapshot.

To speed up local checks, set the site's `interval_seconds` to the five-minute minimum. Fetching rejects loopback/private URLs, so use a public HTTP(S) website. Mailpit captures SMTP locally; no email is delivered externally.

## Inspect and stop services

Check service status and view logs with Compose:

```sh
direnv exec . docker compose ps
direnv exec . docker compose logs -f postgres mailpit
```

Stop and remove the containers while retaining PostgreSQL data in the named volume:

```sh
direnv exec . docker compose down
```

### Destructive: delete local database data

The following command also removes Compose-managed volumes, including `postgres-data`, and permanently deletes the local PostgreSQL data:

```sh
direnv exec . docker compose down -v
```

## Implemented and planned functionality

The application provides the Axum API, PostgreSQL persistence, generated OpenAPI/Swagger,
health/readiness, monitored URL and subscription APIs, asynchronous site checks, hash-based
change history, SSRF restrictions, and PostgreSQL leases with fencing. Migrations are
applied explicitly. The business API is disabled unless `PAGERUNE_DEV_API_ENABLED=true`;
that option requires the default loopback bind and is only for local development.

OIDC, tenant membership, subscriber verification, HTML UI, daily digest, and public change
pages remain unimplemented. Email delivery uses PostgreSQL outbox rows; Mailpit is the
local SMTP capture service.
