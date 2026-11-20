-- Copyright 2026 Zyvor AI Labs · https://zyvor.dev
-- SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

-- A subscription may keep an earlier price after its plan's price changes ("grandfathering"). NULL means it follows
-- the plan's current price. Moving to another plan clears it.
ALTER TABLE subscriptions ADD COLUMN unit_amount_minor BIGINT CHECK (unit_amount_minor IS NULL OR unit_amount_minor >= 0);
