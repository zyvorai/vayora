-- Zyvor Payments core schema. Every tenant-scoped table carries tenant_id and has row-level
-- security keyed on the session setting `zyvor.tenant_id` (set per transaction by the API).
-- NOTE: RLS only binds non-superuser roles; production must connect as one (see deploy/README).

-- ---------- organisations, tenants (test/live), people, keys ----------
CREATE TABLE organisations (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE tenants (
    id TEXT PRIMARY KEY,
    organisation_id TEXT NOT NULL REFERENCES organisations(id),
    env TEXT NOT NULL CHECK (env IN ('test', 'live')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (organisation_id, env)
);
CREATE TABLE branches (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    name TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE users (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    email TEXT NOT NULL, role TEXT NOT NULL CHECK (role IN ('owner', 'finance', 'support', 'developer')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), UNIQUE (tenant_id, email)
);
-- Looked up by hash before the tenant is known, so no RLS here. Only the hash is stored.
CREATE TABLE api_keys (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    name TEXT NOT NULL, prefix TEXT NOT NULL, key_hash TEXT NOT NULL UNIQUE,
    scopes TEXT[] NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(), revoked_at TIMESTAMPTZ
);
CREATE TABLE tenant_settings (
    tenant_id TEXT PRIMARY KEY REFERENCES tenants(id),
    -- refunds above this need a second person's approval
    refund_approval_threshold_minor BIGINT NOT NULL DEFAULT 5000000,
    routing JSONB NOT NULL DEFAULT '{"rules": [], "fees": {}}',
    outbound_webhook_url TEXT,
    outbound_webhook_secret_enc BYTEA
);
CREATE TABLE audit_events (
    id BIGSERIAL PRIMARY KEY, tenant_id TEXT NOT NULL, actor TEXT NOT NULL, action TEXT NOT NULL,
    subject TEXT, detail JSONB NOT NULL DEFAULT '{}', created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ---------- providers ----------
CREATE TABLE provider_accounts (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    provider TEXT NOT NULL CHECK (provider IN ('razorpay', 'cashfree', 'sim')),
    label TEXT NOT NULL DEFAULT '', enabled BOOLEAN NOT NULL DEFAULT TRUE,
    base_url TEXT,                     -- sandbox/mock override
    credentials_enc BYTEA NOT NULL,    -- AES-256-GCM: nonce || ciphertext
    webhook_secret_enc BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, provider)
);

-- ---------- business objects ----------
CREATE TABLE customers (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    name TEXT NOT NULL, email TEXT, phone TEXT, external_ref TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE invoices (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    customer_id TEXT REFERENCES customers(id), number TEXT NOT NULL,
    currency TEXT NOT NULL, amount_minor BIGINT NOT NULL CHECK (amount_minor > 0),
    -- tax is calculated by the billing/GST product; we only carry the figure and its reference
    tax_minor BIGINT NOT NULL DEFAULT 0, external_ref TEXT,
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'partially_paid', 'paid', 'void')),
    due_date DATE, created_at TIMESTAMPTZ NOT NULL DEFAULT now(), UNIQUE (tenant_id, number)
);
CREATE TABLE payment_intents (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    customer_id TEXT REFERENCES customers(id), invoice_id TEXT REFERENCES invoices(id), branch_id TEXT REFERENCES branches(id),
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0), currency TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('requires_payment_method', 'processing', 'succeeded', 'failed', 'canceled')),
    capture_method TEXT NOT NULL DEFAULT 'automatic' CHECK (capture_method IN ('automatic', 'manual')),
    allowed_providers TEXT[],
    client_secret_hash TEXT,           -- lets a browser check out this one intent without an API key
    -- the business object this payment belongs to: student fee, patient deposit, sales order...
    external_ref TEXT, description TEXT, metadata JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE payment_attempts (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    intent_id TEXT NOT NULL REFERENCES payment_intents(id),
    provider TEXT NOT NULL, method TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('created', 'pending', 'authorized', 'captured', 'failed', 'unknown')),
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0), currency TEXT NOT NULL,
    provider_order_ref TEXT, provider_payment_ref TEXT, failure_reason TEXT,
    route JSONB NOT NULL,              -- why this provider was chosen
    client_action JSONB,               -- what the client must do next
    raw JSONB NOT NULL DEFAULT '{}',   -- provider responses, kept verbatim for traceability
    refunded_minor BIGINT NOT NULL DEFAULT 0,
    reserved_refund_minor BIGINT NOT NULL DEFAULT 0,   -- refunds requested but not finished
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (refunded_minor >= 0 AND reserved_refund_minor >= 0 AND refunded_minor + reserved_refund_minor <= amount_minor)
);
CREATE INDEX payment_attempts_intent ON payment_attempts (tenant_id, intent_id);
CREATE UNIQUE INDEX payment_attempts_provider_ref ON payment_attempts (tenant_id, provider, provider_payment_ref) WHERE provider_payment_ref IS NOT NULL;
-- A single intent can be paid at most once.
CREATE UNIQUE INDEX one_capture_per_intent ON payment_attempts (intent_id) WHERE status = 'captured';
CREATE TABLE refunds (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    attempt_id TEXT NOT NULL REFERENCES payment_attempts(id), intent_id TEXT NOT NULL REFERENCES payment_intents(id),
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0), currency TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending_approval', 'processing', 'processed', 'failed', 'rejected', 'unknown')),
    reason TEXT, external_ref TEXT,    -- e.g. the pharmacy return id
    requested_by TEXT NOT NULL, approved_by TEXT,
    provider_refund_ref TEXT, failure_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE invoice_allocations (
    id BIGSERIAL PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    invoice_id TEXT NOT NULL REFERENCES invoices(id), intent_id TEXT NOT NULL REFERENCES payment_intents(id),
    -- negative rows are reversals (refunds); allocations are never edited
    amount_minor BIGINT NOT NULL, reference TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, reference)
);

-- ---------- ledger ----------
CREATE TABLE ledger_transactions (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    reference TEXT NOT NULL, reverses TEXT REFERENCES ledger_transactions(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), UNIQUE (tenant_id, reference)
);
CREATE TABLE ledger_entries (
    id BIGSERIAL PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    transaction_id TEXT NOT NULL REFERENCES ledger_transactions(id),
    account TEXT NOT NULL, side TEXT NOT NULL CHECK (side IN ('debit', 'credit')),
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0), currency TEXT NOT NULL
);
CREATE INDEX ledger_entries_account ON ledger_entries (tenant_id, account, currency);

CREATE FUNCTION forbid_mutation() RETURNS trigger AS $$
BEGIN RAISE EXCEPTION '% is append-only', TG_TABLE_NAME; END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER ledger_entries_immutable BEFORE UPDATE OR DELETE ON ledger_entries FOR EACH ROW EXECUTE FUNCTION forbid_mutation();
CREATE TRIGGER ledger_transactions_immutable BEFORE UPDATE OR DELETE ON ledger_transactions FOR EACH ROW EXECUTE FUNCTION forbid_mutation();
CREATE TRIGGER audit_events_immutable BEFORE UPDATE OR DELETE ON audit_events FOR EACH ROW EXECUTE FUNCTION forbid_mutation();
CREATE TRIGGER invoice_allocations_immutable BEFORE UPDATE OR DELETE ON invoice_allocations FOR EACH ROW EXECUTE FUNCTION forbid_mutation();

-- The database refuses to commit a transaction whose debits and credits differ in any currency.
CREATE FUNCTION assert_balanced() RETURNS trigger AS $$
DECLARE bad RECORD;
BEGIN
  SELECT currency INTO bad FROM ledger_entries WHERE transaction_id = NEW.transaction_id
    GROUP BY currency
    HAVING SUM(amount_minor) FILTER (WHERE side = 'debit') IS DISTINCT FROM SUM(amount_minor) FILTER (WHERE side = 'credit')
    LIMIT 1;
  IF FOUND THEN RAISE EXCEPTION 'ledger transaction % does not balance in %', NEW.transaction_id, bad.currency; END IF;
  RETURN NULL;
END; $$ LANGUAGE plpgsql;
CREATE CONSTRAINT TRIGGER ledger_entries_balanced AFTER INSERT ON ledger_entries
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION assert_balanced();

-- ---------- idempotency, inbox, outbox ----------
CREATE TABLE idempotency_keys (
    tenant_id TEXT NOT NULL REFERENCES tenants(id), scope TEXT NOT NULL, key TEXT NOT NULL,
    request_hash TEXT NOT NULL, response_status INT, response_body JSONB, resource_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), PRIMARY KEY (tenant_id, scope, key)
);
-- Provider webhooks: de-duplicated on (provider, event id) before anything is processed.
CREATE TABLE webhook_inbox (
    id BIGSERIAL PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    provider TEXT NOT NULL, event_id TEXT NOT NULL, kind TEXT NOT NULL, payload JSONB NOT NULL,
    outcome TEXT NOT NULL DEFAULT 'received', received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, provider, event_id)
);
-- Events for the merchant's own systems. A row stays until delivered, so a crashed worker loses nothing.
CREATE TABLE outbox_events (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    type TEXT NOT NULL, payload JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'delivered', 'dead')),
    attempts INT NOT NULL DEFAULT 0, next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    delivered_at TIMESTAMPTZ, last_error TEXT, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX outbox_due ON outbox_events (next_attempt_at) WHERE status = 'pending';

-- ---------- settlements and reconciliation ----------
CREATE TABLE settlements (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    provider TEXT NOT NULL, provider_ref TEXT NOT NULL, utr TEXT, currency TEXT NOT NULL,
    gross_minor BIGINT NOT NULL, refunds_minor BIGINT NOT NULL, fee_minor BIGINT NOT NULL,
    adjustment_minor BIGINT NOT NULL, net_minor BIGINT NOT NULL, settled_on DATE NOT NULL,
    status TEXT NOT NULL DEFAULT 'imported' CHECK (status IN ('imported', 'matched', 'exception')),
    bank_line_id BIGINT, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, provider, provider_ref)
);
CREATE TABLE settlement_items (
    id BIGSERIAL PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    settlement_id TEXT NOT NULL REFERENCES settlements(id),
    kind TEXT NOT NULL CHECK (kind IN ('payment', 'refund', 'fee', 'adjustment')),
    provider_ref TEXT, attempt_id TEXT REFERENCES payment_attempts(id), refund_id TEXT REFERENCES refunds(id),
    amount_minor BIGINT NOT NULL, description TEXT NOT NULL DEFAULT ''
);
CREATE TABLE bank_statement_lines (
    id BIGSERIAL PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    posted_on DATE NOT NULL, amount_minor BIGINT NOT NULL, currency TEXT NOT NULL,
    narration TEXT NOT NULL DEFAULT '', utr TEXT, matched_settlement_id TEXT REFERENCES settlements(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), UNIQUE (tenant_id, utr)
);
CREATE TABLE recon_cases (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    kind TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'assigned', 'resolved')),
    subject_type TEXT NOT NULL, subject_id TEXT NOT NULL,
    expected_minor BIGINT, actual_minor BIGINT, diff_minor BIGINT,
    explanation TEXT NOT NULL, detail JSONB NOT NULL DEFAULT '{}',
    assignee TEXT, resolution TEXT, created_at TIMESTAMPTZ NOT NULL DEFAULT now(), resolved_at TIMESTAMPTZ,
    UNIQUE (tenant_id, kind, subject_id)
);

-- ---------- row-level security ----------
DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY[
    'branches','users','tenant_settings','audit_events','provider_accounts','customers','invoices',
    'payment_intents','payment_attempts','refunds','invoice_allocations','ledger_transactions','ledger_entries',
    'idempotency_keys','webhook_inbox','outbox_events','settlements','settlement_items','bank_statement_lines','recon_cases']
  LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format($p$CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting('zyvor.tenant_id', true)) WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true))$p$, t);
  END LOOP;
END $$;
