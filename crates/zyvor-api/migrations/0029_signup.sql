-- Self-serve signup: one row per email that created an organisation. Not tenant-scoped, because it is
-- read before anyone is signed in. Only a hash of the emailed token is kept.
CREATE TABLE signups (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    organisation_id TEXT NOT NULL REFERENCES organisations(id),
    token_hash TEXT NOT NULL,
    token_expires_at TIMESTAMPTZ NOT NULL,
    verified_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
