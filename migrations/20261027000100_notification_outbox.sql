CREATE TABLE notification_outbox (
    id UUID PRIMARY KEY,
    change_id UUID NOT NULL REFERENCES site_changes(id) ON DELETE CASCADE,
    subscription_id UUID NOT NULL REFERENCES subscriptions(id) ON DELETE CASCADE,
    recipient TEXT NOT NULL CHECK (length(recipient) BETWEEN 1 AND 320),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'sending', 'delivered', 'failed')),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    lease_owner UUID,
    lease_expires_at TIMESTAMPTZ,
    lease_generation BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    delivered_at TIMESTAMPTZ,
    last_error TEXT,
    UNIQUE (change_id, subscription_id)
);

CREATE INDEX notification_outbox_due_idx
    ON notification_outbox (next_attempt_at, created_at, id)
    WHERE status IN ('pending', 'sending');
