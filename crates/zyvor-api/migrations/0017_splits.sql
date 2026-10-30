-- Marketplace splits: a payment can carry fixed shares for vendors. On capture each share accrues to
-- the vendor's payable balance; refunds reverse the shares in proportion; payouts can draw on the balance.
CREATE TABLE payment_splits (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    intent_id TEXT NOT NULL REFERENCES payment_intents(id), vendor_id TEXT NOT NULL REFERENCES vendors(id),
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0),
    accrued BOOLEAN NOT NULL DEFAULT false, reversed_minor BIGINT NOT NULL DEFAULT 0 CHECK (reversed_minor >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (intent_id, vendor_id)
);
CREATE INDEX payment_splits_vendor ON payment_splits (tenant_id, vendor_id);
ALTER TABLE payment_splits ENABLE ROW LEVEL SECURITY;
ALTER TABLE payment_splits FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON payment_splits
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));

-- payment links can carry the same shares into every intent they mint
ALTER TABLE payment_links ADD COLUMN splits JSONB;
-- where a payout's money comes from: the vendor's accrued balance, or your own funds as before
ALTER TABLE payouts ADD COLUMN source TEXT NOT NULL DEFAULT 'own_funds' CHECK (source IN ('own_funds', 'balance'));
