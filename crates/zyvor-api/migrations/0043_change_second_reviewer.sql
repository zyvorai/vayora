-- A change to a live business (a new bank account, new legal details) can need two different Zyvor reviewers, like
-- the first approval does.
--   flagged_because:  what was unclear when the change was made (a check that did not pass, or none that ran). Whether
--                     that calls for a second reviewer is decided by the policy in force when it is decided.
--   first_approved_by: the first reviewer's approval; the change stays 'pending' (payouts stay held) until a
--                     different reviewer approves too.
ALTER TABLE live_changes
    ADD COLUMN flagged_because TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN first_approved_by TEXT REFERENCES platform_reviewers(id),
    ADD COLUMN first_approved_at TIMESTAMPTZ,
    ADD COLUMN first_note TEXT;
