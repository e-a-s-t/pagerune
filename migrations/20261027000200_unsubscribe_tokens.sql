CREATE TABLE unsubscribe_tokens (
    token_hash CHAR(64) PRIMARY KEY,
    subscription_id UUID NOT NULL REFERENCES subscriptions(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at TIMESTAMPTZ
);

CREATE INDEX unsubscribe_tokens_subscription_idx ON unsubscribe_tokens(subscription_id);
