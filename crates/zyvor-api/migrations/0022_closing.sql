-- Cash taken at counters, and the end-of-day closing that compares what the system expects with what
-- was counted. Cash entries are never edited: a mistake is corrected with an opposite entry.
-- Closings are operational records; cash is not posted to the payments ledger.
CREATE TABLE cash_entries (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    branch_id TEXT NOT NULL REFERENCES branches(id), counter_id TEXT REFERENCES counters(id),
    kind TEXT NOT NULL CHECK (kind IN ('sale', 'refund', 'float')),
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0), note TEXT,
    created_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX cash_entries_branch ON cash_entries (tenant_id, branch_id, created_at);
CREATE TABLE closings (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    branch_id TEXT NOT NULL REFERENCES branches(id), day DATE NOT NULL,
    expected_cash_minor BIGINT NOT NULL, counted_cash_minor BIGINT NOT NULL CHECK (counted_cash_minor >= 0),
    difference_minor BIGINT NOT NULL, digital JSONB NOT NULL, note TEXT,
    counted_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    signed_off_by TEXT, signed_off_at TIMESTAMPTZ,
    UNIQUE (branch_id, day)
);
DO $$ DECLARE t TEXT; BEGIN
  FOREACH t IN ARRAY ARRAY['cash_entries','closings'] LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format($p$CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting('zyvor.tenant_id', true)) WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true))$p$, t);
  END LOOP;
END $$;
