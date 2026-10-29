-- Run once as the database owner. The service connects as `zyvor_app`, which is NOT a superuser and
-- does not own the tables, so Postgres row-level security binds it. Never run the API as a superuser.
DO $$ BEGIN
  CREATE ROLE zyvor_app LOGIN PASSWORD 'change-me-in-secrets';
EXCEPTION WHEN duplicate_object THEN NULL; END $$;
GRANT CONNECT ON DATABASE zyvor TO zyvor_app;
-- After migrations have run (zyvor-api migrate):
--   GRANT USAGE ON SCHEMA public TO zyvor_app;
--   GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO zyvor_app;
--   GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO zyvor_app;
--   ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO zyvor_app;
