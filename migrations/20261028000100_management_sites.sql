ALTER TABLE monitored_sites
    ADD COLUMN name TEXT,
    ADD COLUMN monitoring_enabled BOOLEAN NOT NULL DEFAULT TRUE;

UPDATE monitored_sites
SET name = COALESCE(substring(normalized_url FROM '^https?://([^/?#]+)'), original_url);

ALTER TABLE monitored_sites
    ALTER COLUMN name SET NOT NULL,
    ALTER COLUMN name SET DEFAULT 'Website',
    ADD CONSTRAINT monitored_sites_name_length CHECK (length(btrim(name)) BETWEEN 1 AND 200);

ALTER TABLE web_form_nonces
    DROP CONSTRAINT web_form_nonces_purpose_check,
    ADD CONSTRAINT web_form_nonces_purpose_check
        CHECK (purpose IN ('subscribe', 'verify', 'manage_request', 'manage_exchange', 'management_create', 'management_edit', 'management_delete'));
