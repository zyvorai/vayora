-- A till-only console role: records cash and counts the drawer, nothing else.
ALTER TABLE console_users DROP CONSTRAINT console_users_role_check;
ALTER TABLE console_users ADD CONSTRAINT console_users_role_check CHECK (role IN ('admin', 'finance', 'viewer', 'cashier'));
