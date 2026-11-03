-- A queued email can carry one attachment (an invoice's PDF). Cleared with the body once the message is sent.
ALTER TABLE mail_queue ADD COLUMN attachment BYTEA, ADD COLUMN attachment_name TEXT;
