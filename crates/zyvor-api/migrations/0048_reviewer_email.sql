-- Where to tell a reviewer that something has been given to them. Optional and the reviewer's own: they set it
-- (PUT /v1/platform/me) or whoever creates the key does. Nothing is sent to a reviewer who has none.
ALTER TABLE platform_reviewers ADD COLUMN email TEXT;
