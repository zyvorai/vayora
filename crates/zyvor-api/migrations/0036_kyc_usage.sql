-- Each call to a paid identity-check provider (Cashfree Secure ID charges per call) is counted per business, so a
-- public signup form cannot be used to burn the platform's balance. Internal bookkeeping: not tenant data.
CREATE TABLE kyc_usage (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    kind TEXT NOT NULL,
    at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX kyc_usage_recent ON kyc_usage (tenant_id, at);
