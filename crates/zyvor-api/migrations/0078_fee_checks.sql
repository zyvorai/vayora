-- Fee checks: what a provider actually deducted from a settlement, against the merchant's own contract.
-- Rates are data entered by the merchant from their own contract. Nothing here ships a default rate.
-- Findings only explain and ask a person to review; nothing in these tables moves money or touches the ledger.

-- One row per (provider, method, effective_from). The rule in force for a payment is the one with the
-- latest effective_from on or before the payment's date, so older rules stay in place for older payments.
CREATE TABLE fee_rules (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    provider TEXT NOT NULL,
    method TEXT NOT NULL,
    bps INT NOT NULL CHECK (bps >= 0 AND bps <= 10000),              -- percentage in basis points
    flat_minor BIGINT NOT NULL DEFAULT 0 CHECK (flat_minor >= 0),    -- flat paise per payment
    cap_minor BIGINT CHECK (cap_minor IS NULL OR cap_minor >= 0),    -- most the fee (percentage + flat, before GST) can be per payment
    gst_bps INT CHECK (gst_bps IS NULL OR (gst_bps >= 0 AND gst_bps <= 10000)),  -- GST charged on the fee; NULL = none
    refund_returns_fee BOOLEAN NOT NULL DEFAULT false,               -- the provider gives the fee back on refunded value
    effective_from DATE NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, provider, method, effective_from)
);

-- Per-tenant settings for the check: rounding tolerance (paise, per payment line) and who owns new findings.
CREATE TABLE fee_check_settings (
    tenant_id TEXT PRIMARY KEY REFERENCES tenants(id),
    tolerance_minor BIGINT NOT NULL DEFAULT 0 CHECK (tolerance_minor >= 0),
    owner TEXT,
    updated_by TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The result for one payment (attempt) inside one settlement. The rule that was applied is copied in, so the
-- record stays true even if the rule is later removed.
CREATE TABLE fee_check_lines (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    settlement_id TEXT NOT NULL REFERENCES settlements(id),
    attempt_id TEXT NOT NULL REFERENCES payment_attempts(id),
    provider TEXT NOT NULL,
    method TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('ok', 'flagged', 'no_rule')),
    payment_minor BIGINT NOT NULL,
    refunded_minor BIGINT NOT NULL DEFAULT 0,
    actual_minor BIGINT NOT NULL,            -- deducted (positive) or returned (negative), net, in this settlement
    expected_minor BIGINT,                   -- NULL when no rule applied
    variance_minor BIGINT,                   -- actual - expected: positive = provider deducted more than the rule says
    tolerance_minor BIGINT NOT NULL DEFAULT 0,
    rule_id TEXT,
    rule JSONB,                              -- the rule exactly as applied
    case_id TEXT,
    checked_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, settlement_id, attempt_id)
);
CREATE INDEX fee_check_lines_flagged_idx ON fee_check_lines (tenant_id, status);

DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY['fee_rules', 'fee_check_settings', 'fee_check_lines']
  LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format('DROP POLICY IF EXISTS tenant_isolation ON %I', t);
    EXECUTE format($p$CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting('zyvor.tenant_id', true)) WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true))$p$, t);
  END LOOP;
END $$;
