-- Copyright 2026 Zyvor AI Labs · https://zyvor.dev
-- SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

-- A console session has one start, however many times its key is swapped (test <-> live): the new key never
-- outlives the original session's 12 hours, and the key being swapped out is revoked. NULL on older keys,
-- which fall back to their own creation time.
ALTER TABLE api_keys ADD COLUMN session_started_at TIMESTAMPTZ;
