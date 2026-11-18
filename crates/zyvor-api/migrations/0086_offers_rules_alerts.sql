-- Provider-agnostic offers, a no-code routing rule builder with versions, routing A/B experiments, and
-- provider / bank downtime alerts. Additive only. SANDBOX / simulator use: no real money moves.

-- ---------------------------------------------------------------------------------------------------------------
-- What the payer used, without any card number. Set when an attempt is created: the UPI handle (the part after @),
-- an optional UPI app, an optional bank for netbanking, a card network class. Used by offers, routing rules and
-- the downtime alerts. `source` says where it came from (caller_hint | card_token | vpa).
-- ---------------------------------------------------------------------------------------------------------------
ALTER TABLE payment_attempts ADD COLUMN IF NOT EXISTS instrument JSONB NOT NULL DEFAULT '{}'::jsonb;
CREATE INDEX IF NOT EXISTS payment_attempts_instrument_idx ON payment_attempts (tenant_id, provider, method, created_at DESC)
    WHERE instrument <> '{}'::jsonb;

-- ---------------------------------------------------------------------------------------------------------------
-- Offers (merchant-funded discounts, evaluated when a payment attempt is created)
-- ---------------------------------------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS offers (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
    description TEXT,
    -- a typed code, or NULL for an offer that applies by itself when its conditions match
    code TEXT,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'paused', 'archived')),
    kind TEXT NOT NULL CHECK (kind IN ('percent', 'flat')),
    -- percent: basis points (1000 = 10%); flat: minor units
    value BIGINT NOT NULL CHECK (value > 0),
    max_discount_minor BIGINT CHECK (max_discount_minor IS NULL OR max_discount_minor > 0),
    currency TEXT NOT NULL DEFAULT 'INR',
    -- conditions; NULL / empty means "any"
    methods TEXT[],
    card_networks TEXT[],
    bin_ranges JSONB NOT NULL DEFAULT '[]'::jsonb,
    upi_apps TEXT[],
    upi_handles TEXT[],
    banks TEXT[],
    min_amount_minor BIGINT CHECK (min_amount_minor IS NULL OR min_amount_minor >= 0),
    max_amount_minor BIGINT CHECK (max_amount_minor IS NULL OR max_amount_minor > 0),
    segment TEXT CHECK (segment IS NULL OR segment IN ('new', 'returning')),
    valid_from TIMESTAMPTZ,
    valid_until TIMESTAMPTZ,
    -- caps
    max_redemptions INT CHECK (max_redemptions IS NULL OR max_redemptions > 0),
    max_per_customer INT CHECK (max_per_customer IS NULL OR max_per_customer > 0),
    budget_minor BIGINT CHECK (budget_minor IS NULL OR budget_minor > 0),
    -- running totals (reserved + redeemed); kept under a row lock on this row
    redemptions INT NOT NULL DEFAULT 0 CHECK (redemptions >= 0),
    spent_minor BIGINT NOT NULL DEFAULT 0 CHECK (spent_minor >= 0),
    -- stacking: a non-stackable offer applies alone; stackable offers may combine with other stackable ones
    stackable BOOLEAN NOT NULL DEFAULT FALSE,
    stack_group TEXT,
    priority INT NOT NULL DEFAULT 0,
    -- only the merchant funds an offer here. A provider-funded or bank-funded offer is not modelled.
    funded_by TEXT NOT NULL DEFAULT 'merchant' CHECK (funded_by = 'merchant'),
    created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (kind <> 'percent' OR value <= 10000),
    CHECK (min_amount_minor IS NULL OR max_amount_minor IS NULL OR min_amount_minor <= max_amount_minor),
    CHECK (valid_from IS NULL OR valid_until IS NULL OR valid_from < valid_until)
);
CREATE UNIQUE INDEX IF NOT EXISTS offers_code_idx ON offers (tenant_id, code) WHERE code IS NOT NULL;
CREATE INDEX IF NOT EXISTS offers_tenant_status_idx ON offers (tenant_id, status);

-- One row per offer applied to one attempt. reserved: the attempt exists; redeemed: it was captured; released: the
-- attempt failed or was cancelled (the cap is given back); reversed: the whole payment was refunded.
CREATE TABLE IF NOT EXISTS offer_redemptions (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    offer_id TEXT NOT NULL REFERENCES offers(id),
    attempt_id TEXT NOT NULL REFERENCES payment_attempts(id),
    intent_id TEXT NOT NULL REFERENCES payment_intents(id),
    customer_id TEXT,
    status TEXT NOT NULL DEFAULT 'reserved' CHECK (status IN ('reserved', 'redeemed', 'released', 'reversed')),
    gross_minor BIGINT NOT NULL CHECK (gross_minor > 0),
    discount_minor BIGINT NOT NULL CHECK (discount_minor > 0),
    charged_minor BIGINT NOT NULL CHECK (charged_minor > 0),
    funded_by TEXT NOT NULL DEFAULT 'merchant' CHECK (funded_by = 'merchant'),
    method TEXT NOT NULL,
    -- the non-card-number facts the conditions were judged on
    instrument JSONB NOT NULL DEFAULT '{}'::jsonb,
    evaluation JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- refunds: cash handed back and the share of the discount that came back with it
    refunded_cash_minor BIGINT NOT NULL DEFAULT 0 CHECK (refunded_cash_minor >= 0),
    discount_returned_minor BIGINT NOT NULL DEFAULT 0 CHECK (discount_returned_minor >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (offer_id, attempt_id),
    CHECK (discount_returned_minor <= discount_minor)
);
CREATE INDEX IF NOT EXISTS offer_redemptions_attempt_idx ON offer_redemptions (attempt_id);
CREATE INDEX IF NOT EXISTS offer_redemptions_customer_idx ON offer_redemptions (offer_id, customer_id) WHERE status IN ('reserved', 'redeemed');

-- Each refund's share of the discount, so a refund is applied once and can be audited.
CREATE TABLE IF NOT EXISTS offer_refund_adjustments (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    redemption_id TEXT NOT NULL REFERENCES offer_redemptions(id),
    refund_id TEXT NOT NULL,
    cash_minor BIGINT NOT NULL CHECK (cash_minor > 0),
    discount_returned_minor BIGINT NOT NULL CHECK (discount_returned_minor >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (redemption_id, refund_id)
);

-- ---------------------------------------------------------------------------------------------------------------
-- Routing rule sets: immutable numbered versions and one active pointer per business
-- ---------------------------------------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS routing_rule_versions (
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    version INT NOT NULL CHECK (version > 0),
    rules JSONB NOT NULL,
    note TEXT,
    created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, version)
);

-- version NULL = no rule set is active, the older tenant_settings.routing.rules apply
CREATE TABLE IF NOT EXISTS routing_rule_active (
    tenant_id TEXT PRIMARY KEY REFERENCES tenants(id),
    version INT,
    previous_version INT,
    activated_by TEXT NOT NULL,
    activated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (tenant_id, version) REFERENCES routing_rule_versions (tenant_id, version),
    FOREIGN KEY (tenant_id, previous_version) REFERENCES routing_rule_versions (tenant_id, version)
);

-- ---------------------------------------------------------------------------------------------------------------
-- Routing experiments: traffic split by hash between the active rule set (control) and a variant version
-- ---------------------------------------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS routing_experiments (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
    status TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running', 'stopped', 'auto_stopped')),
    -- control_version NULL = the older settings rules were in force when it started
    control_version INT,
    variant_version INT NOT NULL,
    -- the share of payments (intents) sent to the variant, in basis points
    variant_bps INT NOT NULL CHECK (variant_bps BETWEEN 100 AND 5000),
    method TEXT,
    salt TEXT NOT NULL,
    -- guardrails
    min_sample INT NOT NULL DEFAULT 100 CHECK (min_sample BETWEEN 20 AND 100000),
    min_drop_bps INT NOT NULL DEFAULT 300 CHECK (min_drop_bps BETWEEN 50 AND 10000),
    z_x100 INT NOT NULL DEFAULT 233 CHECK (z_x100 BETWEEN 100 AND 500),
    created_by TEXT NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at TIMESTAMPTZ,
    stop_reason TEXT,
    FOREIGN KEY (tenant_id, variant_version) REFERENCES routing_rule_versions (tenant_id, version)
);
-- at most one running experiment per business
CREATE UNIQUE INDEX IF NOT EXISTS routing_experiments_one_running ON routing_experiments (tenant_id) WHERE status = 'running';

CREATE TABLE IF NOT EXISTS routing_experiment_assignments (
    attempt_id TEXT PRIMARY KEY REFERENCES payment_attempts(id),
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    experiment_id TEXT NOT NULL REFERENCES routing_experiments(id),
    intent_id TEXT NOT NULL,
    arm TEXT NOT NULL CHECK (arm IN ('control', 'variant')),
    -- the variant could not route this payment, so the control rules were used: left out of the results
    fell_back BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS routing_experiment_assignments_exp_idx ON routing_experiment_assignments (experiment_id, arm);

-- ---------------------------------------------------------------------------------------------------------------
-- Provider / bank downtime alerts
-- ---------------------------------------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS provider_alert_config (
    tenant_id TEXT PRIMARY KEY REFERENCES tenants(id),
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    min_samples INT NOT NULL DEFAULT 20 CHECK (min_samples BETWEEN 5 AND 10000),
    open_failure_pct INT NOT NULL DEFAULT 50 CHECK (open_failure_pct BETWEEN 1 AND 100),
    resolve_failure_pct INT NOT NULL DEFAULT 20 CHECK (resolve_failure_pct BETWEEN 0 AND 99),
    window_minutes INT NOT NULL DEFAULT 30 CHECK (window_minutes BETWEEN 5 AND 1440),
    hold_seconds INT NOT NULL DEFAULT 300 CHECK (hold_seconds BETWEEN 0 AND 86400),
    flap_window_seconds INT NOT NULL DEFAULT 900 CHECK (flap_window_seconds BETWEEN 0 AND 86400),
    email_to TEXT,
    updated_by TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (resolve_failure_pct < open_failure_pct)
);

CREATE TABLE IF NOT EXISTS provider_alerts (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    provider TEXT NOT NULL,
    method TEXT NOT NULL,
    -- provider_method (whole provider for a method) | bank | upi_handle
    dimension TEXT NOT NULL CHECK (dimension IN ('provider_method', 'bank', 'upi_handle')),
    dimension_value TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'resolved')),
    samples INT NOT NULL,
    failures INT NOT NULL,
    failure_pct INT NOT NULL,
    reason TEXT NOT NULL,
    opened_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at TIMESTAMPTZ,
    resolve_reason TEXT,
    -- how many times it reopened within the flap window instead of raising a new alert
    flap_count INT NOT NULL DEFAULT 0,
    acknowledged_by TEXT,
    acknowledged_at TIMESTAMPTZ
);
-- one open alert per provider, method and dimension value: the de-duplication
CREATE UNIQUE INDEX IF NOT EXISTS provider_alerts_one_open ON provider_alerts (tenant_id, provider, method, dimension, dimension_value) WHERE status = 'open';
CREATE INDEX IF NOT EXISTS provider_alerts_tenant_idx ON provider_alerts (tenant_id, status, opened_at DESC);

-- ---------------------------------------------------------------------------------------------------------------
-- row-level security on every new table
-- ---------------------------------------------------------------------------------------------------------------
DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY['offers', 'offer_redemptions', 'offer_refund_adjustments', 'routing_rule_versions', 'routing_rule_active',
                           'routing_experiments', 'routing_experiment_assignments', 'provider_alert_config', 'provider_alerts']
  LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format('DROP POLICY IF EXISTS tenant_isolation ON %I', t);
    EXECUTE format($p$CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting('zyvor.tenant_id', true)) WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true))$p$, t);
  END LOOP;
END $$;
