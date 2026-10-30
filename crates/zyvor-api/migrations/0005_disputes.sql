-- Disputes (chargebacks). An open dispute reserves the disputed amount on the payment so it cannot
-- also be refunded; a lost dispute converts the reservation into a loss on the ledger.
CREATE TABLE disputes (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    attempt_id TEXT NOT NULL REFERENCES payment_attempts(id), intent_id TEXT NOT NULL REFERENCES payment_intents(id),
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0), currency TEXT NOT NULL,
    reason TEXT NOT NULL, provider_ref TEXT, respond_by DATE,
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'under_review', 'won', 'lost')),
    evidence JSONB NOT NULL DEFAULT '[]', opened_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), resolved_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX one_active_dispute_per_attempt ON disputes (attempt_id) WHERE status IN ('open', 'under_review');
ALTER TABLE disputes ENABLE ROW LEVEL SECURITY;
ALTER TABLE disputes FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON disputes USING (tenant_id = current_setting('zyvor.tenant_id', true)) WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));
