-- Copyright 2026 Zyvor AI Labs · https://zyvor.dev
-- SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

-- Payment-link amount modes and lifecycle, payer-instrument rules, VPA / bank-account verification, the risk rules
-- engine, and payout links. Additive only; every new table is tenant-scoped with row-level security.

-- 1. Payment links: amount mode, suggested amount, instrument rules, normalised lifecycle status.
-- `amount_mode` is NULL on links made before this migration; the code reads NULL as "exact" (or
-- "pay_what_you_want" when `open_amount` is set), so no existing row has to be rewritten.
ALTER TABLE payment_links
    ADD COLUMN amount_mode TEXT CHECK (amount_mode IS NULL OR amount_mode IN ('exact', 'minimum', 'up_to', 'pay_what_you_want')),
    ADD COLUMN suggested_amount_minor BIGINT CHECK (suggested_amount_minor IS NULL OR suggested_amount_minor > 0),
    ADD COLUMN instrument_rules JSONB,
    ADD COLUMN lifecycle_status TEXT NOT NULL DEFAULT 'created'
        CHECK (lifecycle_status IN ('created', 'active', 'partially_paid', 'paid', 'expired', 'cancelled', 'failed')),
    ADD COLUMN lifecycle_at TIMESTAMPTZ,
    ADD COLUMN lifecycle_checked_at TIMESTAMPTZ,
    ADD CONSTRAINT payment_links_amount_mode_shape CHECK (
        amount_mode IS NULL
        OR (amount_mode = 'exact' AND NOT open_amount)
        OR (amount_mode = 'minimum' AND open_amount AND min_amount_minor IS NOT NULL AND max_amount_minor IS NULL)
        OR (amount_mode = 'up_to' AND open_amount AND max_amount_minor IS NOT NULL AND min_amount_minor IS NULL)
        OR (amount_mode = 'pay_what_you_want' AND open_amount));
CREATE INDEX payment_links_lifecycle_sweep ON payment_links (lifecycle_checked_at NULLS FIRST) WHERE lifecycle_status <> 'paid';
-- the payments made through a link (shared name with the link-upgrades migration; harmless when it already exists)
CREATE INDEX IF NOT EXISTS payment_intents_link ON payment_intents ((metadata->>'payment_link')) WHERE metadata->>'payment_link' IS NOT NULL;

ALTER TABLE payment_intents ADD COLUMN instrument_rules JSONB;

-- Every change of a link's lifecycle status, once, in order.
CREATE TABLE payment_link_status_events (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    link_id TEXT NOT NULL REFERENCES payment_links(id),
    from_status TEXT, to_status TEXT NOT NULL, reason TEXT NOT NULL,
    at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX payment_link_status_events_link ON payment_link_status_events (link_id, at);

-- 2. Payer-instrument checks: one row per captured attempt that fell under an instrument rule.
CREATE TABLE instrument_checks (
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    attempt_id TEXT NOT NULL REFERENCES payment_attempts(id),
    intent_id TEXT NOT NULL REFERENCES payment_intents(id),
    link_id TEXT REFERENCES payment_links(id),
    result TEXT NOT NULL CHECK (result IN ('allowed', 'disallowed', 'unverifiable')),
    -- what the provider reported, without the name part of a UPI id
    payer_bank TEXT, payer_vpa_handle TEXT,
    refund_id TEXT, detail TEXT NOT NULL DEFAULT '',
    checked_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (attempt_id)
);

-- 3. VPA and bank-account verification results. Never a full account number or UPI id.
CREATE TABLE verifications (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    kind TEXT NOT NULL CHECK (kind IN ('vpa', 'bank_account')),
    subject_masked TEXT NOT NULL, ifsc TEXT,
    -- keyed hash of the subject and the name given, so a repeat within a few minutes reuses the answer
    request_hash TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('valid', 'invalid', 'inconclusive')),
    name_match TEXT NOT NULL CHECK (name_match IN ('match', 'review', 'mismatch', 'not_checked')),
    provider TEXT NOT NULL, simulated BOOLEAN NOT NULL, detail TEXT NOT NULL,
    created_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX verifications_recent ON verifications (tenant_id, created_at DESC);
CREATE INDEX verifications_reuse ON verifications (tenant_id, request_hash, created_at DESC);

-- 4. Risk rules engine.
-- A per-tenant secret (itself encrypted with the master key) for the keyed hashes below. It does not change when the
-- master key is rotated, so stored list entries keep matching.
CREATE TABLE risk_keys (
    tenant_id TEXT PRIMARY KEY REFERENCES tenants(id), key_enc BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE risk_rules (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 60),
    kind TEXT NOT NULL CHECK (kind IN ('velocity', 'amount')),
    params JSONB NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('allow', 'review', 'decline')),
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, name)
);
CREATE TABLE risk_list_entries (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    list TEXT NOT NULL CHECK (list IN ('allow', 'block')),
    dimension TEXT NOT NULL CHECK (dimension IN ('customer', 'device', 'phone', 'email', 'ip', 'card', 'vpa')),
    value_hash TEXT NOT NULL CHECK (char_length(value_hash) = 32),
    -- a short masked hint so a person can recognise the entry; never the value
    label TEXT NOT NULL DEFAULT '',
    created_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ,
    UNIQUE (tenant_id, list, dimension, value_hash)
);
-- What was seen on each payment attempt, as keyed hashes only, so velocity rules can count. Deleted by the worker
-- after seven days.
CREATE TABLE risk_signals (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    intent_id TEXT NOT NULL, at TIMESTAMPTZ NOT NULL DEFAULT now(),
    amount_minor BIGINT NOT NULL,
    customer_h TEXT, device_h TEXT, phone_h TEXT, email_h TEXT, ip_h TEXT, card_h TEXT, vpa_h TEXT
);
CREATE INDEX risk_signals_at ON risk_signals (at);
CREATE INDEX risk_signals_customer ON risk_signals (tenant_id, customer_h, at) WHERE customer_h IS NOT NULL;
CREATE INDEX risk_signals_device ON risk_signals (tenant_id, device_h, at) WHERE device_h IS NOT NULL;
CREATE INDEX risk_signals_phone ON risk_signals (tenant_id, phone_h, at) WHERE phone_h IS NOT NULL;
CREATE INDEX risk_signals_email ON risk_signals (tenant_id, email_h, at) WHERE email_h IS NOT NULL;
CREATE INDEX risk_signals_ip ON risk_signals (tenant_id, ip_h, at) WHERE ip_h IS NOT NULL;
CREATE INDEX risk_signals_card ON risk_signals (tenant_id, card_h, at) WHERE card_h IS NOT NULL;
CREATE INDEX risk_signals_vpa ON risk_signals (tenant_id, vpa_h, at) WHERE vpa_h IS NOT NULL;
-- Why each decision was made: rule names and the numbers they saw. No personal data.
CREATE TABLE risk_decisions (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    subject_type TEXT NOT NULL CHECK (subject_type IN ('payment_intent', 'order')),
    subject_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('allow', 'review', 'decline')),
    score INT, fired JSONB NOT NULL DEFAULT '[]',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX risk_decisions_subject ON risk_decisions (tenant_id, subject_type, subject_id, created_at DESC);
-- Delivery-risk tiers the merchant knows from their own history, per pin-code prefix (3 to 6 digits).
CREATE TABLE risk_pin_tiers (
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    pin_prefix TEXT NOT NULL CHECK (pin_prefix ~ '^[1-9][0-9]{2,5}$'),
    tier TEXT NOT NULL CHECK (tier IN ('low', 'medium', 'high')),
    updated_by TEXT NOT NULL, updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, pin_prefix)
);

-- 5. Payout links: the recipient enters their own bank or UPI details; the payout then follows the normal
-- request, approval and payment steps. The link token is stored only as a hash.
CREATE TABLE payout_links (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    token_hash TEXT NOT NULL UNIQUE,
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0),
    currency TEXT NOT NULL, purpose TEXT NOT NULL CHECK (char_length(purpose) BETWEEN 1 AND 200),
    -- the name the merchant expects the account to be held in; when absent the name the recipient types is used
    expected_name TEXT CHECK (expected_name IS NULL OR char_length(expected_name) BETWEEN 2 AND 100),
    external_ref TEXT CHECK (external_ref IS NULL OR char_length(external_ref) BETWEEN 1 AND 64),
    expires_at TIMESTAMPTZ NOT NULL,
    cancelled_at TIMESTAMPTZ,
    failed_attempts INT NOT NULL DEFAULT 0,
    vendor_id TEXT REFERENCES vendors(id), payout_id TEXT REFERENCES payouts(id),
    verification_id TEXT REFERENCES verifications(id),
    claimed_at TIMESTAMPTZ,
    created_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX payout_links_external_ref ON payout_links (tenant_id, external_ref) WHERE external_ref IS NOT NULL;

DO $$ DECLARE t TEXT; BEGIN
  FOREACH t IN ARRAY ARRAY['payment_link_status_events', 'instrument_checks', 'verifications', 'risk_keys', 'risk_rules',
                           'risk_list_entries', 'risk_signals', 'risk_decisions', 'risk_pin_tiers', 'payout_links'] LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format($p$CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting('zyvor.tenant_id', true)) WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true))$p$, t);
  END LOOP;
END $$;
