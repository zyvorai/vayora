-- Counters (tills, desks, stalls) within a branch. A payment link can belong to a counter, so a
-- printed QR at the counter ties every payment made through it to that counter and branch.
CREATE TABLE counters (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    branch_id TEXT NOT NULL REFERENCES branches(id), name TEXT NOT NULL,
    created_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deactivated_at TIMESTAMPTZ,
    UNIQUE (branch_id, name)
);
ALTER TABLE counters ENABLE ROW LEVEL SECURITY;
ALTER TABLE counters FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON counters
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));
ALTER TABLE payment_links ADD COLUMN branch_id TEXT REFERENCES branches(id), ADD COLUMN counter_id TEXT REFERENCES counters(id);
ALTER TABLE payment_intents ADD COLUMN counter_id TEXT REFERENCES counters(id);
CREATE INDEX payment_intents_branch ON payment_intents (tenant_id, branch_id, created_at DESC) WHERE branch_id IS NOT NULL;
