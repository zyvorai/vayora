-- UPI AutoPay mandates: the payer approves once, the merchant debits later, each debit announced in advance.
-- A debit is an ordinary payment attempt (so the ledger, events and invoice paying all work unchanged);
-- these tables hold the agreement and the schedule.
CREATE TABLE mandates (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    customer_id TEXT NOT NULL REFERENCES customers(id),
    currency TEXT NOT NULL,
    -- the most a single debit may take
    max_amount_minor BIGINT NOT NULL CHECK (max_amount_minor > 0),
    frequency TEXT NOT NULL CHECK (frequency IN ('daily', 'weekly', 'fortnightly', 'monthly', 'bimonthly', 'quarterly', 'half_yearly', 'yearly', 'as_presented')),
    start_date DATE NOT NULL, end_date DATE NOT NULL CHECK (end_date > start_date),
    status TEXT NOT NULL DEFAULT 'pending_authorization' CHECK (status IN ('pending_authorization', 'active', 'paused', 'revoked', 'expired', 'failed')),
    provider TEXT NOT NULL, provider_mandate_ref TEXT,
    -- what the payer must do to approve it (for example a upi://mandate link); shown, never edited.
    -- (the column is not called `authorization`: that is a reserved word; the API still says `authorization`)
    auth_request JSONB, failure_reason TEXT, revoked_by TEXT,
    external_ref TEXT, created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, external_ref)
);
ALTER TABLE mandates ENABLE ROW LEVEL SECURITY;
ALTER TABLE mandates FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON mandates
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));

CREATE TABLE mandate_debits (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    mandate_id TEXT NOT NULL REFERENCES mandates(id),
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0), currency TEXT NOT NULL,
    due_date DATE NOT NULL,
    -- the payer is told before the debit: the worker emits the notice at notify_at and records notified_at
    notify_at TIMESTAMPTZ NOT NULL DEFAULT now(), notified_at TIMESTAMPTZ,
    invoice_id TEXT REFERENCES invoices(id),
    state TEXT NOT NULL DEFAULT 'scheduled' CHECK (state IN ('scheduled', 'notified', 'initiating', 'initiated', 'captured', 'failed', 'skipped')),
    attempt_id TEXT, retry_no INT NOT NULL DEFAULT 0, failure_reason TEXT,
    created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- a mandate is never debited twice for the same day
    UNIQUE (mandate_id, due_date)
);
CREATE INDEX mandate_debits_due ON mandate_debits (tenant_id, state, due_date);
ALTER TABLE mandate_debits ENABLE ROW LEVEL SECURITY;
ALTER TABLE mandate_debits FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON mandate_debits
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));

ALTER TABLE payment_attempts ADD COLUMN mandate_id TEXT REFERENCES mandates(id), ADD COLUMN mandate_debit_id TEXT REFERENCES mandate_debits(id);
-- a subscription's invoices are collected through this mandate when it is active
ALTER TABLE subscriptions ADD COLUMN mandate_id TEXT REFERENCES mandates(id);
