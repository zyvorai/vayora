-- Merchant onboarding: a reviewable record of who the merchant is and where their money goes.
-- Zyvor stores no document bytes and no full PAN or account numbers (only references and last four
-- digits); verification is recorded by a person, never assumed.
CREATE TABLE merchant_profiles (
    tenant_id TEXT PRIMARY KEY REFERENCES tenants(id),
    legal_name TEXT NOT NULL, business_type TEXT NOT NULL,
    pan_last4 TEXT NOT NULL, gstin TEXT, address TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','in_review','approved','rejected')),
    submitted_by TEXT, submitted_at TIMESTAMPTZ,
    reviewed_by TEXT, reviewed_at TIMESTAMPTZ, review_note TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE onboarding_documents (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    kind TEXT NOT NULL, reference TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','accepted','rejected')),
    added_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE merchant_bank_accounts (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    holder_name TEXT NOT NULL, ifsc TEXT NOT NULL, account_last4 TEXT NOT NULL,
    added_by TEXT NOT NULL, verified_by TEXT, verified_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
DO $$ DECLARE t TEXT; BEGIN
  FOREACH t IN ARRAY ARRAY['merchant_profiles','onboarding_documents','merchant_bank_accounts'] LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format($p$CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting('zyvor.tenant_id', true)) WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true))$p$, t);
  END LOOP;
END $$;
