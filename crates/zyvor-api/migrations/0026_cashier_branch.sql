-- A cashier can be tied to one branch; only cashiers carry one.
ALTER TABLE console_users ADD COLUMN branch_id TEXT REFERENCES branches(id);
ALTER TABLE console_users ADD CONSTRAINT console_users_branch_cashier CHECK (branch_id IS NULL OR role = 'cashier');
