-- Payment link upgrades: presentation, partial payments, bulk creation, privacy-safe analytics, SMS hook.
-- Additive only.

-- 1. How a link looks to the payer, and which methods it accepts.
ALTER TABLE payment_links
    ADD COLUMN message TEXT CHECK (message IS NULL OR char_length(message) BETWEEN 1 AND 500),
    ADD COLUMN thank_you TEXT CHECK (thank_you IS NULL OR char_length(thank_you) BETWEEN 1 AND 300),
    ADD COLUMN return_url TEXT CHECK (return_url IS NULL OR (char_length(return_url) <= 500 AND return_url ~ '^https://[^[:space:]<>"]+$')),
    -- show the business's email logo and accent colour (tenant_settings) on the payer's pages
    ADD COLUMN use_branding BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN allowed_methods TEXT[] CHECK (allowed_methods IS NULL OR (cardinality(allowed_methods) BETWEEN 1 AND 4 AND allowed_methods <@ ARRAY['upi','card','netbanking','wallet']::text[])),
    -- the caller's own reference; unique per business, which is what makes bulk creation repeatable
    ADD COLUMN external_ref TEXT CHECK (external_ref IS NULL OR char_length(external_ref) BETWEEN 1 AND 64),
    -- partial payments: the payer may pay a part (at least min_part_minor) until nothing is left
    ADD COLUMN allow_partial BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN min_part_minor BIGINT CHECK (min_part_minor > 0),
    ADD COLUMN balance_reminder_days INT CHECK (balance_reminder_days IS NULL OR balance_reminder_days BETWEEN 1 AND 60),
    ADD COLUMN balance_reminders_sent INT NOT NULL DEFAULT 0,
    ADD COLUMN balance_reminded_at TIMESTAMPTZ,
    ADD CONSTRAINT payment_links_partial_shape CHECK (
        (NOT allow_partial AND min_part_minor IS NULL AND balance_reminder_days IS NULL)
        OR (allow_partial AND min_part_minor IS NOT NULL AND NOT open_amount AND max_uses IS NULL AND splits IS NULL));
CREATE UNIQUE INDEX payment_links_external_ref ON payment_links (tenant_id, external_ref) WHERE external_ref IS NOT NULL;
-- the balance of a part-paid link is summed from the payments made through it
CREATE INDEX payment_intents_link ON payment_intents ((metadata->>'payment_link')) WHERE metadata->>'payment_link' IS NOT NULL;

-- 2. Bulk creation records: one per distinct uploaded file, so sending the same file again returns the same result.
CREATE TABLE payment_link_imports (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    file_hash TEXT NOT NULL, filename TEXT,
    row_count INT NOT NULL, created_count INT NOT NULL, existing_count INT NOT NULL,
    created_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, file_hash)
);
CREATE TABLE payment_link_import_rows (
    import_id TEXT NOT NULL REFERENCES payment_link_imports(id), tenant_id TEXT NOT NULL REFERENCES tenants(id),
    line_no INT NOT NULL, link_id TEXT NOT NULL REFERENCES payment_links(id),
    outcome TEXT NOT NULL CHECK (outcome IN ('created', 'existing')),
    PRIMARY KEY (import_id, line_no)
);

-- 3. Link analytics. Counters hold numbers only. The "seen" table holds one salted, day-scoped hash of a coarse
-- visitor token (never an address) so a repeat visit the same day is not counted twice; the worker deletes it after
-- two days.
CREATE TABLE link_events_daily (
    tenant_id TEXT NOT NULL REFERENCES tenants(id), link_id TEXT NOT NULL REFERENCES payment_links(id),
    day DATE NOT NULL, event TEXT NOT NULL CHECK (event IN ('opened', 'started')),
    n INT NOT NULL DEFAULT 0,
    PRIMARY KEY (link_id, day, event)
);
CREATE TABLE link_visitors_seen (
    tenant_id TEXT NOT NULL REFERENCES tenants(id), link_id TEXT NOT NULL REFERENCES payment_links(id),
    day DATE NOT NULL, event TEXT NOT NULL, visitor TEXT NOT NULL CHECK (char_length(visitor) = 32),
    PRIMARY KEY (link_id, day, event, visitor)
);
CREATE INDEX link_visitors_seen_day ON link_visitors_seen (day);

-- 4. Outbound SMS hook. In sandbox mode a row is written as `logged` and nothing is sent; with a hook configured the
-- worker delivers `queued` rows to it.
CREATE TABLE sms_queue (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    to_phone TEXT NOT NULL, body TEXT NOT NULL, dedupe_key TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('queued', 'sent', 'dead', 'logged')),
    attempts INT NOT NULL DEFAULT 0, last_error TEXT,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), sent_at TIMESTAMPTZ,
    UNIQUE (tenant_id, dedupe_key)
);
CREATE INDEX sms_queue_due ON sms_queue (next_attempt_at) WHERE status = 'queued';

DO $$ DECLARE t TEXT; BEGIN
  FOREACH t IN ARRAY ARRAY['payment_link_imports', 'payment_link_import_rows', 'link_events_daily', 'link_visitors_seen', 'sms_queue'] LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format($p$CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting('zyvor.tenant_id', true)) WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true))$p$, t);
  END LOOP;
END $$;
