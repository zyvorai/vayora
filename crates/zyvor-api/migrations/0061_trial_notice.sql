-- The trial end a "trial will end" event was last sent for, so one is sent per trial end date (a trial moved to a
-- later day gets a fresh one).
ALTER TABLE subscriptions ADD COLUMN trial_notified_for DATE;
