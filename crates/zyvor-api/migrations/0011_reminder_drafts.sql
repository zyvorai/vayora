-- Reminder drafts: a person reviews (and may edit) a reminder before it is sent. Drafts come from the
-- dunning run when the tenant requires approval, or on request (optionally written by the AI assistant).
ALTER TABLE tenant_settings ADD COLUMN dunning_requires_approval BOOLEAN NOT NULL DEFAULT false;

CREATE TABLE reminder_drafts (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    invoice_id TEXT NOT NULL REFERENCES invoices(id),
    channel TEXT NOT NULL CHECK (channel IN ('email', 'sms', 'whatsapp', 'call', 'other')),
    message TEXT NOT NULL, source TEXT NOT NULL CHECK (source IN ('template', 'ai')),
    ai_requested BOOLEAN NOT NULL DEFAULT false,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'rejected')),
    created_by TEXT NOT NULL, decided_by TEXT, decided_at TIMESTAMPTZ, reminder_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX reminder_drafts_tenant ON reminder_drafts (tenant_id, status, created_at DESC);
CREATE UNIQUE INDEX reminder_drafts_one_pending ON reminder_drafts (invoice_id) WHERE status = 'pending';
ALTER TABLE reminder_drafts ENABLE ROW LEVEL SECURITY;
ALTER TABLE reminder_drafts FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON reminder_drafts
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));
