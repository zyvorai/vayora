-- Onboarding documents uploaded as files. The bytes are stored encrypted with the master key; only the facts read
-- from the document (identifiers that are public, the last four characters of the PAN, the legal name) are kept in
-- the clear. Aadhaar documents are refused before anything is stored.
CREATE TABLE onboarding_uploads (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    kind TEXT NOT NULL, filename TEXT NOT NULL, content_type TEXT NOT NULL,
    size_bytes INT NOT NULL CHECK (size_bytes > 0), sha256 TEXT NOT NULL,
    data_enc BYTEA NOT NULL,
    read_method TEXT NOT NULL CHECK (read_method IN ('text', 'ocr', 'none')),
    facts JSONB NOT NULL DEFAULT '{}',
    uploaded_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, sha256)
);
ALTER TABLE onboarding_uploads ENABLE ROW LEVEL SECURITY;
ALTER TABLE onboarding_uploads FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON onboarding_uploads
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));
