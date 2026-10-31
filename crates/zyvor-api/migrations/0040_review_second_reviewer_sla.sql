-- Two reviewers for the cases that warrant it, and a clock on how long a request has waited.
--   first_approved_by: the first reviewer's approval when a second one is required; the review stays 'pending'
--                      until a different reviewer approves too.
--   escalated_by:      a reviewer asked for a second pair of eyes even though no rule did.
--   sla_alerted_at:    when the operator was emailed that this waited past its target (once).
ALTER TABLE live_access
    ADD COLUMN first_approved_by TEXT REFERENCES platform_reviewers(id),
    ADD COLUMN first_approved_at TIMESTAMPTZ,
    ADD COLUMN first_note TEXT,
    ADD COLUMN escalated_by TEXT REFERENCES platform_reviewers(id),
    ADD COLUMN escalated_note TEXT,
    ADD COLUMN sla_alerted_at TIMESTAMPTZ;
ALTER TABLE live_changes ADD COLUMN sla_alerted_at TIMESTAMPTZ;
