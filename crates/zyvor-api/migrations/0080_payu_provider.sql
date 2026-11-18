-- PayU provider accounts (hosted checkout, Verify Payment, refunds, settlement reports). Additive: the same
-- constraint as before with one more allowed value.

ALTER TABLE provider_accounts DROP CONSTRAINT IF EXISTS provider_accounts_provider_check;
ALTER TABLE provider_accounts
    ADD CONSTRAINT provider_accounts_provider_check
    CHECK (provider IN ('razorpay', 'cashfree', 'sim', 'custom', 'sponsor_upi', 'phonepe', 'payu'));
