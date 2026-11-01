-- One reviewer holds a pending change, as they can hold a live-access review.
ALTER TABLE live_changes
    ADD COLUMN assigned_to TEXT REFERENCES platform_reviewers(id),
    ADD COLUMN assigned_at TIMESTAMPTZ;
