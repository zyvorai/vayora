-- How a business's payer-facing emails look: an accent colour for the button and top edge, and a logo shown beside the
-- business name (an https address the business hosts; the payer's mail program fetches it).
ALTER TABLE tenant_settings
    ADD COLUMN email_accent TEXT CHECK (email_accent IS NULL OR email_accent ~ '^#[0-9a-fA-F]{6}$'),
    ADD COLUMN email_logo_url TEXT CHECK (email_logo_url IS NULL OR (length(email_logo_url) <= 300 AND email_logo_url ~ '^https://[^[:space:]<>"]+$'));
