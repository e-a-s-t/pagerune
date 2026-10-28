# Test cases

Status: PLANNED unless explicitly marked IMPLEMENTED. Implemented does not mean executed.
Use local PostgreSQL for integration tests, never SQLite substitutes. SMTP tests use
a local capture server. OIDC tests use controlled discovery/JWKS and invalid-token fixtures.

| ID | Scenario | Expected result | Layer / status |
|---|---|---|---|
| OPS-01 | Process without reachable database | /healthz 200; /readyz 503 | HTTP / IMPLEMENTED |
| API-01 | Fetch OpenAPI | Every implemented operation and error response described | Contract / IMPLEMENTED |
| CFG-00 | Missing, malformed or non-PostgreSQL config | Startup validation rejects it without echoing secrets | Unit / IMPLEMENTED |
| DB-00 | PostgreSQL connectivity and baseline migration | `SELECT 1` succeeds and migration version is recorded | PostgreSQL integration / IMPLEMENTED |
| CORE-01 | Equivalent URL registration | Normalized URL uniqueness returns the same monitored site | Unit + PostgreSQL integration / IMPLEMENTED |
| CORE-02 | Invalid scheme, userinfo, interval, or pagination | Request rejected with structured error | Unit / HTTP |
| SUB-09 | Create, list, retrieve, and delete a development subscription | Operations succeed only for the matching opaque subscriber identity | PostgreSQL integration / IMPLEMENTED |
| MON-01 | Initial, unchanged, then changed hash | Baseline is silent; unchanged content is ignored; changed content records old/new hashes | PostgreSQL worker integration / IMPLEMENTED |
| MAIL-01 | Baseline, unchanged, changed with multiple subscriptions | Only changed content creates one unique outbox row per active subscription | PostgreSQL worker integration / IMPLEMENTED |
| MAIL-02 | Subscription created after a change | No historical outbox rows are created | PostgreSQL worker integration |
| MAIL-03 | Delete subscription with queued notification | Pending rows are removed by FK cascade | PostgreSQL worker integration / IMPLEMENTED |
| MAIL-04 | Concurrent outbox claims, expired lease and stale completion | Claims do not overlap; recovery advances generation; stale worker cannot record a result | PostgreSQL integration / IMPLEMENTED |
| MAIL-05 | SMTP success to local capture server | Multipart plain-text/HTML message is accepted by Mailpit | SMTP integration / IMPLEMENTED |
| MAIL-06 | Temporary/permanent SMTP rejection and retry limit | Backoff retry or terminal failure is recorded without logging body/secrets | SMTP integration |
| MAIL-07 | Worker shutdown/restart | Active send drains; expired work is recoverable after restart | PostgreSQL + SMTP integration |
| MAIL-08 | HTML and text templates render shared variables and escape untrusted values | Both alternatives contain source/change/unsubscribe details; no source HTML is rendered | Unit / IMPLEMENTED |
| MAIL-09 | Missing or syntactically invalid template | Missing file uses bundled default; invalid template is reported as terminal template failure | Unit / IMPLEMENTED |
| SUB-10 | Unsubscribe GET, invalid token, repeated browser/one-click POST | GET is read-only; invalid token does not affect a subscription; POST is idempotent and removes only the matching subscription | HTTP + PostgreSQL / IMPLEMENTED |
| PUB-01 | Site subscription flag omitted at registration | Flag defaults false and generated public URL remains inactive | HTTP + PostgreSQL |
| PUB-02 | Owner enables/disables site enrollment through private API | Only enabled site URLs render an enrollment form; disabled site GET/POST cannot create subscriptions | HTTP |
| PUB-03 | Public subscription with forged/failed/expired Turnstile token | Server rejects and does not create a subscription; local test verifier is loopback-only or behind explicit trusted proxy mode in Compose | HTTP + provider adapter |
| PUB-13 | Turnstile success, invalid, expired, replayed, wrong action/hostname, timeout and provider failure | Only successful tokens for the expected action and hostname are accepted; failures reject enrollment and management requests | Provider adapter unit + HTTP |
| UI-02 | Monitoring interval labels on subscribe and management pages | 60 seconds through one day use correct units and singular/plural without zero rounding | Unit + rendered page |
| PUB-04 | Valid public request, duplicate active/pending address, concurrent duplicate requests | Generic response; one site/email row; active row is not duplicated; pending token is rotated safely | PostgreSQL concurrency |
| PUB-05 | Verification GET, valid POST, repeat POST, wrong/expired token, invalid CSRF nonce | GET never changes state; only the first valid POST activates | HTTP + PostgreSQL |
| PUB-06 | Pending versus active notification generation and delivery | Pending rows receive no outbox message; verified active rows receive notifications | PostgreSQL integration |
| PUB-07 | Verification email outbox delivery through Mailpit | Token link is derived from shared key and ID; only its hash is stored; delivery is leased and retryable | PostgreSQL + SMTP |
| PUB-08 | Management email request for absent/existing address | Same response and timing shape; request is rate limited and email sent through outbox | HTTP + SMTP |
| PUB-09 | Management token GET, POST exchange, repeated exchange, session on another replica | GET is read-only; POST consumes once; only verified email subscriptions are visible | HTTP + PostgreSQL |
| PUB-10 | Management session CSRF, expiry and individual unsubscribe | Cross-session/expired requests fail; valid POST removes only selected subscription | HTTP + PostgreSQL |
| PUB-11 | Subscription and management rate limits across replicas | IP/email windows are PostgreSQL-backed and consistently enforced | PostgreSQL concurrency |
| PUB-12 | Oversized public form body and invalid email/URL fields | Body is rejected; only valid emails accepted; public input cannot register or change a website URL | HTTP |
| MAIL-10 | Unsubscribe races with pending or in-flight delivery on multiple replicas | Pending outbox work is cascaded; unsubscribe waits for a send holding the shared PostgreSQL advisory lock; no send begins after unsubscribe commits; accepted mail cannot be recalled | PostgreSQL concurrency + SMTP |
| MAIL-11 | Captured email headers and body in Mailpit | Multipart HTML/text and RFC 8058 headers are present | SMTP integration / IMPLEMENTED |
| MON-02 | Failed fetch after baseline | Baseline remains unchanged and a stable error is recorded | PostgreSQL worker integration / IMPLEMENTED |
| SEC-05 | Private/loopback/metadata destination | Fetch is rejected before connecting | Unit + local HTTP / IMPLEMENTED |
| API-02 | Malformed JSON and disabled business API | Structured JSON error with request ID | HTTP integration / IMPLEMENTED |
| OWN-01 | Site ownership not verified | Cannot activate watches or publish subscription form | Integration |
| TEN-01 | Tenant A requests B site/watch IDs | No disclosure or mutation; composite FK prevents cross-link | Integration |
| TEN-02 | Same person subscribes to two tenants | Single daily digest; owners see only their own contact scope | Integration |
| AUTH-01 | Wrong issuer, expired/forged token or invalid OIDC session | OAuth2 Proxy rejects authentication | Compose integration |
| AUTH-02 | Valid local administrator versus a user outside the management group | Only the configured management group reaches management routes; tenant authorization is not implied | Compose integration |
| AUTH-03 | OIDC callback has wrong state/nonce | OAuth2 Proxy rejects invalid callback | Compose integration |
| AUTH-04 | Public routes through OAuth2 Proxy without a session | Landing, subscribe, verify, subscriber `/manage`, unsubscribe and static routes remain accessible | Compose integration |
| AUTH-05 | Unauthenticated management UI request | Redirects to Keycloak; website management page requires a session | Compose integration |
| AUTH-06 | Unauthenticated management API request with JSON Accept | Returns 401 without a login redirect | Compose integration |
| AUTH-07 | Seeded administrator login | `admin` / `admin` obtains authorized management access | Compose integration |
| AUTH-08 | Invalid credentials | Management access is not granted | Compose integration |
| AUTH-09 | Logout then revisit management | Proxy cookie is cleared; next request requires authentication again | Compose integration |
| AUTH-10 | Spoofed identity headers and direct backend access | Proxy strips client auth headers; Compose has no published backend port | Compose integration |
| AUTH-11 | Realm bootstrap with existing PostgreSQL volume | DB setup is idempotent and realm/client/user import needs no console setup | Compose integration |
| UI-03 | Management listing with websites, empty state, pagination, status/history and active subscriber counts | Correct escaped site information and bounded page navigation | HTTP + PostgreSQL |
| UI-03a | Management subscription link for enabled and disabled sites | Absolute URL uses configured public base URL and stable site UUID; View and Copy link are present in both states, state is clear, and HTML is escaped | Unit + HTTP |
| UI-03b | Disabled site's public subscription link | Link remains visible in management, but public endpoint rejects enrollment and creates no subscription | HTTP + PostgreSQL |
| UI-04 | Register, edit name/URL/interval, pause/resume and remove through management forms | Changes persist without restart; URL edits retain site identity and rebaseline silently | HTTP + PostgreSQL |
| UI-04a | Register and edit public subscription availability through management forms | Registration defaults disabled; enable/disable persists independently of monitoring; current state is shown after save and the permanent URL remains available | HTTP + PostgreSQL |
| UI-05 | Invalid/duplicate management URL, invalid name, expired or forged CSRF nonce | No invalid mutation; clear validation feedback and single-use form protection for all management settings | HTTP + PostgreSQL |
| UI-06 | Remove website with active subscribers and pending verification/notification work | In-flight send drains; related subscriptions, tokens, history, and pending work are removed | PostgreSQL + SMTP concurrency |
| MON-03 | Disabled site becomes due, scheduler polling and re-enable across replicas | Disabled site is not claimed; enabled site is claimed without restart; stale lease cannot update removed/paused site | PostgreSQL concurrency |
| SUB-01 | Anonymous subscription request | Generic response; no notifications before confirmation | Integration |
| SUB-02 | Valid, expired, reused or wrong-purpose confirmation | Only valid unused token activates | Integration |
| SUB-03 | Email scanner GETs every mail link | No activation/unsubscribe/state mutation | HTTP |
| SUB-04 | Anonymous request targets existing subscriber | Existing preferences remain unchanged | Integration |
| SUB-05 | All vs selected watches, then add new watch | Only all-mode automatically includes new watch | Integration |
| SUB-06 | Preselected watch, multiple watches, invalid/cross-site slug | Correct preselection; no cross-site enrollment | HTTP/UI |
| SUB-07 | Manage/unsubscribe link | Scope enforced; opt-out honored by pending deliveries | Integration |
| SUB-08 | Subscribe after today's changes | No historical backfill | Integration |
| CHECK-01 | First successful fetch | Silent baseline; no change record | PostgreSQL worker integration / IMPLEMENTED |
| CHECK-02 | Same content hash | No duplicate change record | PostgreSQL worker integration / IMPLEMENTED |
| CHECK-03 | Later content hash differs | One metadata record stores old/new hashes | PostgreSQL worker integration / IMPLEMENTED |
| CHECK-04 | Script/style/noise-only changes | No false update after normalization | Unit |
| CHECK-05 | Timeout, non-2xx, oversize response | Old baseline retained; stable error status recorded | Local HTTP + PostgreSQL / IMPLEMENTED |
| CHECK-07 | Empty selector or selector normalization change | Fail check or silently rebaseline | Worker |
| CHECK-06 | Selector/normalizer configuration changes | Silent rebaseline | Worker |
| JOB-01 | Two replicas claim/create same cycle/watch | Single logical job/result | PostgreSQL concurrency |
| JOB-02 | Worker dies; lease expires | Another worker reclaims safely | PostgreSQL worker integration / IMPLEMENTED |
| JOB-03 | Expired worker later completes | Fencing blocks stale result | PostgreSQL worker integration / IMPLEMENTED |
| JOB-04 | Slow source past cutoff | Other changes delivered; sealed cycle immutable | Worker |
| DIGEST-01 | Several selected pages/sites change | One message per destination/channel, no repeated source | Integration |
| DIGEST-02 | No selected changes or unverified contact | No message | Integration |
| DIGEST-03 | Concurrent digest assemblers/restart | Unique durable outbox row | PostgreSQL concurrency |
| DIGEST-04 | SMTP transient/permanent failure | Bounded retry/backoff or terminal failure; status retained | Integration |
| DIGEST-05 | SMTP accepts then process crashes | Ambiguous result handled/documented; no exactly-once claim | Fault injection |
| DIGEST-06 | Check every content link | All updates point to original pages; no public diff link | Template |
| TIME-01 | DST, restart and overdue cycle | One logical daily cycle per configured local date | Worker |
| SEC-01 | Redirect to private network or DNS rebinding | Fetch denied after re-resolving and validating destination | Integration |
| SEC-02 | Remote HTML/script in label/content | Escaped; no executable output | HTTP/mail |
| SEC-03 | Repeated requests, forged form, cross-origin POST | Rate limit/CSRF controls work | HTTP |
| SEC-04 | Inspect logs | No secrets, recipient addresses or token URLs | Integration |
| CFG-01 | SMTP TLS/auth combinations or invalid production config | Valid modes work; invalid/insecure config rejected explicitly | Config |
| UI-01 | Owner creates/edits/pauses watch and copies links | Correct URLs; paused watch not polled | Browser |
| OPS-02 | SIGTERM/restart, backup/restore | Durable jobs recover; no app PVC dependency | Deployment |

Core tests are in `src/config.rs`, `src/fetcher.rs`, `src/worker.rs`, `src/web.rs`, and `tests/http.rs`. Run
`direnv exec . devbox run check` with local PostgreSQL available; this runs formatting,
Clippy, unit tests and PostgreSQL integration tests. The Devbox and Cargo lockfiles are
committed so the configured toolchain and resolved dependencies stay reproducible.
