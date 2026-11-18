-- Bank statement import and matching. Additive only.
--   * bank_statement_imports: one row per uploaded file (who, when, what it contained)
--   * bank_statement_lines: a stable per-line fingerprint (so overlapping or repeated files add nothing), the
--     reference column as the bank gave it, and where in the file the line came from
--   * bank_line_matches: the evidence for every match between a bank line and a settlement, payment, payout or
--     refund. 'confirmed' means a reference matched exactly or a person accepted it; 'suggested' (amount and date
--     only) never becomes confirmed by itself.
--   * payment_attempts.bank_reference: the bank/UPI reference (RRN) when the provider reports one.
CREATE TABLE bank_statement_imports (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    source TEXT NOT NULL,
    layout TEXT NOT NULL,
    filename TEXT,
    file_sha256 TEXT NOT NULL,
    rows_total INT NOT NULL,
    rows_added INT NOT NULL,
    rows_duplicate INT NOT NULL,
    credits_minor BIGINT NOT NULL DEFAULT 0,
    debits_minor BIGINT NOT NULL DEFAULT 0,
    period_from DATE,
    period_to DATE,
    created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX bank_statement_imports_tenant ON bank_statement_imports (tenant_id, created_at DESC);

ALTER TABLE bank_statement_lines
    ADD COLUMN reference TEXT,
    ADD COLUMN fingerprint TEXT,
    ADD COLUMN import_id BIGINT REFERENCES bank_statement_imports(id),
    ADD COLUMN line_no INT,
    ADD COLUMN value_date DATE,
    ADD COLUMN balance_minor BIGINT;
CREATE UNIQUE INDEX bank_statement_lines_fingerprint ON bank_statement_lines (tenant_id, fingerprint) WHERE fingerprint IS NOT NULL;
CREATE INDEX bank_statement_lines_posted ON bank_statement_lines (tenant_id, posted_on);

CREATE TABLE bank_line_matches (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    bank_line_id BIGINT NOT NULL REFERENCES bank_statement_lines(id),
    subject_type TEXT NOT NULL CHECK (subject_type IN ('settlement', 'payment_attempt', 'payout', 'refund')),
    subject_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('suggested', 'confirmed', 'rejected', 'superseded')),
    -- reference_column, reference_in_narration, reference_amount_differs, reference_ambiguous, amount_date_window
    rule TEXT NOT NULL,
    key_matched TEXT,
    evidence JSONB NOT NULL DEFAULT '{}',
    decided_by TEXT,
    decided_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, bank_line_id, subject_type, subject_id)
);
-- a line is explained by one thing, and one thing by one line
CREATE UNIQUE INDEX bank_line_matches_one_per_line ON bank_line_matches (tenant_id, bank_line_id) WHERE status = 'confirmed';
CREATE UNIQUE INDEX bank_line_matches_one_per_subject ON bank_line_matches (tenant_id, subject_type, subject_id) WHERE status = 'confirmed';
CREATE INDEX bank_line_matches_subject ON bank_line_matches (tenant_id, subject_type, subject_id);

ALTER TABLE payment_attempts ADD COLUMN bank_reference TEXT;
CREATE INDEX payment_attempts_bank_reference ON payment_attempts (tenant_id, bank_reference) WHERE bank_reference IS NOT NULL;

DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY['bank_statement_imports', 'bank_line_matches']
  LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format('DROP POLICY IF EXISTS tenant_isolation ON %I', t);
    EXECUTE format($p$CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting('zyvor.tenant_id', true)) WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true))$p$, t);
  END LOOP;
END $$;
