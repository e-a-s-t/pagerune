# Configuration

## Implemented
| Variable | Meaning | Default |
|---|---|---|
| PAGERUNE_BIND | HTTP socket address | 127.0.0.1:8080 |
| DATABASE_URL | PostgreSQL URL, including TLS settings | Required |
| RUST_LOG | tracing filter | pagerune=info |

The readiness probe executes SELECT 1. It does not yet validate schema migrations or
worker readiness. Database pool is currently capped at 5 connections per replica;
make limits configurable during worker implementation and size against CNPG capacity.

## Planned contract (not read by scaffold yet)
| Variable | Meaning |
|---|---|
| PAGERUNE_PUBLIC_BASE_URL | Canonical HTTPS URL used to generate admin/subscriber links |
| PAGERUNE_DIGEST_TIME | Daily local time; proposed default 18:00 |
| PAGERUNE_TIMEZONE | IANA timezone; proposed default Europe/Stockholm |
| SMTP_HOST / SMTP_PORT | SMTP endpoint |
| SMTP_TLS_MODE | starttls, implicit, none (explicit local/trusted relay use) |
| SMTP_USERNAME / SMTP_PASSWORD | Optional auth; both set together |
| SMTP_FROM_EMAIL / SMTP_FROM_NAME | Configured sender address and display name |
| OIDC_ISSUER_URL | Discovery issuer, verified exactly |
| OIDC_CLIENT_ID / OIDC_CLIENT_SECRET | Server-side browser client; secret if confidential |
| OIDC_AUDIENCE | Expected API access-token audience |
| OIDC_REDIRECT_URL | Registered callback URL |

Fetch concurrency, timeouts/body limits, job lease duration, retry/backoff bounds,
retention and rate limits need validated settings when those components are implemented.
Secrets are supplied via Kubernetes Secret or secret injection; never write them to
ConfigMaps, examples, logs, or source. Validate configuration once at startup without
printing sensitive values. Plaintext SMTP must be explicit, never a TLS-failure fallback.
