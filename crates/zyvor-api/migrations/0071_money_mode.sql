-- Dual money modes: orchestrate (default) vs acquire (custody under sponsor / PA).
-- Also allow sponsor_upi and phonepe provider accounts.

ALTER TABLE organisations
    ADD COLUMN IF NOT EXISTS money_mode TEXT NOT NULL DEFAULT 'orchestrate'
        CHECK (money_mode IN ('orchestrate', 'acquire'));

ALTER TABLE tenant_settings
    ADD COLUMN IF NOT EXISTS settlement_delay_days INT NOT NULL DEFAULT 1
        CHECK (settlement_delay_days >= 0 AND settlement_delay_days <= 7);

ALTER TABLE provider_accounts DROP CONSTRAINT IF EXISTS provider_accounts_provider_check;
ALTER TABLE provider_accounts
    ADD CONSTRAINT provider_accounts_provider_check
    CHECK (provider IN ('razorpay', 'cashfree', 'sim', 'custom', 'sponsor_upi', 'phonepe'));

-- Settlement batches for acquire mode (zyvor-settle).
CREATE TABLE IF NOT EXISTS settlement_batches (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    currency TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open'
        CHECK (status IN ('open', 'releasing', 'released', 'failed')),
    captures_minor BIGINT NOT NULL DEFAULT 0,
    refunds_minor BIGINT NOT NULL DEFAULT 0,
    fees_minor BIGINT NOT NULL DEFAULT 0,
    holds_minor BIGINT NOT NULL DEFAULT 0,
    net_minor BIGINT NOT NULL DEFAULT 0,
    rail TEXT,
    rail_ref TEXT,
    failure_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    released_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS settlement_batches_tenant_idx ON settlement_batches (tenant_id, created_at DESC);

CREATE TABLE IF NOT EXISTS settlement_batch_items (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    batch_id TEXT NOT NULL REFERENCES settlement_batches(id),
    kind TEXT NOT NULL CHECK (kind IN ('capture', 'refund', 'fee', 'hold')),
    reference TEXT NOT NULL,
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0),
    currency TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, reference, kind)
);

-- Network card tokens (zyvor-cards): never store PAN/CVV.
CREATE TABLE IF NOT EXISTS card_tokens (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    customer_id TEXT REFERENCES customers(id),
    network_token_ref TEXT NOT NULL,
    last4 TEXT NOT NULL CHECK (char_length(last4) = 4),
    brand TEXT,
    exp_month INT CHECK (exp_month IS NULL OR (exp_month BETWEEN 1 AND 12)),
    exp_year INT CHECK (exp_year IS NULL OR exp_year BETWEEN 2020 AND 2100),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at TIMESTAMPTZ,
    UNIQUE (tenant_id, network_token_ref)
);

-- Acquire-mode KYC / AML gate.
ALTER TABLE organisations
    ADD COLUMN IF NOT EXISTS acquire_kyc_status TEXT NOT NULL DEFAULT 'not_started'
        CHECK (acquire_kyc_status IN ('not_started', 'pending', 'approved', 'rejected', 're_kyc_due'));

ALTER TABLE organisations
    ADD COLUMN IF NOT EXISTS acquire_kyc_approved_at TIMESTAMPTZ;

-- Risk scoring (parity): optional score threshold alongside hard rules.
ALTER TABLE tenant_settings
    ADD COLUMN IF NOT EXISTS risk_score_block_below INT
        CHECK (risk_score_block_below IS NULL OR (risk_score_block_below >= 0 AND risk_score_block_below <= 100));
