-- Copyright 2026 Zyvor AI Labs · https://zyvor.dev
-- SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

-- An uploaded logo for payer emails, kept in the database and served from the API. The version is part of the
-- address, so replacing the logo gives mail programs a new address to fetch.
ALTER TABLE tenant_settings
    ADD COLUMN email_logo_data BYTEA,
    ADD COLUMN email_logo_type TEXT,
    ADD COLUMN email_logo_version INT NOT NULL DEFAULT 0;
