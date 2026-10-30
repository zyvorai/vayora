-- Open-amount links: a printed QR where the payer types the amount, within limits the merchant sets.
DO $$ DECLARE c TEXT; BEGIN
  FOR c IN SELECT conname FROM pg_constraint WHERE conrelid = 'payment_links'::regclass AND contype = 'c' AND pg_get_constraintdef(oid) LIKE '%invoice_id%' LOOP
    EXECUTE format('ALTER TABLE payment_links DROP CONSTRAINT %I', c);
  END LOOP;
END $$;
ALTER TABLE payment_links
    ADD COLUMN open_amount BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN min_amount_minor BIGINT CHECK (min_amount_minor > 0),
    ADD COLUMN max_amount_minor BIGINT CHECK (max_amount_minor > 0),
    ADD CONSTRAINT payment_links_amount_kind CHECK (
        CASE WHEN open_amount THEN amount_minor IS NULL AND invoice_id IS NULL
             ELSE (amount_minor IS NOT NULL) <> (invoice_id IS NOT NULL) AND min_amount_minor IS NULL AND max_amount_minor IS NULL END),
    ADD CONSTRAINT payment_links_open_range CHECK (min_amount_minor IS NULL OR max_amount_minor IS NULL OR min_amount_minor <= max_amount_minor);
