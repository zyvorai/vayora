-- A free trial: a new subscription on the plan is first billed this many days after it starts.
ALTER TABLE plans ADD COLUMN trial_days INT NOT NULL DEFAULT 0 CHECK (trial_days BETWEEN 0 AND 365);
-- When a subscription's free trial ends (NULL: it never had one). Nothing is prorated before it.
ALTER TABLE subscriptions ADD COLUMN trial_ends_on DATE;
