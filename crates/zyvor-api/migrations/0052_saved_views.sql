-- Saved views: a named set of list filters (for now only on Payments). Personal by default; a view can be
-- shared with everyone in the environment. Only the filters are stored, never results.
CREATE TABLE saved_views (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    scope TEXT NOT NULL CHECK (scope IN ('payments')),
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 60),
    query JSONB NOT NULL DEFAULT '{}',
    owner TEXT NOT NULL, shared BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, owner, scope, name)
);
ALTER TABLE saved_views ENABLE ROW LEVEL SECURITY;
ALTER TABLE saved_views FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON saved_views
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));
