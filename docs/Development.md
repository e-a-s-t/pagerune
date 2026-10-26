# Local development

## Prerequisites

- [Devbox](https://www.jetify.com/docs/devbox/) for the Rust toolchain and development commands.
- [direnv](https://direnv.net/) with its shell hook enabled, so application settings are loaded into your shell.
- Docker Compose and a running compatible container engine for PostgreSQL and Mailpit.

The Rust application runs on the host through Devbox. Only PostgreSQL and Mailpit run in containers. Devbox does not provide a container engine.

## Initial setup

From the repository root, copy the example environment file, review its values, and approve direnv's environment for this directory:

```sh
cp .env.example .env
# Review .env, especially database credentials and connection settings.
direnv allow
```

`.env` is ignored by Git. `.envrc` loads `.env` and then `.env.example` through direnv; keep development application variables in these direnv-managed files (or ignored `.envrc.local` overrides), not in Devbox configuration or application dotenv code. If your shell hook is active, entering the directory loads the environment. `direnv exec . <command>` explicitly runs a command with it loaded.

## Start local services

Start the existing PostgreSQL and Mailpit services from the repository root:

```sh
direnv exec . docker compose up -d
```

Compose reads `POSTGRES_DB`, `POSTGRES_USER`, and `POSTGRES_PASSWORD` from the direnv environment. It publishes PostgreSQL on `127.0.0.1:5432` and Mailpit's SMTP and web interfaces on `127.0.0.1:1025` and `127.0.0.1:8025`, respectively. PostgreSQL stores its data in the named `postgres-data` volume.

The locally running Rust application uses `DATABASE_URL` from `.env` to connect to PostgreSQL at `127.0.0.1:5432`. The example's database name, user, and password match the Compose variables. SMTP settings in `.env` point at Mailpit (`SMTP_HOST=127.0.0.1`, `SMTP_PORT=1025`, `SMTP_TLS_MODE=none`), which is suitable for local mail capture. These settings are planned configuration only: the current application does not read them and does not send email.

## Run PageRune and checks

Run the server locally:

```sh
direnv exec . devbox run run
```

In another terminal, run the configured check suite:

```sh
direnv exec . devbox run check
```

Available Devbox scripts are:

- `devbox run run`: run PageRune with Cargo.
- `devbox run check`: check formatting, run Clippy with warnings denied, and run tests.
- `devbox run test`: run tests only.
- `devbox run fmt`: format Rust code.
- `devbox run build`: build a release binary.

Prefix these with `direnv exec .` if direnv has not already loaded the project environment in the current shell. `DATABASE_URL` is required at startup. The server binds to `127.0.0.1:8080` by default. Swagger UI is available at <http://127.0.0.1:8080/swagger-ui/> and the generated OpenAPI document at <http://127.0.0.1:8080/api/openapi.json>. Mailpit's web interface is <http://127.0.0.1:8025>.

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

The current scaffold provides an Axum HTTP server, JSON logs, graceful shutdown, generated OpenAPI and bundled Swagger UI, `GET /healthz`, and `GET /readyz`. Readiness runs `SELECT 1` against PostgreSQL. It does not check migration status.

There are no SQLx migrations or business database schema yet. Subscription, tenant, OIDC, checking, digest, and email workflows are planned and are not implemented. In particular, no SMTP client or email sending exists; Mailpit is available to capture mail once sending is implemented.
