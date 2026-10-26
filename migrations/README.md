# PostgreSQL migrations

Reserved for versioned SQLx migrations. No business schema is implemented in this scaffold.
Follow docs/Architecture.md for entity boundaries and composite tenant foreign keys.
Use explicit single-run migration jobs and expand/contract changes. Tests must apply
migrations to isolated PostgreSQL databases; never use SQLite as a substitute.
