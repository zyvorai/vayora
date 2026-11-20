-- Copyright 2026 Zyvor AI Labs · https://zyvor.dev
-- SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

-- A test organisation's own clock: when set, billing, reminders and dates the API stamps follow it instead of the
-- calendar. Only a test environment can set it (checked in the handler); null means real time.
ALTER TABLE tenant_settings ADD COLUMN clock_date DATE;
