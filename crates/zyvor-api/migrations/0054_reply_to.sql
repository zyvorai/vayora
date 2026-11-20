-- Copyright 2026 Zyvor AI Labs · https://zyvor.dev
-- SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

-- A business's own address for replies to the emails sent for it (receipts, refund notices, payment links).
ALTER TABLE tenant_settings ADD COLUMN reply_to_email TEXT CHECK (reply_to_email IS NULL OR length(reply_to_email) <= 200);
ALTER TABLE mail_queue ADD COLUMN reply_to TEXT;
