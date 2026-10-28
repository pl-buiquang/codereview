-- New reviews open with the file/comment/context pane showing. `sidebar_collapsed` was added with
-- DEFAULT 1 (0010) and SQLite cannot change a column default without rebuilding `review` (which
-- half the schema references), so flip it on insert instead. Existing reviews keep their state, and
-- `set_sidebar_collapsed` (an UPDATE) is unaffected.
CREATE TRIGGER review_sidebar_open AFTER INSERT ON review
WHEN NEW.sidebar_collapsed = 1
BEGIN
    UPDATE review SET sidebar_collapsed = 0 WHERE id = NEW.id;
END;
