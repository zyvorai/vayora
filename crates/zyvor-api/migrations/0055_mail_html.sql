-- Copyright 2026 Zyvor AI Labs · https://zyvor.dev
-- SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

-- An HTML version of a queued email (payer-facing mail only); the plain-text body is always kept.
ALTER TABLE mail_queue ADD COLUMN html TEXT;
