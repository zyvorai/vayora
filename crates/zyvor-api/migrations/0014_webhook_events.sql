-- Copyright 2026 Zyvor AI Labs · https://zyvor.dev
-- SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

-- Which event types the merchant's webhook endpoint wants. NULL means all of them.
ALTER TABLE tenant_settings ADD COLUMN webhook_events TEXT[];
