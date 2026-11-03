-- Opt-in: email each subscription invoice to its customer when billing raises it (off by default).
ALTER TABLE tenant_settings ADD COLUMN invoice_emails BOOLEAN NOT NULL DEFAULT FALSE;
