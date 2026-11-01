-- When each Zyvor reviewer is working, as weekly shifts in the review desk's local time
-- (ZYVOR_REVIEW_UTC_OFFSET_MINUTES). It informs; it never stops a reviewer from deciding.
--   weekday: 0 = Monday ... 6 = Sunday. Minutes since local midnight; end is exclusive (1440 = end of day).
--   A shift that crosses midnight is two rows.
CREATE TABLE reviewer_shifts (
    id TEXT PRIMARY KEY,
    reviewer_id TEXT NOT NULL REFERENCES platform_reviewers(id),
    weekday SMALLINT NOT NULL CHECK (weekday BETWEEN 0 AND 6),
    start_min SMALLINT NOT NULL CHECK (start_min BETWEEN 0 AND 1439),
    end_min SMALLINT NOT NULL CHECK (end_min BETWEEN 1 AND 1440),
    CHECK (start_min < end_min)
);
CREATE INDEX reviewer_shifts_reviewer ON reviewer_shifts (reviewer_id);
