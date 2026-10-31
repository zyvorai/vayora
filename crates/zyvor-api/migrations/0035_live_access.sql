-- Live mode opens only after a Zyvor reviewer approves the business. Reviewers are named operators with their
-- own keys; merchant keys can never act as one. Neither table is tenant-scoped: reviewers work across
-- organisations, and a request is read before a merchant has a live tenant to speak of.
CREATE TABLE platform_reviewers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    key_hash TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    disabled_at TIMESTAMPTZ
);
CREATE TABLE live_access (
    organisation_id TEXT PRIMARY KEY REFERENCES organisations(id),
    status TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'rejected')),
    requested_by TEXT NOT NULL,
    requested_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    reviewed_by TEXT REFERENCES platform_reviewers(id),
    reviewed_at TIMESTAMPTZ,
    note TEXT,
    -- how many times the merchant has asked (a rejected request can be sent again)
    round INT NOT NULL DEFAULT 1
);
CREATE INDEX live_access_pending ON live_access (requested_at) WHERE status = 'pending';
