-- Routing automation: shadow mode, soft-decline retry on another provider, cost-aware routing, and one kill switch.
-- Additive. Nothing here changes a payment that already exists. Every automation is OFF until a merchant turns it on;
-- no row means everything off and the kill switch not pulled.

CREATE TABLE IF NOT EXISTS routing_automation (
    tenant_id TEXT PRIMARY KEY REFERENCES tenants(id),
    -- Kill switch: while true, NO routing automation acts (shadow, failover, soft retry, cost-aware, circuit breaker).
    -- Routing falls back to the merchant's own rules and the lowest configured fee.
    paused BOOLEAN NOT NULL DEFAULT FALSE,
    paused_reason TEXT,
    paused_by TEXT,
    paused_at TIMESTAMPTZ,
    -- Shadow mode: record what a candidate policy would have chosen next to what was applied. Never changes routing.
    shadow_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    shadow_policy JSONB,                   -- {"rules": [...optional...], "cost_aware": true|false}
    -- Soft retry: after a definitive provider-side failure, start the next attempt on another provider.
    soft_retry_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    soft_retry_max_attempts INT NOT NULL DEFAULT 2 CHECK (soft_retry_max_attempts BETWEEN 2 AND 5),   -- total attempts per payment
    soft_retry_window_seconds INT NOT NULL DEFAULT 600 CHECK (soft_retry_window_seconds BETWEEN 30 AND 86400),
    -- Cost-aware routing.
    cost_aware_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    cost_success_floor_bps INT NOT NULL DEFAULT 8000 CHECK (cost_success_floor_bps BETWEEN 0 AND 10000),
    cost_premium_bps INT NOT NULL DEFAULT 500 CHECK (cost_premium_bps BETWEEN 0 AND 10000),
    cost_min_fee_samples INT NOT NULL DEFAULT 30 CHECK (cost_min_fee_samples BETWEEN 1 AND 100000),
    updated_by TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- What a soft retry needs to repeat the payer's request on another provider. Written only while soft retry is on,
-- and cleared when the retry has been decided (so a payer's UPI ID is not kept longer than needed).
ALTER TABLE payment_attempts ADD COLUMN IF NOT EXISTS retry_input JSONB;

-- One row per failed attempt a soft retry was decided for. UNIQUE (from_attempt_id) is what makes it idempotent:
-- a failed attempt is retried at most once, however many workers, webhooks and requests look at it at the same time.
CREATE TABLE IF NOT EXISTS routing_soft_retries (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    intent_id TEXT NOT NULL REFERENCES payment_intents(id),
    from_attempt_id TEXT NOT NULL UNIQUE REFERENCES payment_attempts(id),
    to_attempt_id TEXT REFERENCES payment_attempts(id),
    status TEXT NOT NULL CHECK (status IN ('started', 'skipped')),
    reason TEXT,
    from_provider TEXT NOT NULL,
    to_provider TEXT,
    method TEXT NOT NULL,
    failure_class TEXT NOT NULL,
    attempt_number INT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((status = 'started' AND to_attempt_id IS NOT NULL) OR status = 'skipped')
);
CREATE INDEX IF NOT EXISTS routing_soft_retries_intent_idx ON routing_soft_retries (tenant_id, intent_id);

-- Shadow mode: the decision a candidate policy WOULD have made, next to the one applied (the attempt's own route).
CREATE TABLE IF NOT EXISTS routing_shadow_decisions (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    intent_id TEXT NOT NULL REFERENCES payment_intents(id),
    attempt_id TEXT NOT NULL UNIQUE REFERENCES payment_attempts(id),
    method TEXT NOT NULL,
    amount_minor BIGINT NOT NULL,
    actual_provider TEXT NOT NULL,
    shadow_provider TEXT,                  -- NULL when the candidate policy found no route
    agree BOOLEAN NOT NULL,
    actual_fee_minor BIGINT,               -- estimated with the fee rules / observed fees; NULL when no basis
    shadow_fee_minor BIGINT,
    fee_basis JSONB NOT NULL DEFAULT '{}'::jsonb,
    actual_explanation TEXT,
    shadow_explanation TEXT,
    shadow_decision JSONB,                 -- the full decision, with every provider considered and why
    shadow_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS routing_shadow_decisions_window_idx ON routing_shadow_decisions (tenant_id, created_at DESC);

DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY['routing_automation', 'routing_soft_retries', 'routing_shadow_decisions']
  LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format('DROP POLICY IF EXISTS tenant_isolation ON %I', t);
    EXECUTE format($p$CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting('zyvor.tenant_id', true)) WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true))$p$, t);
  END LOOP;
END $$;
