-- Payment links: a reusable URL that mints a fresh payment intent each time someone opens it.
CREATE TABLE payment_links (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    token TEXT NOT NULL UNIQUE,
    description TEXT, amount_minor BIGINT CHECK (amount_minor > 0), currency TEXT NOT NULL DEFAULT 'INR',
    invoice_id TEXT REFERENCES invoices(id), customer_id TEXT REFERENCES customers(id),
    max_uses INT CHECK (max_uses > 0), uses INT NOT NULL DEFAULT 0,
    expires_at TIMESTAMPTZ, canceled_at TIMESTAMPTZ,
    created_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- a link is either a fixed amount or "whatever is still owed on this invoice"
    CHECK ((amount_minor IS NOT NULL) <> (invoice_id IS NOT NULL))
);
CREATE INDEX payment_links_tenant ON payment_links (tenant_id, created_at DESC);
ALTER TABLE payment_links ENABLE ROW LEVEL SECURITY;
ALTER TABLE payment_links FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON payment_links
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));
