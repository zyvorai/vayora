-- An HTML version of a queued email (payer-facing mail only); the plain-text body is always kept.
ALTER TABLE mail_queue ADD COLUMN html TEXT;
