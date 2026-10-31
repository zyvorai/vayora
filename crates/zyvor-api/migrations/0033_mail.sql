-- Outgoing email goes through a queue the worker drains, so a slow or failing mail server never fails a
-- request and a failed send is retried. The body (which can carry a one-time link) is blanked once sent.
CREATE TABLE mail_queue (
    id TEXT PRIMARY KEY, to_email TEXT NOT NULL, subject TEXT NOT NULL, body TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'sent', 'dead')),
    attempts INT NOT NULL DEFAULT 0, next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error TEXT, created_at TIMESTAMPTZ NOT NULL DEFAULT now(), sent_at TIMESTAMPTZ
);
CREATE INDEX mail_due ON mail_queue (next_attempt_at) WHERE status = 'pending';

-- One-time links sent by email: invitations to join an organisation and password resets. Only a hash of
-- the token is kept. Not tenant-scoped: they are used before anyone is signed in.
CREATE TABLE email_tokens (
    id TEXT PRIMARY KEY,
    purpose TEXT NOT NULL CHECK (purpose IN ('invite', 'reset')),
    email TEXT NOT NULL,
    organisation_id TEXT NOT NULL REFERENCES organisations(id),
    token_hash TEXT NOT NULL UNIQUE,
    payload JSONB NOT NULL DEFAULT '{}',
    expires_at TIMESTAMPTZ NOT NULL,
    used_at TIMESTAMPTZ, revoked_at TIMESTAMPTZ,
    created_by TEXT, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX email_tokens_org ON email_tokens (organisation_id, purpose);
