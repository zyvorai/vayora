-- Copyright 2026 Zyvor AI Labs · https://zyvor.dev
-- SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

-- Per-tenant allowlist of return_url origins (https://shop.example.com). Empty = any https URL still passes SSRF checks.
ALTER TABLE tenant_settings
    ADD COLUMN IF NOT EXISTS return_url_origins JSONB NOT NULL DEFAULT '[]'::jsonb;
