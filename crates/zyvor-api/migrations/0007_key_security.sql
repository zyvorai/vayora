-- API key hardening: optional expiry, optional source-address allow-list, last-used tracking.
ALTER TABLE api_keys
    ADD COLUMN expires_at TIMESTAMPTZ,
    ADD COLUMN last_used_at TIMESTAMPTZ,
    ADD COLUMN allowed_cidrs TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN rotated_from TEXT;
