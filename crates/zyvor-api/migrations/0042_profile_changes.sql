-- A business Zyvor approved asks to change its legal details (name, type, PAN, GSTIN, address). The profile is locked
-- after approval, so the new values wait here, with the change in live_changes, until a Zyvor reviewer decides.
-- Not tenant-scoped, like live_changes. Only the last four digits of the PAN are kept.
ALTER TABLE live_changes DROP CONSTRAINT live_changes_kind_check;
ALTER TABLE live_changes ADD CONSTRAINT live_changes_kind_check CHECK (kind IN ('bank_account', 'profile'));
CREATE UNIQUE INDEX live_changes_one_profile_pending ON live_changes (organisation_id) WHERE kind = 'profile' AND status = 'pending';
CREATE TABLE profile_changes (
    change_id TEXT PRIMARY KEY REFERENCES live_changes(id),
    legal_name TEXT NOT NULL,
    business_type TEXT NOT NULL,
    pan_last4 TEXT NOT NULL,
    gstin TEXT,
    address TEXT NOT NULL,
    reason TEXT NOT NULL
);
