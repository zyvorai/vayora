-- Copyright 2026 Zyvor AI Labs · https://zyvor.dev
-- SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

-- Vendor onboarding workflow, commission rules, payout batches and payout schedules. Additive only.
--
--   * vendors: a workflow status (invited -> details_submitted -> bank_added -> verified -> active, or suspended),
--     the details a person recorded, who checked the bank account and how, and per-vendor payout settings (hold
--     days, minimum payout, a default commission rule). Vendors that already exist, made with POST /v1/vendors,
--     stay 'active': the default is 'active' so nothing that works today stops working.
--   * vendor_documents: optional files kept for a vendor, encrypted, like onboarding uploads.
--   * commission_rules: percentage (basis points) and/or a flat amount, with a floor and a cap, for a vendor, a
--     payment link or an event reference, with effective dates. No rate is built in: every rule is typed by a person.
--   * payment_splits: what the commission was and which rule produced it, so a refund can be explained later.
--   * payout_batches / payout_batch_items: a run of payouts approved and sent together. Every item is a normal row in
--     `payouts`, so the existing approval, ledger and bank-reference rules still apply to each one.
--   * payout_schedules: when a vendor's balance should be turned into a proposed batch.
--   * tenant_settings.payout_policy: limits and the one explicit switch for approving small scheduled batches.

ALTER TABLE vendors
    ADD COLUMN status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('invited', 'details_submitted', 'bank_added', 'verified', 'active', 'suspended')),
    ADD COLUMN legal_name TEXT,
    ADD COLUMN email TEXT,
    ADD COLUMN phone TEXT,
    ADD COLUMN address TEXT,
    ADD COLUMN bank_holder_name TEXT,
    ADD COLUMN bank_added_by TEXT,
    ADD COLUMN bank_added_at TIMESTAMPTZ,
    -- the automatic check (when a provider is configured) and the name comparison, as recorded
    ADD COLUMN bank_check JSONB,
    ADD COLUMN verified_by TEXT,
    ADD COLUMN verified_at TIMESTAMPTZ,
    ADD COLUMN name_override_reason TEXT,
    ADD COLUMN invited_by TEXT,
    ADD COLUMN invited_at TIMESTAMPTZ,
    ADD COLUMN details_submitted_at TIMESTAMPTZ,
    ADD COLUMN activated_at TIMESTAMPTZ,
    ADD COLUMN suspended_at TIMESTAMPTZ,
    ADD COLUMN suspended_reason TEXT,
    ADD COLUMN hold_days INT NOT NULL DEFAULT 0 CHECK (hold_days BETWEEN 0 AND 365),
    ADD COLUMN min_payout_minor BIGINT NOT NULL DEFAULT 0 CHECK (min_payout_minor >= 0),
    ADD COLUMN default_rule_id TEXT;

CREATE TABLE vendor_documents (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    vendor_id TEXT NOT NULL REFERENCES vendors(id),
    kind TEXT NOT NULL,
    filename TEXT NOT NULL,
    content_type TEXT NOT NULL,
    size_bytes INT NOT NULL,
    sha256 TEXT NOT NULL,
    data_enc BYTEA NOT NULL,
    uploaded_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (vendor_id, sha256)
);
CREATE INDEX vendor_documents_vendor ON vendor_documents (tenant_id, vendor_id);

CREATE TABLE commission_rules (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    name TEXT NOT NULL,
    -- the scope: a vendor, a payment link and/or an event reference. All three empty = a shared rule that applies
    -- only where a vendor names it as its default.
    vendor_id TEXT REFERENCES vendors(id),
    payment_link_id TEXT,
    event_ref TEXT,
    currency TEXT NOT NULL DEFAULT 'INR',
    basis_points INT CHECK (basis_points BETWEEN 0 AND 10000),
    flat_minor BIGINT CHECK (flat_minor >= 0),
    min_commission_minor BIGINT CHECK (min_commission_minor >= 0),
    cap_minor BIGINT CHECK (cap_minor >= 0),
    effective_from DATE NOT NULL,
    effective_to DATE,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'retired')),
    created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    retired_by TEXT,
    retired_at TIMESTAMPTZ,
    CHECK (basis_points IS NOT NULL OR flat_minor IS NOT NULL),
    CHECK (effective_to IS NULL OR effective_to >= effective_from),
    CHECK (cap_minor IS NULL OR min_commission_minor IS NULL OR cap_minor >= min_commission_minor)
);
CREATE INDEX commission_rules_scope ON commission_rules (tenant_id, vendor_id, status);

ALTER TABLE payment_splits
    ADD COLUMN gross_minor BIGINT,
    ADD COLUMN commission_minor BIGINT NOT NULL DEFAULT 0 CHECK (commission_minor >= 0),
    ADD COLUMN rule_id TEXT,
    ADD COLUMN accrued_at TIMESTAMPTZ;

CREATE TABLE payout_batches (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    name TEXT NOT NULL,
    -- your own reference for the run; a second identical file needs a different one
    batch_ref TEXT,
    source TEXT NOT NULL CHECK (source IN ('csv', 'items', 'balances', 'schedule')),
    -- only the decision lives here; whether the items are paid is read from the payouts themselves
    status TEXT NOT NULL CHECK (status IN ('pending_approval', 'approved', 'rejected', 'canceled')),
    currency TEXT NOT NULL,
    from_balance BOOLEAN NOT NULL DEFAULT false,
    rail TEXT NOT NULL DEFAULT 'imps' CHECK (rail IN ('imps', 'neft', 'upi')),
    item_count INT NOT NULL,
    total_minor BIGINT NOT NULL CHECK (total_minor > 0),
    -- the same content cannot be proposed twice while a batch with it is open or approved
    fingerprint TEXT NOT NULL,
    file_name TEXT,
    note TEXT,
    created_by TEXT NOT NULL,
    approved_by TEXT,
    approved_at TIMESTAMPTZ,
    decision_note TEXT,
    auto_approved BOOLEAN NOT NULL DEFAULT false,
    completed_notified_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX payout_batches_fingerprint ON payout_batches (tenant_id, fingerprint)
    WHERE status IN ('pending_approval', 'approved');
CREATE UNIQUE INDEX payout_batches_ref ON payout_batches (tenant_id, batch_ref) WHERE batch_ref IS NOT NULL;
CREATE INDEX payout_batches_created ON payout_batches (tenant_id, created_at DESC);

CREATE TABLE payout_batch_items (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    batch_id TEXT NOT NULL REFERENCES payout_batches(id),
    line_no INT NOT NULL,
    vendor_id TEXT NOT NULL REFERENCES vendors(id),
    -- one payout per item, ever: the payout is the thing that is paid and the rail's idempotency key
    payout_id TEXT NOT NULL UNIQUE REFERENCES payouts(id),
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0),
    purpose TEXT NOT NULL,
    reference TEXT,
    schedule_id TEXT,
    excluded BOOLEAN NOT NULL DEFAULT false,
    excluded_by TEXT,
    excluded_reason TEXT,
    -- set while a rail call is in flight or after one that did not pay; empty otherwise
    rail_state TEXT CHECK (rail_state IN ('sending', 'failed', 'unknown')),
    attempts INT NOT NULL DEFAULT 0,
    last_error TEXT,
    rail_ref TEXT,
    last_attempt_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (batch_id, line_no)
);
CREATE INDEX payout_batch_items_batch ON payout_batch_items (tenant_id, batch_id);
CREATE INDEX payout_batch_items_rail ON payout_batch_items (tenant_id, rail_state) WHERE rail_state IS NOT NULL;

CREATE TABLE payout_schedules (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    vendor_id TEXT NOT NULL REFERENCES vendors(id),
    kind TEXT NOT NULL CHECK (kind IN ('weekly', 'monthly', 'threshold')),
    -- 1 = Monday .. 7 = Sunday (weekly)
    day_of_week SMALLINT CHECK (day_of_week BETWEEN 1 AND 7),
    -- 1..28, or 0 for the last day of the month (monthly)
    day_of_month SMALLINT CHECK (day_of_month BETWEEN 0 AND 28),
    -- threshold: propose a payout once the vendor's payable balance reaches this
    threshold_minor BIGINT CHECK (threshold_minor > 0),
    -- never propose more than this in one run
    max_amount_minor BIGINT CHECK (max_amount_minor > 0),
    currency TEXT NOT NULL DEFAULT 'INR',
    active BOOLEAN NOT NULL DEFAULT true,
    next_run_on DATE NOT NULL,
    last_run_on DATE,
    last_batch_id TEXT,
    last_outcome TEXT,
    created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((kind = 'weekly') = (day_of_week IS NOT NULL)),
    CHECK ((kind = 'monthly') = (day_of_month IS NOT NULL)),
    CHECK ((kind = 'threshold') = (threshold_minor IS NOT NULL))
);
CREATE INDEX payout_schedules_due ON payout_schedules (tenant_id, next_run_on) WHERE active;

ALTER TABLE tenant_settings ADD COLUMN payout_policy JSONB NOT NULL DEFAULT '{}';

DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY['vendor_documents', 'commission_rules', 'payout_batches', 'payout_batch_items', 'payout_schedules']
  LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format('DROP POLICY IF EXISTS tenant_isolation ON %I', t);
    EXECUTE format($p$CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting('zyvor.tenant_id', true)) WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true))$p$, t);
  END LOOP;
END $$;
