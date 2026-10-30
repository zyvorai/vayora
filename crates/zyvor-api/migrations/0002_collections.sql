-- Collections workspace: promises to pay and a log of reminders sent to customers.
CREATE TABLE payment_promises (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    invoice_id TEXT NOT NULL REFERENCES invoices(id),
    promised_minor BIGINT NOT NULL CHECK (promised_minor > 0),
    promised_date DATE NOT NULL, note TEXT, created_by TEXT NOT NULL,
    canceled_at TIMESTAMPTZ, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX payment_promises_invoice ON payment_promises (tenant_id, invoice_id);
CREATE TABLE invoice_reminders (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    invoice_id TEXT NOT NULL REFERENCES invoices(id),
    channel TEXT NOT NULL CHECK (channel IN ('email', 'sms', 'whatsapp', 'call', 'other')),
    message TEXT, sent_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX invoice_reminders_invoice ON invoice_reminders (tenant_id, invoice_id);

DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY['payment_promises', 'invoice_reminders']
  LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format($p$CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting('zyvor.tenant_id', true)) WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true))$p$, t);
  END LOOP;
END $$;
