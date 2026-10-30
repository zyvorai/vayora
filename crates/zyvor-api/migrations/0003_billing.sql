-- Billing: plans, subscriptions, metered usage, credit notes.
CREATE TABLE plans (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    name TEXT NOT NULL, currency TEXT NOT NULL, amount_minor BIGINT NOT NULL CHECK (amount_minor >= 0),
    interval_unit TEXT NOT NULL CHECK (interval_unit IN ('day', 'week', 'month', 'year')),
    interval_count INT NOT NULL DEFAULT 1 CHECK (interval_count > 0),
    usage_unit_minor BIGINT NOT NULL DEFAULT 0 CHECK (usage_unit_minor >= 0),   -- price per metered unit
    due_days INT NOT NULL DEFAULT 7 CHECK (due_days >= 0),
    active BOOLEAN NOT NULL DEFAULT true, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE subscriptions (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    customer_id TEXT NOT NULL REFERENCES customers(id), plan_id TEXT NOT NULL REFERENCES plans(id),
    quantity INT NOT NULL DEFAULT 1 CHECK (quantity > 0),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'paused', 'canceled')),
    next_bill_date DATE NOT NULL, canceled_at TIMESTAMPTZ, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX subscriptions_due ON subscriptions (tenant_id, status, next_bill_date);
ALTER TABLE invoices ADD COLUMN subscription_id TEXT REFERENCES subscriptions(id), ADD COLUMN period_start DATE;
-- one invoice per subscription period, however many times the billing run executes
CREATE UNIQUE INDEX invoice_one_per_period ON invoices (subscription_id, period_start) WHERE subscription_id IS NOT NULL;
CREATE TABLE usage_records (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    subscription_id TEXT NOT NULL REFERENCES subscriptions(id),
    quantity BIGINT NOT NULL CHECK (quantity > 0), note TEXT,
    idempotency_ref TEXT, billed_invoice_id TEXT REFERENCES invoices(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), UNIQUE (tenant_id, idempotency_ref)
);
CREATE TABLE credit_notes (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    invoice_id TEXT NOT NULL REFERENCES invoices(id),
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0), reason TEXT NOT NULL,
    created_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TRIGGER credit_notes_immutable BEFORE UPDATE OR DELETE ON credit_notes FOR EACH ROW EXECUTE FUNCTION forbid_mutation();

-- Money that has settled an invoice: payments allocated plus credit notes.
CREATE FUNCTION invoice_settled(inv TEXT) RETURNS BIGINT LANGUAGE sql STABLE AS $$
  SELECT (COALESCE((SELECT SUM(amount_minor) FROM invoice_allocations WHERE invoice_id = inv), 0)
        + COALESCE((SELECT SUM(amount_minor) FROM credit_notes WHERE invoice_id = inv), 0))::BIGINT
$$;

DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY['plans', 'subscriptions', 'usage_records', 'credit_notes']
  LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format($p$CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting('zyvor.tenant_id', true)) WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true))$p$, t);
  END LOOP;
END $$;
