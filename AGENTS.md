# PageRune

Self-hosted, multi-tenant website update subscriptions built in Rust.

## Project context
Read README.md and docs/Requirements.md before implementation.
Consult docs/Architecture.md, docs/Adr.md, docs/Api.md and
docs/Testcases.md as relevant to the task.

Keep product requirements in docs/, rather than duplicating them here.
Record significant architecture decisions in docs/Adr.md.

## Development
- Use Devbox for development tools and commands.
- Use direnv for all development application environment variables.
- Keep secrets in ignored .env or .envrc.local; never commit them.
- Commit tool-generated Cargo.lock and devbox.lock.
- Use PostgreSQL for development and tests; CloudNativePG in production.

## Implementation
- Keep application replicas stateless and interchangeable.
- Enforce tenant isolation on every tenant-owned operation.
- Use OIDC SSO and OAuth2 for administrative authentication and API access.
- Document all implemented APIs in OpenAPI and expose Swagger UI.
- Keep the frontend server-rendered with minimal JavaScript.
- Protect against SSRF, unsafe content rendering and token disclosure.
- Do not introduce additional infrastructure without a documented need.

## Verification
Run `direnv exec . devbox run check` and relevant integration tests.
Update documentation and API schemas when behavior changes.
Report checks that could not be run and features that remain unimplemented.
