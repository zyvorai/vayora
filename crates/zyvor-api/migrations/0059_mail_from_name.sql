-- The name payers see on an email sent for a business ("Acme Stores via Zyvor"). The address stays Zyvor's own,
-- which is the only one that can be proved with SPF and DKIM until a business brings its own domain.
ALTER TABLE mail_queue ADD COLUMN from_name TEXT;
