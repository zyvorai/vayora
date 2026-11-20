-- Copyright 2026 Zyvor AI Labs · https://zyvor.dev
-- SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

-- Events and ticketing, built on the existing payment intents, coupons, refunds and splits.
-- Seats are counted in two atomic counters (held = unpaid orders inside their hold time, sold = paid tickets) that are
-- only ever changed while the event row is locked, so concurrent purchases cannot oversell. Money is whole paise.
-- Tables are named ticket_* / event_* so they cannot be confused with the webhook "events" (outbox_events).

CREATE TABLE ticket_events (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    description TEXT NOT NULL DEFAULT '' CHECK (length(description) <= 5000),
    mode TEXT NOT NULL DEFAULT 'venue' CHECK (mode IN ('venue', 'online')),
    venue TEXT,                                   -- address or room (venue events)
    online_url TEXT,                              -- joining link (online events); only shown on paid tickets
    starts_at TIMESTAMPTZ NOT NULL,
    ends_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'on_sale', 'closed', 'cancelled')),
    currency TEXT NOT NULL DEFAULT 'INR' CHECK (currency = 'INR'),
    capacity INT NOT NULL CHECK (capacity > 0),   -- seats across all tiers
    hold_minutes INT NOT NULL DEFAULT 10 CHECK (hold_minutes BETWEEN 1 AND 180),
    seats_held INT NOT NULL DEFAULT 0 CHECK (seats_held >= 0),
    seats_sold INT NOT NULL DEFAULT 0 CHECK (seats_sold >= 0),
    -- the shares of every paid order owed to vendors, in basis points of the order total: [{"vendor_id": "...", "share_bp": 2000}]
    splits JSONB,
    qr_secret_enc BYTEA NOT NULL,                 -- signs ticket QR payloads; sealed with the master key
    created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    cancelled_at TIMESTAMPTZ,
    CHECK (seats_held + seats_sold <= capacity),
    CHECK (ends_at IS NULL OR ends_at >= starts_at)
);
CREATE INDEX ticket_events_tenant_idx ON ticket_events (tenant_id, starts_at DESC);

CREATE TABLE ticket_tiers (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    event_id TEXT NOT NULL REFERENCES ticket_events(id),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    description TEXT NOT NULL DEFAULT '',
    price_minor BIGINT NOT NULL CHECK (price_minor >= 0),
    quantity INT NOT NULL CHECK (quantity > 0),
    per_order_max INT NOT NULL DEFAULT 10 CHECK (per_order_max > 0),
    sale_starts_at TIMESTAMPTZ,
    sale_ends_at TIMESTAMPTZ,
    -- early bird: a lower price inside its own window; outside it the regular price applies
    early_bird_price_minor BIGINT CHECK (early_bird_price_minor IS NULL OR early_bird_price_minor >= 0),
    early_bird_starts_at TIMESTAMPTZ,
    early_bird_ends_at TIMESTAMPTZ,
    allowed_methods TEXT[] CHECK (allowed_methods IS NULL OR allowed_methods <@ ARRAY['upi', 'card', 'netbanking', 'wallet']),
    position INT NOT NULL DEFAULT 0,
    seats_held INT NOT NULL DEFAULT 0 CHECK (seats_held >= 0),
    seats_sold INT NOT NULL DEFAULT 0 CHECK (seats_sold >= 0),
    archived_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (seats_held + seats_sold <= quantity),
    CHECK (sale_ends_at IS NULL OR sale_starts_at IS NULL OR sale_ends_at > sale_starts_at),
    CHECK ((early_bird_price_minor IS NULL) = (early_bird_ends_at IS NULL)),
    CHECK (early_bird_ends_at IS NULL OR early_bird_starts_at IS NULL OR early_bird_ends_at > early_bird_starts_at)
);
CREATE INDEX ticket_tiers_event_idx ON ticket_tiers (event_id, position);

CREATE TABLE event_form_fields (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    event_id TEXT NOT NULL REFERENCES ticket_events(id),
    key TEXT NOT NULL CHECK (key ~ '^[a-z][a-z0-9_]{0,39}$'),
    label TEXT NOT NULL CHECK (length(label) BETWEEN 1 AND 200),
    kind TEXT NOT NULL CHECK (kind IN ('text', 'email', 'phone', 'select', 'checkbox')),
    required BOOLEAN NOT NULL DEFAULT false,
    options JSONB,                                -- select: ["a", "b"]
    min_len INT CHECK (min_len IS NULL OR min_len >= 0),
    max_len INT CHECK (max_len IS NULL OR max_len BETWEEN 1 AND 2000),
    position INT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (event_id, key)
);

CREATE TABLE event_orders (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    event_id TEXT NOT NULL REFERENCES ticket_events(id),
    status TEXT NOT NULL CHECK (status IN ('pending', 'paid', 'expired', 'cancelled', 'refunded', 'needs_refund')),
    buyer_name TEXT NOT NULL,
    buyer_email TEXT NOT NULL,
    buyer_phone TEXT,
    answers JSONB NOT NULL DEFAULT '{}'::jsonb,
    currency TEXT NOT NULL DEFAULT 'INR',
    subtotal_minor BIGINT NOT NULL CHECK (subtotal_minor >= 0),
    discount_minor BIGINT NOT NULL DEFAULT 0 CHECK (discount_minor >= 0),
    total_minor BIGINT NOT NULL CHECK (total_minor >= 0),
    coupon_id TEXT,
    quantity INT NOT NULL CHECK (quantity > 0),
    intent_id TEXT REFERENCES payment_intents(id),
    hold_expires_at TIMESTAMPTZ,
    paid_at TIMESTAMPTZ,
    cancelled_at TIMESTAMPTZ,
    cancel_reason TEXT,
    emailed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX event_orders_intent_uq ON event_orders (intent_id) WHERE intent_id IS NOT NULL;
CREATE INDEX event_orders_event_idx ON event_orders (event_id, status);
CREATE INDEX event_orders_holds_idx ON event_orders (hold_expires_at) WHERE status = 'pending';

CREATE TABLE event_order_lines (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    order_id TEXT NOT NULL REFERENCES event_orders(id),
    tier_id TEXT NOT NULL REFERENCES ticket_tiers(id),
    quantity INT NOT NULL CHECK (quantity > 0),
    unit_price_minor BIGINT NOT NULL CHECK (unit_price_minor >= 0),
    price_kind TEXT NOT NULL CHECK (price_kind IN ('regular', 'early_bird')),
    UNIQUE (order_id, tier_id)
);

CREATE TABLE tickets (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    event_id TEXT NOT NULL REFERENCES ticket_events(id),
    order_id TEXT NOT NULL REFERENCES event_orders(id),
    tier_id TEXT NOT NULL REFERENCES ticket_tiers(id),
    seq INT NOT NULL,                             -- 1..n inside the order: a second issue of the same seat cannot insert
    code TEXT NOT NULL UNIQUE,
    holder_name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'issued' CHECK (status IN ('issued', 'checked_in', 'refunded', 'void')),
    issued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    checked_in_at TIMESTAMPTZ,
    checked_in_by TEXT,
    UNIQUE (order_id, seq)
);
CREATE INDEX tickets_event_idx ON tickets (event_id, status);

-- Every scan, good or bad, so a double scan can be shown with who scanned first and when.
CREATE TABLE ticket_scans (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    event_id TEXT NOT NULL REFERENCES ticket_events(id),
    ticket_id TEXT,
    code TEXT NOT NULL,
    result TEXT NOT NULL,
    actor TEXT NOT NULL,
    scanned_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    offline BOOLEAN NOT NULL DEFAULT false
);
CREATE INDEX ticket_scans_event_idx ON ticket_scans (event_id, id DESC);

-- Bulk refunds (an event cancelled, one order cancelled, a payment that arrived after its seats were released).
CREATE TABLE event_refund_batches (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    event_id TEXT NOT NULL REFERENCES ticket_events(id),
    kind TEXT NOT NULL CHECK (kind IN ('event_cancel', 'order_cancel', 'late_payment')),
    reason TEXT,
    requested_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE event_refund_items (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    batch_id TEXT NOT NULL REFERENCES event_refund_batches(id),
    event_id TEXT NOT NULL REFERENCES ticket_events(id),
    order_id TEXT NOT NULL REFERENCES event_orders(id),
    intent_id TEXT NOT NULL REFERENCES payment_intents(id),
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0),
    state TEXT NOT NULL DEFAULT 'queued' CHECK (state IN ('queued', 'requested', 'error')),
    refund_id TEXT REFERENCES refunds(id),
    error TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (order_id)                              -- an order is refunded through one item, ever
);
CREATE INDEX event_refund_items_batch_idx ON event_refund_items (batch_id, state);
CREATE INDEX event_refund_items_queue_idx ON event_refund_items (state) WHERE state = 'queued';

DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY['ticket_events', 'ticket_tiers', 'event_form_fields', 'event_orders', 'event_order_lines', 'tickets', 'ticket_scans', 'event_refund_batches', 'event_refund_items']
  LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format('DROP POLICY IF EXISTS tenant_isolation ON %I', t);
    EXECUTE format($p$CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting('zyvor.tenant_id', true)) WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true))$p$, t);
  END LOOP;
END $$;
