-- Copyright 2026 Zyvor AI Labs · https://zyvor.dev
-- SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

-- Allow the generic HTTP provider ("bring your own gateway") alongside the hosted ones.
ALTER TABLE provider_accounts DROP CONSTRAINT provider_accounts_provider_check;
ALTER TABLE provider_accounts
    ADD CONSTRAINT provider_accounts_provider_check CHECK (provider IN ('razorpay', 'cashfree', 'sim', 'custom'));
