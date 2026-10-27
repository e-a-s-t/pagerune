CREATE TABLE monitored_sites (
    id UUID PRIMARY KEY,
    original_url TEXT NOT NULL,
    normalized_url TEXT NOT NULL UNIQUE,
    interval_seconds INTEGER NOT NULL DEFAULT 86400
        CHECK (interval_seconds BETWEEN 300 AND 2592000),
    current_content_hash CHAR(64),
    etag TEXT,
    last_modified TEXT,
    last_checked_at TIMESTAMPTZ,
    last_successful_check_at TIMESTAMPTZ,
    current_status TEXT NOT NULL DEFAULT 'pending'
        CHECK (current_status IN ('pending', 'ok', 'error')),
    last_error TEXT,
    next_check_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    lease_owner UUID,
    lease_expires_at TIMESTAMPTZ,
    lease_generation BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX monitored_sites_due_idx
    ON monitored_sites (next_check_at)
    WHERE current_status <> 'deleted';

CREATE TABLE subscriptions (
    id UUID PRIMARY KEY,
    site_id UUID NOT NULL REFERENCES monitored_sites(id) ON DELETE CASCADE,
    subscriber_identity TEXT NOT NULL CHECK (length(subscriber_identity) BETWEEN 1 AND 256),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (site_id, subscriber_identity)
);

CREATE INDEX subscriptions_identity_idx
    ON subscriptions (subscriber_identity, created_at DESC, id);

CREATE TABLE site_changes (
    id UUID PRIMARY KEY,
    site_id UUID NOT NULL REFERENCES monitored_sites(id) ON DELETE CASCADE,
    previous_content_hash CHAR(64) NOT NULL,
    new_content_hash CHAR(64) NOT NULL,
    detected_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX site_changes_history_idx
    ON site_changes (site_id, detected_at DESC, id DESC);
