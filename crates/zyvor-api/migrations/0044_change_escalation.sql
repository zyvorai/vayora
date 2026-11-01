-- A reviewer can ask for a second, different reviewer on a pending change that no rule flagged.
ALTER TABLE live_changes
    ADD COLUMN escalated_by TEXT REFERENCES platform_reviewers(id),
    ADD COLUMN escalated_note TEXT;
