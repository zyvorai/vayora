-- A payment link can ask for one reminder to its customer in the last day before it expires.
ALTER TABLE payment_links
    ADD COLUMN remind_before_expiry BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN expiry_reminded_at TIMESTAMPTZ;
