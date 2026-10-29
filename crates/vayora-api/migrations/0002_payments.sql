CREATE TABLE orders (
    id           TEXT PRIMARY KEY,
    amount_paise BIGINT NOT NULL CHECK (amount_paise > 0),
    currency     TEXT NOT NULL DEFAULT 'INR' CHECK (currency = 'INR'),
    status       TEXT NOT NULL CHECK (status IN ('created', 'paid')),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE payments (
    id             TEXT PRIMARY KEY,
    order_id       TEXT NOT NULL REFERENCES orders(id),
    vpa            TEXT NOT NULL,
    amount_paise   BIGINT NOT NULL CHECK (amount_paise > 0),
    refunded_paise BIGINT NOT NULL DEFAULT 0,
    status         TEXT NOT NULL CHECK (status IN ('captured', 'failed')),
    upi_ref        TEXT,
    failure_reason TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (refunded_paise BETWEEN 0 AND amount_paise)
);
-- Belt and braces: an order can never be captured twice.
CREATE UNIQUE INDEX payments_one_capture_per_order ON payments (order_id) WHERE status = 'captured';

CREATE TABLE refunds (
    id           TEXT PRIMARY KEY,
    payment_id   TEXT NOT NULL REFERENCES payments(id),
    amount_paise BIGINT NOT NULL CHECK (amount_paise > 0),
    status       TEXT NOT NULL CHECK (status IN ('processed')),
    upi_ref      TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Replay protection. The row is inserted in the same DB transaction as the
-- work, so concurrent duplicates block on the unique key, then replay.
CREATE TABLE idempotency_keys (
    scope           TEXT NOT NULL,
    key             TEXT NOT NULL,
    request_hash    TEXT NOT NULL,
    response_status INT,
    response_body   JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (scope, key)
);

-- Transactional outbox for webhooks.
CREATE TABLE webhook_events (
    id              TEXT PRIMARY KEY,
    type            TEXT NOT NULL,
    payload         JSONB NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'delivered', 'dead')),
    attempts        INT NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    delivered_at    TIMESTAMPTZ,
    last_error      TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX webhook_events_due ON webhook_events (next_attempt_at) WHERE status = 'pending';
