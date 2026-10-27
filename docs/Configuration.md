# Configuration

## Implemented
| Variable | Meaning | Default |
|---|---|---|
| PAGERUNE_BIND | HTTP socket address | 127.0.0.1:8080 |
| DATABASE_URL | PostgreSQL URL, including TLS settings | Required; PostgreSQL scheme only |
| RUST_LOG | tracing filter | pagerune=info |
| PAGERUNE_DEV_API_ENABLED | Enable temporary business APIs; requires loopback bind | false |
| PAGERUNE_FETCH_TIMEOUT_SECONDS | Per-request HTTP timeout in seconds | 15 (range 1–120) |
| PAGERUNE_MAX_RESPONSE_BYTES | Maximum fetched body size | 2097152 (range 1024–20971520) |

Startup rejects malformed bind addresses, missing or invalid PostgreSQL URLs, and other
database schemes. Configuration errors do not include URL credentials. The readiness probe
executes SELECT 1. It does not validate migration or worker readiness. The fetch worker
uses a 180-second lease and retries failed requests within five minutes. The database pool
is capped at five connections per replica; size total connections against CNPG capacity.

## Implemented SMTP and notification settings
| Variable | Meaning | Default |
|---|---|---|
| SMTP_HOST / SMTP_PORT | SMTP server endpoint | 127.0.0.1 / 1025 |
| SMTP_TLS_MODE | `none`, `starttls`, or `implicit` | none |
| SMTP_USERNAME / SMTP_PASSWORD | Optional auth; configure both or neither | unset |
| SMTP_FROM_EMAIL / SMTP_FROM_NAME | Message sender | updates@localhost / PageRune |
| SMTP_TIMEOUT_SECONDS | Connection/operation timeout | 15 (range 1–120) |
| PAGERUNE_NOTIFICATION_CONCURRENCY | Maximum in-flight deliveries per replica | 2 (range 1–32) |
| PAGERUNE_NOTIFICATION_MAX_ATTEMPTS | Attempts before terminal failure | 8 (range 1–20) |
| PAGERUNE_PUBLIC_BASE_URL | Reserved for future PageRune links | http://localhost:8080 |

## Planned contract
| Variable | Meaning |
|---|---|
| PAGERUNE_DIGEST_TIME | Daily local time; proposed default 18:00 |
| PAGERUNE_TIMEZONE | IANA timezone; proposed default Europe/Stockholm |
| OIDC_ISSUER_URL | Discovery issuer, verified exactly |
| OIDC_CLIENT_ID / OIDC_CLIENT_SECRET | Server-side browser client; secret if confidential |
| OIDC_AUDIENCE | Expected API access-token audience |
| OIDC_REDIRECT_URL | Registered callback URL |

Retention and rate limits need validated settings before public launch.
Secrets are supplied via Kubernetes Secret or secret injection; never write them to
ConfigMaps, examples, logs, or source. Validate configuration once at startup without
printing sensitive values. Plaintext SMTP must be explicit, never a TLS-failure fallback.
