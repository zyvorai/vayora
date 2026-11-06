-- Tenant isolation for acquire settlement batches and network card tokens.
ALTER TABLE settlement_batches ENABLE ROW LEVEL SECURITY;
ALTER TABLE settlement_batches FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON settlement_batches;
CREATE POLICY tenant_isolation ON settlement_batches
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));

ALTER TABLE settlement_batch_items ENABLE ROW LEVEL SECURITY;
ALTER TABLE settlement_batch_items FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON settlement_batch_items;
CREATE POLICY tenant_isolation ON settlement_batch_items
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));

ALTER TABLE card_tokens ENABLE ROW LEVEL SECURITY;
ALTER TABLE card_tokens FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON card_tokens;
CREATE POLICY tenant_isolation ON card_tokens
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));
