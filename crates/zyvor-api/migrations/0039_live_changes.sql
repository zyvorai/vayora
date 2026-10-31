-- A business Zyvor approved for live mode changes something that matters (today: it adds a bank account). The change
-- waits here for a Zyvor reviewer, and while one is waiting the business's live payouts are held. Not tenant-scoped,
-- like live_access: reviewers work across organisations. No account numbers: only what a reviewer needs to recognise it.
CREATE TABLE live_changes (
    id TEXT PRIMARY KEY,
    organisation_id TEXT NOT NULL REFERENCES organisations(id),
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    kind TEXT NOT NULL CHECK (kind IN ('bank_account')),
    ref_id TEXT NOT NULL,
    summary TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'rejected')),
    requested_by TEXT NOT NULL,
    requested_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    reviewed_by TEXT REFERENCES platform_reviewers(id),
    reviewed_at TIMESTAMPTZ,
    note TEXT
);
CREATE INDEX live_changes_pending ON live_changes (requested_at) WHERE status = 'pending';
CREATE INDEX live_changes_org ON live_changes (organisation_id, status);
