-- Exchange rates you enter (INR per 1 USD, in millionths), used to show foreign-currency amounts in INR.
CREATE TABLE fx_rates (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    currency TEXT NOT NULL CHECK (currency <> 'INR'), rate_micro BIGINT NOT NULL CHECK (rate_micro > 0),
    as_of DATE NOT NULL, note TEXT, created_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, currency, as_of)
);
CREATE TRIGGER fx_rates_immutable BEFORE UPDATE OR DELETE ON fx_rates FOR EACH ROW EXECUTE FUNCTION forbid_mutation();
ALTER TABLE fx_rates ENABLE ROW LEVEL SECURITY;
ALTER TABLE fx_rates FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON fx_rates
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));
