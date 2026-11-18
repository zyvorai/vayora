-- Success-rate-aware routing, a per-method circuit breaker and failover bookkeeping.
-- The success-rate signal itself is derived from payment_attempts on demand; only what cannot be derived is stored here.

-- Why an attempt failed, in a form routing can act on:
--   declined          the provider said it failed (a definitive answer; nothing was taken)
--   provider_rejected the provider refused the request or could not be reached before it was sent (nothing was created)
--   cancelled         the merchant cancelled it (a late capture is still possible, so never a reason to switch provider)
ALTER TABLE payment_attempts ADD COLUMN IF NOT EXISTS failure_class TEXT
    CHECK (failure_class IS NULL OR failure_class IN ('declined', 'provider_rejected', 'cancelled'));
CREATE INDEX IF NOT EXISTS payment_attempts_health_idx ON payment_attempts (tenant_id, provider, method, created_at DESC);

-- Per-tenant thresholds. No row means the defaults below.
CREATE TABLE IF NOT EXISTS routing_health_config (
    tenant_id TEXT PRIMARY KEY REFERENCES tenants(id),
    breaker_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    failover_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    window_attempts INT NOT NULL DEFAULT 100 CHECK (window_attempts BETWEEN 5 AND 1000),
    window_minutes INT NOT NULL DEFAULT 30 CHECK (window_minutes BETWEEN 1 AND 1440),
    min_samples INT NOT NULL DEFAULT 10 CHECK (min_samples BETWEEN 3 AND 1000),
    degrade_failure_pct INT NOT NULL DEFAULT 50 CHECK (degrade_failure_pct BETWEEN 1 AND 100),
    recover_failure_pct INT NOT NULL DEFAULT 20 CHECK (recover_failure_pct BETWEEN 0 AND 99),
    cooldown_seconds INT NOT NULL DEFAULT 120 CHECK (cooldown_seconds BETWEEN 10 AND 86400),
    updated_by TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (recover_failure_pct < degrade_failure_pct)
);

-- Circuit-breaker state: only providers that are (or were) degraded have a row.
CREATE TABLE IF NOT EXISTS provider_health_state (
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    provider TEXT NOT NULL,
    method TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('healthy', 'degraded')),
    reason TEXT,
    degraded_since TIMESTAMPTZ,
    last_probe_at TIMESTAMPTZ,
    changed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, provider, method)
);

DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY['routing_health_config', 'provider_health_state']
  LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format('DROP POLICY IF EXISTS tenant_isolation ON %I', t);
    EXECUTE format($p$CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting('zyvor.tenant_id', true)) WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true))$p$, t);
  END LOOP;
END $$;
