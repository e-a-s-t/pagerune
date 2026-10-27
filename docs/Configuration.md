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
| PAGERUNE_PUBLIC_BASE_URL | Public origin for subscription, verification, management and unsubscribe URLs | http://localhost:8080 |
| PAGERUNE_EMAIL_TEMPLATE_DIR | Directory containing `change.html` and `change.txt` | templates/email |
| PAGERUNE_TOKEN_KEY | 64-hex-character shared key for deriving queued verification and management link tokens | Required |
| PAGERUNE_TURNSTILE_SITE_KEY | Public widget key | Required unless loopback test mode |
| PAGERUNE_TURNSTILE_SECRET | Server-side Turnstile validation secret | Required unless loopback test mode |
| PAGERUNE_TURNSTILE_TEST_MODE | Enable fixed local test token; requires the loopback development API | false |

The subscription and management forms render the Turnstile widget with its appearance set
to `always` when test mode is off. Configure the Cloudflare widget in Managed mode
(recommended) or Non-Interactive mode; Invisible mode suppresses the visible widget and is
not appropriate for these forms. The Cloudflare widget mode is selected in its dashboard
configuration, while the page sets the appearance. Turnstile site keys are public and
rendered into the page; the secret remains server-side. Every POST validates the token with Turnstile and checks its
success, action and hostname. Invalid, expired, replayed, malformed, timed-out or
unavailable provider responses are rejected. Production must provide both Turnstile keys;
startup fails if either is missing. For local development only, set
`PAGERUNE_DEV_API_ENABLED=true` on a loopback bind and
`PAGERUNE_TURNSTILE_TEST_MODE=true`; the page then submits the explicit fixed test token
and no external challenge is used. Do not use test mode in production.

Verification links expire after 24 hours, management links after 15 minutes, and
management sessions after 30 minutes. Public form request limits are PostgreSQL-backed:
10 requests per IP/hour and 3 per normalized email/hour. Keep `PAGERUNE_TOKEN_KEY`
identical across replicas and protect it as a secret. Rotate it only after pending action
email jobs have drained, since those jobs regenerate their token links from this key.

Email templates use MiniJinja variables `site_url`, `site_name`, `detected_at`,
`change_id`, `unsubscribe_url`, and `application_name`. Bundled files are defaults.
Template files are read for each email, so edits apply without recompilation or restart.
Missing files fall back to bundled defaults; unreadable or invalid templates fail delivery.
HTML variables are autoescaped.

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
