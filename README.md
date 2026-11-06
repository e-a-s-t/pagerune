# PageRune

Website updates, delivered. A self-hosted service where site owners register pages
and visitors subscribe to updates. PageRune monitors normalized pages and sends email
notifications when they change.

## Status

Core monitoring, public double opt-in subscriptions, email-token subscriber management,
secure unsubscribe, local OIDC management access, and a functional website administration
UI are implemented. Administrators can register, edit, pause, inspect, and remove monitored
websites. Tenant membership and per-tenant resource authorization remain future work. SMTP
delivery is at-least-once.

## Development

Install Devbox, direnv with its shell hook, and Docker Compose. Copy the ignored local
environment file and start the full Compose stack:

```sh
cp .env.example .env
devbox install
direnv allow
direnv exec . docker compose up -d
```

Open the public site at http://localhost:8088. The protected Management UI is at
http://localhost:8088/management; sign in with the local Keycloak account `admin` /
`admin`. These are development-only credentials. Management logout clears the OAuth2 Proxy
session. Keycloak is available at http://keycloak.localhost:8081 and Mailpit at
http://127.0.0.1:8025. The realm and client are imported automatically.

The original host-run Devbox workflow is also supported; see
[docs/Development.md](docs/Development.md) for host gateway setup and migration commands.
Run `direnv exec . devbox run check` for formatting, Clippy, unit tests, and PostgreSQL
integration tests. `devbox.lock` and `Cargo.lock` are committed. The application does not
load `.env`; direnv provides environment variables.

## Production image and releases

The production image is published to Docker Hub as
[`erjoda/pagerune`](https://hub.docker.com/r/erjoda/pagerune). Build it locally with
`docker build -t pagerune:local .`; see [docs/Development.md](docs/Development.md) for a
local run example and [docs/Operations.md](docs/Operations.md) for release instructions.
The image receives all application configuration and secrets through environment variables;
it does not contain deployment configuration or credentials.

## Documentation

- [Requirements](docs/Requirements.md): product scope and acceptance criteria.
- [Architecture](docs/Architecture.md): tenancy, delivery, and local authentication.
- [ADRs](docs/Adr.md): architecture decisions.
- [API](docs/Api.md): routes and security boundaries.
- [Test cases](docs/Testcases.md): behavioral verification matrix.
- [Development](docs/Development.md): local workflows and commands.
- [Configuration](docs/Configuration.md): application and local auth settings.
- [Security](docs/Security.md): identity, tenancy, tokens, and fetching controls.
- [Operations](docs/Operations.md): CNPG, replicas, backups, and observability.
- [Roadmap](docs/Roadmap.md): implementation phases.

## License

Not selected. There is deliberately no LICENSE file; the owner must choose terms before
public redistribution. No association with any existing PageRune product is claimed.
