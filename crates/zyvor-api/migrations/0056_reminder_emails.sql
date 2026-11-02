-- Opt-in: email a payment reminder to the customer when its channel is email (otherwise Zyvor only logs it and
-- emits `invoice.reminder` for the business's own messaging, as before).
ALTER TABLE tenant_settings ADD COLUMN reminder_emails BOOLEAN NOT NULL DEFAULT FALSE;
