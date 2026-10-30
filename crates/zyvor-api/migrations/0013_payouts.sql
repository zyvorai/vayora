-- Payouts: money the merchant sends to vendors. Zyvor controls and records it (maker-checker approval,
-- bank reference, ledger posting); the transfer itself is made from the merchant's own bank.
ALTER TABLE tenant_settings ADD COLUMN payout_approval_threshold_minor BIGINT NOT NULL DEFAULT 0 CHECK (payout_approval_threshold_minor >= 0);

CREATE TABLE vendors (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    name TEXT NOT NULL, external_ref TEXT,
    -- shown in lists; the full details are encrypted and revealed only to people who may approve payouts
    account_last4 TEXT, ifsc TEXT, vpa_masked TEXT, details_enc BYTEA,
    archived_at TIMESTAMPTZ, created_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, external_ref)
);
CREATE TABLE payouts (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    vendor_id TEXT NOT NULL REFERENCES vendors(id),
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0), currency TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending_approval', 'approved', 'paid', 'rejected', 'canceled')),
    purpose TEXT NOT NULL, external_ref TEXT, requested_by TEXT NOT NULL, approved_by TEXT, paid_by TEXT,
    bank_ref TEXT, created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, external_ref), UNIQUE (tenant_id, bank_ref)
);
CREATE INDEX payouts_status ON payouts (tenant_id, status, created_at DESC);
DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY['vendors', 'payouts']
  LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format($p$CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting('zyvor.tenant_id', true)) WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true))$p$, t);
  END LOOP;
END $$;
