# PostgreSQL migrations

Versioned SQLx migrations are embedded in the application. Apply them explicitly with
`direnv exec . devbox run migrate`; production should use a single-run migration job before
serving application replicas. The core monitoring schema creates sites, subscriptions,
metadata-only changes, and persisted leases. Follow docs/Architecture.md for tenant
boundaries and composite foreign keys as authenticated domain tables are added. Use
expand/contract changes. Tests apply migrations to PostgreSQL; never use SQLite.
