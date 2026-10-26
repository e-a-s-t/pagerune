# PageRune

Website updates, delivered. A self-hosted service where site owners register pages
and visitors subscribe to all updates or selected topics. Pages are checked once
before a daily combined digest. Update links go directly to the original pages.

## Status
**Scaffold, not a complete or deployable subscription service.** Implemented:
Rust/Axum HTTP server, PostgreSQL readiness, JSON logs, graceful shutdown, generated
OpenAPI at `/api/openapi.json` and bundled Swagger UI at `/swagger-ui/`.
Business endpoints, OIDC, frontend, checking and email workers are planned, not implemented.
No authentication-protected business functionality is exposed by this scaffold.

## Development
Prerequisites: install Devbox, direnv with its shell hook enabled, and Docker Compose. The
Rust application runs locally through Devbox; the existing Compose file runs PostgreSQL
and Mailpit.

```sh
cp .env.example .env
# Review .env settings; it is ignored by Git.
direnv allow
direnv exec . docker compose up -d
direnv exec . devbox run run
```

Run `direnv exec . devbox run check` for formatting, Clippy, and tests. PostgreSQL
data persists in a Compose volume when containers are stopped with
`direnv exec . docker compose down`. Swagger UI is at
http://127.0.0.1:8080/swagger-ui/ and Mailpit at http://127.0.0.1:8025.
The scaffold does not yet include migrations or SMTP sending; see the
[local development guide](docs/Development.md) for details and container commands.

First setup downloads toolchain/dependencies. Commit the tool-generated `devbox.lock`
and `Cargo.lock` once generated. This scaffold contains neither fabricated lockfile.
The application intentionally does not load `.env`; direnv supplies the environment.
CI must run the same Devbox scripts with its environment injected explicitly.

## Documentation
- [Requirements](docs/Requirements.md): agreed scope and acceptance criteria.
- [Architecture](docs/Architecture.md): tenancy, daily cycle and durable delivery.
- [ADRs](docs/Adr.md): accepted decisions and unresolved choices.
- [API plan](docs/Api.md): route inventory and security boundaries.
- [Test cases](docs/Testcases.md): behavioral verification matrix.
- [Development](docs/Development.md): tools, environment and commands.
- [Configuration](docs/Configuration.md): current and planned settings.
- [Security](docs/Security.md): identity, tenancy, token and fetching controls.
- [Operations](docs/Operations.md): CNPG, replicas, backups and observability.
- [Roadmap](docs/Roadmap.md): implementation phases.

## License
Not selected. There is deliberately no LICENSE file; the owner must choose terms
before public redistribution. No association with any existing PageRune product is claimed.
