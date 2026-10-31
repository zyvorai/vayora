-- Identity and bank checks recorded for onboarding, and why a submission went to a person.
-- Only the outcome and a short reason are kept; the full PAN and account number are never stored.
CREATE TABLE kyc_checks (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    kind TEXT NOT NULL CHECK (kind IN ('pan','gstin','bank')),
    subject TEXT NOT NULL,                       -- 'profile', or the bank account id
    outcome TEXT NOT NULL CHECK (outcome IN ('pass','fail','inconclusive')),
    detail TEXT NOT NULL, provider TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
ALTER TABLE kyc_checks ENABLE ROW LEVEL SECURITY;
ALTER TABLE kyc_checks FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON kyc_checks
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));
ALTER TABLE merchant_profiles ADD COLUMN review_reasons TEXT[] NOT NULL DEFAULT '{}';
