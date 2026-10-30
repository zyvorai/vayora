-- Instalment plans: one agreed total split into dated invoices (term fees, treatment plans, fit-outs).
CREATE TABLE installment_plans (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    customer_id TEXT NOT NULL REFERENCES customers(id), name TEXT NOT NULL,
    total_minor BIGINT NOT NULL CHECK (total_minor > 0), currency TEXT NOT NULL, instalments INT NOT NULL CHECK (instalments BETWEEN 2 AND 60),
    external_ref TEXT, created_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, external_ref)
);
ALTER TABLE installment_plans ENABLE ROW LEVEL SECURITY;
ALTER TABLE installment_plans FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON installment_plans
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));
ALTER TABLE invoices ADD COLUMN installment_plan_id TEXT REFERENCES installment_plans(id), ADD COLUMN installment_no INT;
