-- Copyright 2026 Zyvor AI Labs · https://zyvor.dev
-- SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

-- Cash visibility, refund analytics and accounting exports. Additive only. Nothing here moves money or posts to
-- the ledger; every table is tenant-scoped with row-level security.
--   * cash_settings: per-tenant knobs for the settlement forecast / delay alerts and the refund SLA.
--   * settlement_lag_rules: the merchant's OWN expected settlement lag per provider and method (calendar days
--     from the payment to the settlement). Entered by a person from their own contract; nothing is shipped.
--   * account_mappings: ledger account -> stable accounting code and name for exports. An exact ledger account
--     (`bank`) or a prefix ending in `:*` (`provider_receivable:*`).
--   * refund_blocks: refund requests that were stopped as duplicates (same idempotency key replayed, or more than
--     is left to refund). Counts only; no personal data.
CREATE TABLE cash_settings (
    tenant_id TEXT PRIMARY KEY REFERENCES tenants(id),
    -- observed lag is shown and used only with at least this many settled payments per provider and method
    min_samples INT NOT NULL DEFAULT 10 CHECK (min_samples >= 3 AND min_samples <= 500),
    -- extra days allowed beyond the expected or observed lag before an alert opens
    alert_grace_days INT NOT NULL DEFAULT 0 CHECK (alert_grace_days >= 0 AND alert_grace_days <= 30),
    -- refunds still unknown / failed / processing / waiting approval after this long breach the SLA
    refund_sla_hours INT NOT NULL DEFAULT 48 CHECK (refund_sla_hours >= 1 AND refund_sla_hours <= 24 * 90),
    updated_by TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE settlement_lag_rules (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    provider TEXT NOT NULL,
    method TEXT NOT NULL,                      -- a payment method, or '*' for every method of the provider
    expected_lag_days INT NOT NULL CHECK (expected_lag_days >= 0 AND expected_lag_days <= 60),
    note TEXT NOT NULL DEFAULT '',
    created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, provider, method)
);

CREATE TABLE account_mappings (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    ledger_account TEXT NOT NULL,
    code TEXT NOT NULL CHECK (length(code) BETWEEN 1 AND 40),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    updated_by TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, ledger_account)
);

CREATE TABLE refund_blocks (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    intent_id TEXT NOT NULL,
    reason TEXT NOT NULL CHECK (reason IN ('idempotent_replay', 'exceeds_refundable')),
    amount_minor BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX refund_blocks_tenant_time ON refund_blocks (tenant_id, created_at);

-- the journal export reads the entries of a page of transactions; the foreign key does not index them
CREATE INDEX ledger_entries_transaction ON ledger_entries (transaction_id);
CREATE INDEX settlement_items_attempt_kind ON settlement_items (tenant_id, attempt_id) WHERE kind = 'payment';

DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY['cash_settings', 'settlement_lag_rules', 'account_mappings', 'refund_blocks']
  LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format('DROP POLICY IF EXISTS tenant_isolation ON %I', t);
    EXECUTE format($p$CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting('zyvor.tenant_id', true)) WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true))$p$, t);
  END LOOP;
END $$;
