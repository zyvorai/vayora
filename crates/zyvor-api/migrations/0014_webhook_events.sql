-- Which event types the merchant's webhook endpoint wants. NULL means all of them.
ALTER TABLE tenant_settings ADD COLUMN webhook_events TEXT[];
