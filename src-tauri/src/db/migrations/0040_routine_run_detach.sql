-- Scratch dirs a run created (JSON array of paths). A run can outlive the app that started it, so
-- whichever process finalizes it removes them.
ALTER TABLE routine_run ADD COLUMN cleanup_json TEXT;
