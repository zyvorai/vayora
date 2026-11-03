-- What a subscription invoice is made of, for the person reading it: [{description, quantity, unit_price_minor,
-- amount_minor}]. Plain (non-tax) lines; a GST tax invoice keeps using invoice_lines. NULL on invoices made before this.
ALTER TABLE invoices ADD COLUMN details JSONB;
