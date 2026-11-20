-- Copyright 2026 Zyvor AI Labs · https://zyvor.dev
-- SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

-- Whose fault was a decline? Routing health must not count a payer's insufficient funds against a provider.
--
-- failure_class (set only on a failed attempt) now distinguishes:
--   payer_declined    the payer's doing (insufficient funds, the payer left or refused). Visible in routing health but
--                     NOT counted in the success rate or the circuit breaker, and not a reason to fail over
--   provider_fault    the provider or the bank side failed (for example: the provider could not be asked and later said
--                     the payment failed). Counts against the provider
--   declined          the provider said it failed and the cause was not recognised ("unknown"). Counts against the
--                     provider, the conservative choice. Every row written before this migration is this class
--   provider_rejected the provider refused the request before creating anything. Counts against the provider
--   cancelled         the merchant cancelled it. Never counted
-- Additive: existing rows keep their class. No new table, so no new row-level-security policy is needed.
DO $$
DECLARE c TEXT;
BEGIN
  FOR c IN
    SELECT con.conname FROM pg_constraint con
    JOIN pg_attribute att ON att.attrelid = con.conrelid AND att.attnum = ANY (con.conkey)
    WHERE con.conrelid = 'payment_attempts'::regclass AND con.contype = 'c' AND att.attname = 'failure_class'
  LOOP
    EXECUTE format('ALTER TABLE payment_attempts DROP CONSTRAINT %I', c);
  END LOOP;
END $$;
ALTER TABLE payment_attempts ADD CONSTRAINT payment_attempts_failure_class_check
    CHECK (failure_class IS NULL OR failure_class IN ('declined', 'provider_rejected', 'cancelled', 'payer_declined', 'provider_fault'));
