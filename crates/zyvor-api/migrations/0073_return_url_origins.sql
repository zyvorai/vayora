-- Per-tenant allowlist of return_url origins (https://shop.example.com). Empty = any https URL still passes SSRF checks.
ALTER TABLE tenant_settings
    ADD COLUMN IF NOT EXISTS return_url_origins JSONB NOT NULL DEFAULT '[]'::jsonb;
