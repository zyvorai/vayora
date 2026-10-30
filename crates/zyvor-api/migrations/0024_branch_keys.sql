-- An API key can be pinned to one branch: it can only create and see that branch's payments, links,
-- cash entries and closings (a till or a branch manager's key).
ALTER TABLE api_keys ADD COLUMN branch_id TEXT REFERENCES branches(id);
