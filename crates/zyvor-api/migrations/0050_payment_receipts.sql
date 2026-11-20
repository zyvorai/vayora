-- Copyright 2026 Zyvor AI Labs · https://zyvor.dev
-- SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

-- Payment receipt emails: opt-in per tenant. A receipt is queued in the same transaction as the capture, with a
-- key that makes a repeated capture queue nothing a second time.
ALTER TABLE tenant_settings ADD COLUMN payment_receipts BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE mail_queue ADD COLUMN dedupe_key TEXT UNIQUE;
