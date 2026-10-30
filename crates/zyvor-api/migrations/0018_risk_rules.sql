-- Per-tenant risk limits checked when a payment is created and when an attempt starts.
ALTER TABLE tenant_settings ADD COLUMN risk_rules JSONB NOT NULL DEFAULT '{}';
