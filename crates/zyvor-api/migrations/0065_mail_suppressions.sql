-- Emails a customer asked to stop receiving from a business (the unsubscribe link in automatic mail). Automatic
-- mail (invoices raised by billing, reminders) to these addresses is skipped; mail a person sends by hand is not.
CREATE TABLE mail_suppressions (
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    email TEXT NOT NULL CHECK (length(email) <= 320),
    source TEXT NOT NULL DEFAULT 'unsubscribe',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, email)
);
ALTER TABLE mail_suppressions ENABLE ROW LEVEL SECURITY;
ALTER TABLE mail_suppressions FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON mail_suppressions
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));

-- The queue now knows whose mail it is and whether it is automatic (so it may carry an unsubscribe link and be skipped
-- for an address that unsubscribed).
ALTER TABLE mail_queue ADD COLUMN tenant_id TEXT, ADD COLUMN automatic BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE mail_queue DROP CONSTRAINT mail_queue_status_check;
ALTER TABLE mail_queue ADD CONSTRAINT mail_queue_status_check CHECK (status IN ('pending', 'sent', 'dead', 'suppressed'));
