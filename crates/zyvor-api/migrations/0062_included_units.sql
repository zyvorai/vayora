-- Metered plans: the first `included_units` units used in each billing period are free; only the units beyond
-- them are charged. Units inside the allowance on an invoice-less period are waived so they are not counted again.
ALTER TABLE plans ADD COLUMN included_units BIGINT NOT NULL DEFAULT 0 CHECK (included_units >= 0);
ALTER TABLE usage_records ADD COLUMN waived_on DATE;
