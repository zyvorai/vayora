-- Tamper evidence for the audit trail. A seal covers a contiguous range of a tenant's audit events:
-- its hash commits to the events' content AND to the previous seal, so editing, deleting or
-- inserting an event anywhere in a sealed range (even with a superuser that bypasses the
-- append-only triggers) is detectable by recomputing the chain.
CREATE TABLE audit_seals (
    id BIGSERIAL PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    from_id BIGINT NOT NULL, to_id BIGINT NOT NULL, row_count BIGINT NOT NULL,
    prev_hash TEXT NOT NULL, hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, to_id)
);
CREATE TRIGGER audit_seals_immutable BEFORE UPDATE OR DELETE ON audit_seals FOR EACH ROW EXECUTE FUNCTION forbid_mutation();
ALTER TABLE audit_seals ENABLE ROW LEVEL SECURITY;
ALTER TABLE audit_seals FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON audit_seals
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));

-- Digest of a tenant's audit events in an id range, in a canonical text form.
CREATE FUNCTION audit_range_digest(p_tenant TEXT, p_from BIGINT, p_to BIGINT) RETURNS TEXT AS $$
    SELECT encode(sha256(convert_to(COALESCE(string_agg(
        concat_ws('|', id, tenant_id, actor, action, COALESCE(subject, ''), detail::text,
                  to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US')),
        E'\n' ORDER BY id), ''), 'UTF8')), 'hex')
    FROM audit_events WHERE tenant_id = p_tenant AND id BETWEEN p_from AND p_to
$$ LANGUAGE sql STABLE;
