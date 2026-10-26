# PostgreSQL migrations

Versioned SQLx migrations are embedded in the application. Apply them explicitly with
`direnv exec . devbox run migrate`; production should use a single-run migration job before
serving application replicas. The foundation baseline intentionally adds no product schema.
Follow docs/Architecture.md for entity boundaries and composite tenant foreign keys when
domain migrations are introduced. Use expand/contract changes. Tests apply migrations to
PostgreSQL; never use SQLite as a substitute.
