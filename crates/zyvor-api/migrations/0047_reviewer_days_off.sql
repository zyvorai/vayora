-- Days nobody (or one reviewer) is working, on top of the weekly shifts: leave for one reviewer, or a desk holiday
-- (reviewer_id NULL) for everyone. Whole local days in the review desk's time; both ends inclusive. Like the shifts, it
-- informs and feeds automatic assignment; it never stops a reviewer from deciding.
CREATE TABLE reviewer_days_off (
    id TEXT PRIMARY KEY,
    reviewer_id TEXT REFERENCES platform_reviewers(id),
    first_day DATE NOT NULL,
    last_day DATE NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    created_by TEXT NOT NULL REFERENCES platform_reviewers(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (first_day <= last_day)
);
CREATE INDEX reviewer_days_off_last ON reviewer_days_off (last_day);
