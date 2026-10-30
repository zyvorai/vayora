-- Milestone billing: an amount owed when a piece of work is accepted (factory stage, project phase).
CREATE TABLE milestones (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    customer_id TEXT NOT NULL REFERENCES customers(id), name TEXT NOT NULL,
    currency TEXT NOT NULL, amount_minor BIGINT NOT NULL CHECK (amount_minor > 0),
    due_days INT NOT NULL DEFAULT 7 CHECK (due_days >= 0), external_ref TEXT,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'invoiced', 'canceled')),
    invoice_id TEXT REFERENCES invoices(id), completed_by TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
ALTER TABLE milestones ENABLE ROW LEVEL SECURITY;
ALTER TABLE milestones FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON milestones USING (tenant_id = current_setting('zyvor.tenant_id', true)) WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));
