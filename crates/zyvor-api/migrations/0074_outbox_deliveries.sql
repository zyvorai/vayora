-- Per-attempt history for outbound webhook deliveries.
CREATE TABLE IF NOT EXISTS outbox_deliveries (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    event_id TEXT NOT NULL REFERENCES outbox_events(id) ON DELETE CASCADE,
    attempt_no INT NOT NULL,
    ok BOOLEAN NOT NULL,
    http_status INT,
    error TEXT,
    latency_ms INT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS outbox_deliveries_event_idx ON outbox_deliveries (tenant_id, event_id, created_at DESC);

ALTER TABLE outbox_deliveries ENABLE ROW LEVEL SECURITY;
ALTER TABLE outbox_deliveries FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON outbox_deliveries;
CREATE POLICY tenant_isolation ON outbox_deliveries
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));
