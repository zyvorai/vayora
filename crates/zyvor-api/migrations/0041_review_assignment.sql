-- One reviewer owns a pending review, so two people do not read the same file. Nothing is enforced against a
-- review that nobody owns; a reviewer can take one over from another on purpose, and that is audited.
ALTER TABLE live_access
    ADD COLUMN assigned_to TEXT REFERENCES platform_reviewers(id),
    ADD COLUMN assigned_at TIMESTAMPTZ;
