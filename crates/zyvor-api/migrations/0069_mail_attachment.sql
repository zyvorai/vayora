-- Copyright 2026 Zyvor AI Labs · https://zyvor.dev
-- SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

-- A queued email can carry one attachment (an invoice's PDF). Cleared with the body once the message is sent.
ALTER TABLE mail_queue ADD COLUMN attachment BYTEA, ADD COLUMN attachment_name TEXT;
