# PageRune

Website updates, delivered. A self-hosted service where site owners register pages
and visitors subscribe to all updates or selected topics. Pages are checked once
before a daily combined digest. Update links go directly to the original pages.

## Status
**Core monitoring, customizable HTML/text email, and secure unsubscribe implemented; not a complete or deployable subscription service.** The
service persists normalized monitored URLs and development subscriptions, checks due sites
asynchronously with SSRF controls, records hash-based change metadata, queues one email per
active subscription in a PostgreSQL transactional outbox, and coordinates monitoring and
delivery workers with leases and fencing. Site, subscription and change APIs are in
OpenAPI; the business API is disabled by default and only available with explicit opt-in on
a loopback bind. SMTP delivery is at-least-once; exactly-once delivery cannot be guaranteed
if a process crashes after SMTP acceptance and before recording success. Tenant
authentication, email verification, frontend, daily digest, and production onboarding are
not implemented.

## Development
Prerequisites: install Devbox, direnv with its shell hook enabled, and Docker Compose. The
Rust application runs locally through Devbox; the existing Compose file runs PostgreSQL
and Mailpit. See [the local development guide](docs/Development.md) for details.

```sh
cp .env.example .env
# Review local values; .env is ignored by Git.
devbox install
direnv allow
direnv exec . docker compose up -d
direnv exec . devbox run migrate
direnv exec . devbox run run
```

In another terminal, run `direnv exec . devbox run check` for formatting, Clippy, unit
tests, and PostgreSQL integration tests. Stop services with
`direnv exec . docker compose down`; PostgreSQL data remains in its Compose volume. Swagger
UI is at http://127.0.0.1:8080/swagger-ui/ and Mailpit at http://127.0.0.1:8025. Run the
migration command again after pulling migration changes.

First setup downloads the pinned toolchain and dependencies. `devbox.lock` and
`Cargo.lock` are generated lockfiles and are committed to keep the development
environment and Rust dependency resolution reproducible.
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
