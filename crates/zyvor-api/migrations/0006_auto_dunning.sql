-- Opt-in automatic dunning: the worker reminds overdue invoices on the tenant's policy.
ALTER TABLE tenant_settings
    ADD COLUMN auto_dunning BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN dunning_channel TEXT NOT NULL DEFAULT 'email' CHECK (dunning_channel IN ('email', 'sms', 'whatsapp', 'call', 'other')),
    ADD COLUMN dunning_min_days INT NOT NULL DEFAULT 1 CHECK (dunning_min_days >= 0),
    ADD COLUMN dunning_repeat_days INT NOT NULL DEFAULT 7 CHECK (dunning_repeat_days BETWEEN 0 AND 365);
