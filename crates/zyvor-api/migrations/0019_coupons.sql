-- Coupons (percent or flat discounts redeemed on payment links) and an optional convenience fee that
-- the payer covers on a fixed-amount link.
CREATE TABLE coupons (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    code TEXT NOT NULL, kind TEXT NOT NULL CHECK (kind IN ('percent', 'flat')),
    -- percent: basis points (1000 = 10%); flat: minor units
    value BIGINT NOT NULL CHECK (value > 0),
    max_uses INT CHECK (max_uses > 0), uses INT NOT NULL DEFAULT 0,
    expires_at TIMESTAMPTZ, deactivated_at TIMESTAMPTZ,
    created_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, code),
    CHECK (kind <> 'percent' OR value <= 10000)
);
ALTER TABLE coupons ENABLE ROW LEVEL SECURITY;
ALTER TABLE coupons FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON coupons
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));
ALTER TABLE payment_links ADD COLUMN convenience_fee_bp INT NOT NULL DEFAULT 0 CHECK (convenience_fee_bp BETWEEN 0 AND 1000);
