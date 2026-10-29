-- Append-only double-entry ledger. Amounts are integer paise.
CREATE TABLE ledger_transactions (
    id         UUID PRIMARY KEY,
    reference  TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE ledger_entries (
    id             BIGSERIAL PRIMARY KEY,
    transaction_id UUID NOT NULL REFERENCES ledger_transactions(id),
    account        TEXT NOT NULL,
    side           TEXT NOT NULL CHECK (side IN ('debit', 'credit')),
    amount_paise   BIGINT NOT NULL CHECK (amount_paise >= 0)
);

CREATE FUNCTION forbid_mutation() RETURNS trigger AS $$
BEGIN RAISE EXCEPTION 'ledger is append-only'; END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER ledger_entries_immutable BEFORE UPDATE OR DELETE ON ledger_entries
    FOR EACH ROW EXECUTE FUNCTION forbid_mutation();
CREATE TRIGGER ledger_transactions_immutable BEFORE UPDATE OR DELETE ON ledger_transactions
    FOR EACH ROW EXECUTE FUNCTION forbid_mutation();
