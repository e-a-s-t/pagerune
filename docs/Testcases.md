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
| MON-02 | Failed fetch after baseline | Baseline remains unchanged and a stable error is recorded | PostgreSQL worker integration / IMPLEMENTED |
| SEC-05 | Private/loopback/metadata destination | Fetch is rejected before connecting | Unit + local HTTP / IMPLEMENTED |
| API-02 | Malformed JSON and disabled business API | Structured JSON error with request ID | HTTP integration / IMPLEMENTED |
| OWN-01 | Site ownership not verified | Cannot activate watches or publish subscription form | Integration |
| TEN-01 | Tenant A requests B site/watch IDs | No disclosure or mutation; composite FK prevents cross-link | Integration |
| TEN-02 | Same person subscribes to two tenants | Single daily digest; owners see only their own contact scope | Integration |
| AUTH-01 | Wrong issuer/audience, expired/forged token or ID token | Admin API rejects request | Integration |
| AUTH-02 | Valid token without tenant membership | Access denied to tenant resources | Integration |
| AUTH-03 | OIDC callback wrong state/nonce; session on another replica | Invalid callback rejected; valid session portable | Integration |
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

Core tests are in `src/config.rs`, `src/fetcher.rs`, `src/worker.rs`, and `tests/http.rs`. Run
`direnv exec . devbox run check` with local PostgreSQL available; this runs formatting,
Clippy, unit tests and PostgreSQL integration tests. The Devbox and Cargo lockfiles are
committed so the configured toolchain and resolved dependencies stay reproducible.
