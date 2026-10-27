ALTER TABLE monitored_sites
    ADD COLUMN public_subscriptions_enabled BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE subscriptions
    ADD COLUMN status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'active'));

CREATE INDEX subscriptions_active_site_idx
    ON subscriptions(site_id, created_at DESC)
    WHERE status = 'active';

CREATE UNIQUE INDEX subscriptions_site_identity_unique
    ON subscriptions(site_id, lower(btrim(subscriber_identity)));

CREATE TABLE verification_tokens (
    token_id UUID PRIMARY KEY,
    token_hash CHAR(64) NOT NULL UNIQUE,
    subscription_id UUID NOT NULL REFERENCES subscriptions(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX verification_tokens_subscription_idx
    ON verification_tokens(subscription_id, created_at DESC);

CREATE TABLE management_tokens (
    token_id UUID PRIMARY KEY,
    token_hash CHAR(64) NOT NULL UNIQUE,
    subscriber_identity TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE management_sessions (
    session_hash CHAR(64) PRIMARY KEY,
    subscriber_identity TEXT NOT NULL,
    csrf_hash CHAR(64) NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX management_sessions_expiry_idx ON management_sessions(expires_at);

CREATE TABLE web_form_nonces (
    token_hash CHAR(64) PRIMARY KEY,
    purpose TEXT NOT NULL CHECK (purpose IN ('subscribe', 'verify', 'manage_request', 'manage_exchange')),
    site_id UUID REFERENCES monitored_sites(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX web_form_nonces_expiry_idx ON web_form_nonces(expires_at);

CREATE TABLE email_action_outbox (
    id UUID PRIMARY KEY,
    action TEXT NOT NULL CHECK (action IN ('verify', 'manage')),
    recipient TEXT NOT NULL CHECK (length(recipient) BETWEEN 3 AND 320),
    token_id UUID NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'sending', 'delivered', 'failed')),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    lease_owner UUID,
    lease_expires_at TIMESTAMPTZ,
    lease_generation BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    delivered_at TIMESTAMPTZ,
    last_error TEXT
);

CREATE INDEX email_action_outbox_due_idx
    ON email_action_outbox(next_attempt_at, created_at, id)
    WHERE status IN ('pending', 'sending');

CREATE TABLE web_rate_limits (
    rate_key CHAR(64) NOT NULL,
    bucket_start TIMESTAMPTZ NOT NULL,
    request_count INTEGER NOT NULL DEFAULT 1,
    expires_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY(rate_key, bucket_start)
);

CREATE INDEX web_rate_limits_expiry_idx ON web_rate_limits(expires_at);
