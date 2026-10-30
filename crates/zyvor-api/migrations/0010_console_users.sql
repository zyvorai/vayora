-- Named console users with roles, optional TOTP second factor, and lockout after repeated failures.
CREATE TABLE console_users (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    username TEXT NOT NULL, password_hash TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('admin', 'finance', 'viewer')),
    totp_secret_enc BYTEA, totp_confirmed BOOLEAN NOT NULL DEFAULT false, totp_last_step BIGINT NOT NULL DEFAULT 0,
    recovery_hashes TEXT[] NOT NULL DEFAULT '{}',
    failed_attempts INT NOT NULL DEFAULT 0, locked_until TIMESTAMPTZ,
    last_login_at TIMESTAMPTZ, disabled_at TIMESTAMPTZ,
    created_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, username)
);
ALTER TABLE console_users ENABLE ROW LEVEL SECURITY;
ALTER TABLE console_users FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON console_users
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));
