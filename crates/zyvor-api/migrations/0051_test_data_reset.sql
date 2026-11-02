-- "Reset test data": a test environment can clear its own payments and money records. The tables below are
-- append-only for everyone else; a delete is let through only when the session names the row's own tenant in
-- `zyvor.test_reset` AND that tenant is a test tenant (looked up here, not supplied by the caller). Live
-- tenants stay strictly append-only. The audit trail and its seals are never deletable, and are not in this list.
CREATE OR REPLACE FUNCTION forbid_mutation() RETURNS trigger AS $$
BEGIN
  IF TG_OP = 'DELETE'
     AND TG_TABLE_NAME IN ('ledger_entries', 'ledger_transactions', 'invoice_allocations', 'credit_notes', 'invoice_lines')
     AND current_setting('zyvor.test_reset', true) = OLD.tenant_id
     AND EXISTS (SELECT 1 FROM tenants WHERE id = OLD.tenant_id AND env = 'test')
  THEN
    RETURN OLD;
  END IF;
  RAISE EXCEPTION '% is append-only', TG_TABLE_NAME;
END;
$$ LANGUAGE plpgsql;
