-- Recent authenticated API calls for the Developers inspector (no bodies; route templates only).
CREATE TABLE IF NOT EXISTS api_request_log (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    actor TEXT NOT NULL,
    method TEXT NOT NULL,
    route TEXT NOT NULL,
    status INT NOT NULL,
    request_id TEXT NOT NULL,
    latency_ms INT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS api_request_log_tenant_idx ON api_request_log (tenant_id, created_at DESC);

ALTER TABLE api_request_log ENABLE ROW LEVEL SECURITY;
ALTER TABLE api_request_log FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON api_request_log;
CREATE POLICY tenant_isolation ON api_request_log
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));
