-- India GST: seller and buyer GSTINs, tax invoices with HSN/SAC lines split into CGST/SGST/IGST,
-- and tax-aware credit notes. Everything is optional: invoices without lines work as before.
ALTER TABLE tenant_settings ADD COLUMN gstin TEXT, ADD COLUMN gst_legal_name TEXT;
ALTER TABLE customers ADD COLUMN gstin TEXT;
ALTER TABLE invoices
    ADD COLUMN place_of_supply TEXT, ADD COLUMN buyer_gstin TEXT,
    ADD COLUMN reverse_charge BOOLEAN NOT NULL DEFAULT false;

CREATE TABLE invoice_lines (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    invoice_id TEXT NOT NULL REFERENCES invoices(id), position INT NOT NULL,
    description TEXT NOT NULL, hsn_sac TEXT NOT NULL,
    quantity BIGINT NOT NULL CHECK (quantity > 0), unit_price_minor BIGINT NOT NULL CHECK (unit_price_minor >= 0),
    taxable_minor BIGINT NOT NULL, rate_bp INT NOT NULL,
    cgst_minor BIGINT NOT NULL DEFAULT 0, sgst_minor BIGINT NOT NULL DEFAULT 0, igst_minor BIGINT NOT NULL DEFAULT 0,
    UNIQUE (invoice_id, position)
);
CREATE TRIGGER invoice_lines_immutable BEFORE UPDATE OR DELETE ON invoice_lines FOR EACH ROW EXECUTE FUNCTION forbid_mutation();
ALTER TABLE invoice_lines ENABLE ROW LEVEL SECURITY;
ALTER TABLE invoice_lines FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON invoice_lines
    USING (tenant_id = current_setting('zyvor.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true));

-- a credit note on a tax invoice reverses tax in proportion to the invoice
ALTER TABLE credit_notes
    ADD COLUMN taxable_minor BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN cgst_minor BIGINT NOT NULL DEFAULT 0, ADD COLUMN sgst_minor BIGINT NOT NULL DEFAULT 0, ADD COLUMN igst_minor BIGINT NOT NULL DEFAULT 0;
