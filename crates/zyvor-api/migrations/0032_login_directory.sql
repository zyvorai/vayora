-- Which organisation an email address signs in to. Not tenant-scoped: it is read before anyone is
-- signed in. People always sign in to the organisation's test tenant (their identity lives there) and
-- reach live through the environment switch, so a second factor cannot be skipped by asking for live.
CREATE TABLE login_directory (
    email TEXT PRIMARY KEY,
    organisation_id TEXT NOT NULL REFERENCES organisations(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO login_directory (email, organisation_id) SELECT email, organisation_id FROM signups;
