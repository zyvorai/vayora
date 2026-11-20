-- Copyright 2026 Zyvor AI Labs · https://zyvor.dev
-- SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

-- A console session key remembers whose it is, so it can be exchanged for the same person's key in the
-- organisation's other environment (test <-> live) without asking for the password again.
ALTER TABLE api_keys ADD COLUMN session_username TEXT;
